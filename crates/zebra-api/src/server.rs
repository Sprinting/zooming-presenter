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
    Call { client: u64, id: Option<Value>, origin: Origin, call: Call, document: Option<String> },
    Disconnected { client: u64 },
}

pub(crate) struct Conn {
    pub name: Option<String>,
    pub out: Sender<String>,
    /// None = not subscribed; Some(None) = all events; Some(Some(set)) = filter.
    pub subs: Option<Option<HashSet<String>>>,
    stream: UnixStream,
    /// Bytes queued for this client and not yet written to its socket.
    backlog: Arc<std::sync::atomic::AtomicUsize>,
}

/// Replies and events queued for a client that has stopped reading are
/// kept up to this many bytes (plus the line that crosses it); then it is
/// disconnected, instead of the queue growing for as long as Zebra runs.
/// Replies count too: a client asking for 10 MB captures without reading
/// them grew it without bound (hostile-input audit L2).
pub(crate) const MAX_BACKLOG: usize = 16 << 20;

/// Queue `line` for a client, or disconnect it if it has stopped reading.
/// Closing the socket ends its reader, which reports the disconnect.
fn queue(out: &Sender<String>, backlog: &std::sync::atomic::AtomicUsize, stream: &UnixStream, line: String) {
    if backlog.load(Ordering::SeqCst) > MAX_BACKLOG {
        let _ = stream.shutdown(std::net::Shutdown::Both);
        return;
    }
    backlog.fetch_add(line.len(), Ordering::SeqCst);
    let _ = out.send(line);
}

impl Conn {
    fn push(&self, line: String) {
        queue(&self.out, &self.backlog, &self.stream, line);
    }
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
            c.push(line);
        }
    }
    pub fn broadcast_event(&self, name: &str, params: &Value) {
        let line = protocol::notification("event", params.clone());
        for c in self.clients.lock().unwrap().values() {
            let wants = match &c.subs {
                Some(None) => true,
                Some(Some(set)) => set.contains(name),
                None => false,
            };
            if !wants {
                continue;
            }
            c.push(line.clone());
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

pub(crate) fn start(opts: ApiOptions, waker: Waker) -> anyhow::Result<ServerInner> {
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

/// How long a new connection has to authenticate with `hello`.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

fn connection(id: u64, stream: UnixStream, shared: Arc<Shared>, tx: Sender<Inbound>) {
    let (out_tx, out_rx) = channel::<String>();
    let Ok(write_half) = stream.try_clone() else { return };
    let Ok(ctl) = stream.try_clone() else { return };
    let Ok(ctl2) = stream.try_clone() else { return };
    let backlog = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    shared.clients.lock().unwrap().insert(id, Conn { name: None, out: out_tx.clone(), subs: None, stream: ctl, backlog: backlog.clone() });
    let written = backlog.clone();
    let writer = std::thread::spawn(move || {
        let mut w = std::io::BufWriter::new(write_half);
        while let Ok(line) = out_rx.recv() {
            let r = w.write_all(line.as_bytes()).and_then(|_| w.write_all(b"\n"));
            let _ = written.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some(n.saturating_sub(line.len())));
            if r.is_err() {
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
    // A client that hasn't said `hello` (with the token) within this long
    // is dropped, so silent connections can't pile up (test drive O4).
    let deadline = std::time::Instant::now() + HELLO_TIMEOUT;
    loop {
        buf.clear();
        if origin.is_none() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() || reader.get_ref().set_read_timeout(Some(left)).is_err() {
                break;
            }
        }
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
        let reply = |s: String| queue(&out_tx, &backlog, &ctl2, s);
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
        // A field beside `params` (say `command`) would otherwise be lost.
        if let Some(Err(e)) = msg.as_object().map(|o| crate::strict::check_keys("the request", o, &["jsonrpc", "id", "method", "params"])) {
            if let Some(i) = &rid {
                reply(protocol::response_err(i, &e.hint(r#"a method's params go inside "params", e.g. {"jsonrpc":"2.0","id":1,"method":"apply","params":{"command":{...}}}"#)));
            }
            continue;
        }
        let mut params = msg.get("params").cloned().unwrap_or(Value::Null);
        // Any call can name the document it is for; the rest of the params
        // are the method's own.
        let document = match params.as_object_mut().and_then(|o| o.remove("document")) {
            None | Some(Value::Null) => None,
            Some(Value::String(d)) => Some(d),
            Some(_) => {
                if let Some(i) = &rid {
                    reply(protocol::response_err(i, &RpcError::invalid("`document` must be a string: a document's id or name").hint("call `documents` to list them")));
                }
                continue;
            }
        };
        let respond = |r: Result<Value, RpcError>| {
            if let Some(i) = &rid {
                reply(match r {
                    Ok(v) => protocol::response_ok(i, v),
                    Err(e) => protocol::response_err(i, &e),
                });
            }
        };
        // Before anything is read from the request: parsing `add_image
        // {path}` opens that file, and its error says whether it exists.
        if origin.is_none() && !matches!(method, "hello" | "ping") {
            respond(Err(RpcError::new(
                codes::UNAUTHENTICATED,
                "unauthenticated",
                format!("call `hello {{\"name\":..., \"token\":...}}` before `{method}`"),
            )
            .hint("the token is in the instance's discovery file; zebractl does this for you")));
            continue;
        }
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
                // Known now: it may sit quietly as long as it likes (waiting
                // for events).
                let _ = reader.get_ref().set_read_timeout(None);
                (shared.waker)(); // status bar: a client connected
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
                if tx.send(Inbound::Call { client: id, id: rid.clone(), origin: o, call, document }).is_err() {
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
