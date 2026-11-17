//! The socket side: accept loop and one reader + one writer thread per
//! connection. Connection threads authenticate, answer editor-free methods
//! themselves, parse everything else into a typed [`Call`] and queue it for
//! the [`crate::Dispatcher`], waking the UI thread.
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use zebra_core::Origin;

use crate::protocol::{self, Call, Local, Parsed, RpcError, codes};
use crate::{ApiOptions, ClientInfo, Permission, Waker, discovery};

pub(crate) enum Inbound {
    Call { client: u64, id: Option<Value>, origin: Origin, call: Call },
    Disconnected { client: u64 },
}

pub(crate) struct Conn {
    pub name: Option<String>,
    pub out: Sender<String>,
    /// None = not subscribed; Some(None) = all events; Some(Some(set)) = filter.
    pub subs: Option<Option<HashSet<String>>>,
    stream: UnixStream,
}

pub(crate) struct Shared {
    pub clients: Mutex<HashMap<u64, Conn>>,
    pub shutdown: AtomicBool,
    /// The dispatcher has timed work (waits, deferred calls): keep waking it.
    pub poll: AtomicBool,
    pub waker: Waker,
    next_client: AtomicU64,
    token: String,
}

impl Shared {
    pub fn send(&self, client: u64, line: String) {
        if let Some(c) = self.clients.lock().unwrap().get(&client) {
            let _ = c.out.send(line);
        }
    }
    pub fn broadcast_event(&self, name: &str, params: &Value) {
        let line = protocol::notification("event", params.clone());
        for c in self.clients.lock().unwrap().values() {
            match &c.subs {
                Some(None) => {
                    let _ = c.out.send(line.clone());
                }
                Some(Some(set)) if set.contains(name) => {
                    let _ = c.out.send(line.clone());
                }
                _ => {}
            }
        }
    }
}

pub struct ServerInner {
    pub(crate) socket: PathBuf,
    pub(crate) token: String,
    pub(crate) permission: Permission,
    pub(crate) discovery_file: Option<PathBuf>,
    pub(crate) rx: Mutex<Receiver<Inbound>>,
    pub(crate) shared: Arc<Shared>,
}

static SERVERS_IN_PROCESS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn start(opts: ApiOptions, waker: Waker, headless: bool) -> anyhow::Result<ServerInner> {
    let pid = std::process::id();
    let nth = SERVERS_IN_PROCESS.fetch_add(1, Ordering::SeqCst);
    let stem = if nth == 0 { format!("{pid}") } else { format!("{pid}-{nth}") };
    let socket = match &opts.socket {
        Some(s) => s.clone(),
        None => discovery::default_socket_path(&stem)?,
    };
    if socket.as_os_str().len() > 103 {
        anyhow::bail!("socket path is {} bytes; macOS allows at most 103: {}", socket.as_os_str().len(), socket.display());
    }
    if socket.exists() {
        if UnixStream::connect(&socket).is_ok() {
            anyhow::bail!("another process is already listening on {}", socket.display());
        }
        std::fs::remove_file(&socket)?;
    }
    if let Some(parent) = socket.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let listener = UnixListener::bind(&socket).map_err(|e| anyhow::anyhow!("cannot bind {}: {e}", socket.display()))?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;

    let token = format!("{:032x}", rand::random::<u128>());
    let (tx, rx) = channel();
    let shared = Arc::new(Shared {
        clients: Mutex::new(HashMap::new()),
        shutdown: AtomicBool::new(false),
        poll: AtomicBool::new(false),
        waker,
        next_client: AtomicU64::new(1),
        token: token.clone(),
    });

    let inst = discovery::Instance {
        pid,
        socket: socket.clone(),
        token: token.clone(),
        version: env!("CARGO_PKG_VERSION").into(),
        api_version: zebra_core::API_VERSION,
        started_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        headless,
        file: PathBuf::new(),
    };
    let discovery_file = match discovery::write(&inst, &format!("{stem}.json")) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("zebra-api: could not write discovery file: {e}");
            None
        }
    };

    // Accept loop.
    {
        let shared = shared.clone();
        std::thread::Builder::new().name("zebra-api-accept".into()).spawn(move || {
            for stream in listener.incoming() {
                if shared.shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let id = shared.next_client.fetch_add(1, Ordering::SeqCst);
                let shared2 = shared.clone();
                let tx2 = tx.clone();
                let _ = std::thread::Builder::new()
                    .name(format!("zebra-api-conn-{id}"))
                    .spawn(move || connection(id, stream, shared2, tx2));
            }
        })?;
    }
    // Ticker: while the dispatcher has timed work, wake the UI thread.
    {
        let shared = shared.clone();
        std::thread::Builder::new().name("zebra-api-ticker".into()).spawn(move || {
            while !shared.shutdown.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(30));
                if shared.poll.load(Ordering::SeqCst) {
                    (shared.waker)();
                }
            }
        })?;
    }

    Ok(ServerInner { socket, token, permission: opts.permission, discovery_file, rx: Mutex::new(rx), shared })
}

impl ServerInner {
    pub(crate) fn clients(&self) -> Vec<ClientInfo> {
        let mut v: Vec<ClientInfo> = self
            .shared
            .clients
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, c)| c.name.clone().map(|name| ClientInfo { id: *id, name }))
            .collect();
        v.sort_by_key(|c| c.id);
        v
    }
}

impl Drop for ServerInner {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        // Unblock accept().
        let _ = UnixStream::connect(&self.socket);
        for c in self.shared.clients.lock().unwrap().values() {
            let _ = c.stream.shutdown(std::net::Shutdown::Both);
        }
        let _ = std::fs::remove_file(&self.socket);
        if let Some(f) = &self.discovery_file {
            let _ = std::fs::remove_file(f);
        }
    }
}

const MAX_LINE: usize = 128 * 1024 * 1024;

fn connection(id: u64, stream: UnixStream, shared: Arc<Shared>, tx: Sender<Inbound>) {
    let (out_tx, out_rx) = channel::<String>();
    let Ok(write_half) = stream.try_clone() else { return };
    let Ok(ctl) = stream.try_clone() else { return };
    shared.clients.lock().unwrap().insert(id, Conn { name: None, out: out_tx.clone(), subs: None, stream: ctl });
    let writer = std::thread::spawn(move || {
        let mut w = std::io::BufWriter::new(write_half);
        while let Ok(line) = out_rx.recv() {
            if w.write_all(line.as_bytes()).and_then(|_| w.write_all(b"\n")).is_err() {
                break;
            }
            // Flush when nothing else is queued.
            if w.flush().is_err() {
                break;
            }
        }
    });

    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    let mut origin: Option<Origin> = None;
    loop {
        buf.clear();
        match (&mut reader).take(MAX_LINE as u64).read_line(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if shared.shutdown.load(Ordering::SeqCst) {
            break;
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        let reply = |s: String| {
            let _ = out_tx.send(s);
        };
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                reply(protocol::response_err(
                    &Value::Null,
                    &RpcError::new(codes::PARSE_ERROR, "parse_error", format!("invalid JSON: {e}"))
                        .hint(r#"send one JSON-RPC object per line, e.g. {"jsonrpc":"2.0","id":1,"method":"ping"}"#),
                ));
                continue;
            }
        };
        if msg.is_array() {
            reply(protocol::response_err(
                &Value::Null,
                &RpcError::new(codes::INVALID_REQUEST, "invalid_request", "JSON-RPC batch arrays are not supported")
                    .hint("send requests one per line; to apply several commands atomically use the `batch` method"),
            ));
            continue;
        }
        let rid = msg.get("id").cloned();
        let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
            reply(protocol::response_err(
                &rid.unwrap_or(Value::Null),
                &RpcError::new(codes::INVALID_REQUEST, "invalid_request", "missing \"method\"")
                    .values(protocol::METHODS.iter().map(|s| s.to_string()).collect()),
            ));
            continue;
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let respond = |r: Result<Value, RpcError>| {
            if let Some(i) = &rid {
                reply(match r {
                    Ok(v) => protocol::response_ok(i, v),
                    Err(e) => protocol::response_err(i, &e),
                });
            }
        };
        let parsed = match protocol::parse(method, &params) {
            Ok(p) => p,
            Err(e) => {
                respond(Err(e));
                continue;
            }
        };
        match parsed {
            Parsed::Local(Local::Ping) => respond(Ok(json!("pong"))),
            Parsed::Local(Local::Hello { name, token }) => {
                if token != shared.token {
                    respond(Err(RpcError::new(codes::UNAUTHENTICATED, "unauthenticated", "wrong or missing token").hint(format!(
                        "read \"token\" from the discovery file in {} (zebractl does this for you)",
                        discovery::instances_dir().display()
                    ))));
                    continue;
                }
                if let Some(c) = shared.clients.lock().unwrap().get_mut(&id) {
                    c.name = Some(name.clone());
                }
                origin = Some(Origin::Agent { name: name.clone() });
                (shared.waker)(); // status pill: a client connected
                respond(Ok(json!({
                    "server": "zebra",
                    "version": env!("CARGO_PKG_VERSION"),
                    "api_version": zebra_core::API_VERSION,
                    "client_id": id,
                    "name": name,
                    "methods": protocol::METHODS,
                    "events": crate::EVENT_NAMES,
                    "next": "call `describe` for an outline, `commands` for the command catalogue",
                })));
            }
            _ if origin.is_none() => {
                respond(Err(RpcError::new(
                    codes::UNAUTHENTICATED,
                    "unauthenticated",
                    format!("call `hello {{\"name\":..., \"token\":...}}` before `{method}`"),
                )
                .hint("the token is in the instance's discovery file; zebractl does this for you")));
            }
            Parsed::Local(Local::Methods) => respond(Ok(json!(protocol::METHODS))),
            Parsed::Local(Local::Commands) => respond(Ok(crate::catalogue::catalogue_json())),
            Parsed::Local(Local::Schema) => respond(Ok(crate::catalogue::schema_json())),
            Parsed::Local(Local::Subscribe { events }) => {
                let list = events.clone();
                if let Some(c) = shared.clients.lock().unwrap().get_mut(&id) {
                    c.subs = Some(events.map(|e| e.into_iter().collect()));
                }
                respond(Ok(json!({ "subscribed": list.map(Value::from).unwrap_or(json!(crate::EVENT_NAMES)) })));
            }
            Parsed::Local(Local::Unsubscribe) => {
                if let Some(c) = shared.clients.lock().unwrap().get_mut(&id) {
                    c.subs = None;
                }
                respond(Ok(json!({ "subscribed": [] })));
            }
            Parsed::Call(call) => {
                let o = origin.clone().unwrap();
                if tx.send(Inbound::Call { client: id, id: rid.clone(), origin: o, call }).is_err() {
                    respond(Err(RpcError::app("shutting_down", "the app is shutting down")));
                    break;
                }
                (shared.waker)();
            }
        }
    }
    shared.clients.lock().unwrap().remove(&id);
    let _ = tx.send(Inbound::Disconnected { client: id });
    (shared.waker)();
    drop(out_tx);
    let _ = writer.join();
}
