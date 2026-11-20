//! One persistent connection per agent.
//!
//! Each `zebractl` call is its own short process. Connecting straight to
//! Zebra would open and close a socket per command, so Zebra could never
//! tell whether an agent is still there. Instead the first call starts a
//! small background relay for that agent name (`zebractl __relay`): it
//! says `hello` to Zebra once and keeps that one socket open; later calls
//! talk to the relay, which forwards their requests over the same socket.
//! Zebra therefore sees exactly one connection per agent, open while the
//! agent is working, so its status bar can say who is connected.
//!
//! The relay exits when Zebra closes the connection (AI control switched
//! off, `zebractl disconnect`, Zebra quit) or after `IDLE` without calls.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::client::Client;

/// A relay with no calls for this long exits (and its connection closes).
pub const IDLE: Duration = Duration::from_secs(10 * 60);

/// The relay socket for `name` on the Zebra instance at `zebra_socket`:
/// next to it, named from the instance and a hash of the agent name.
pub fn socket_for(zebra_socket: &Path, name: &str) -> PathBuf {
    // FNV-1a: stable across runs and builds (unlike DefaultHasher).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let stem = zebra_socket.file_stem().and_then(|s| s.to_str()).unwrap_or("zebra");
    zebra_socket.with_file_name(format!("{stem}.agent{:08x}.sock", h as u32))
}

/// A zebractl process that sends nothing (or reads nothing) for this long
/// is dropped; the others go on.
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Most zebractl processes served at once; more are turned away (and fall
/// back to their own connection).
pub const MAX_CLIENTS: usize = 16;

/// Run the relay in this process until Zebra goes away or it idles out.
pub fn run(zebra_socket: &Path, token: &str, name: &str, relay_socket: &Path) -> anyhow::Result<()> {
    let mut upstream = Client::connect_hello(zebra_socket, name, token).map_err(|e| anyhow::anyhow!("{e}"))?;
    // Each zebractl call has its own timeout; the relay itself waits.
    upstream.timeout = None;
    if relay_socket.exists() {
        if UnixStream::connect(relay_socket).is_ok() {
            return Ok(()); // another relay for this agent won the race
        }
        let _ = std::fs::remove_file(relay_socket);
    }
    let listener = UnixListener::bind(relay_socket)?;
    listener.set_nonblocking(true)?;
    use std::os::fd::AsRawFd;
    let lfd = listener.as_raw_fd();
    let _cleanup = RemoveOnDrop(relay_socket.to_path_buf());
    // Each zebractl process is served on its own thread, so one that
    // connects and says nothing stalls only itself; their calls share the
    // one upstream connection in turn.
    let upstream = Arc::new(Mutex::new(upstream));
    let active = Arc::new(AtomicUsize::new(0));
    let gone = Arc::new(AtomicBool::new(false));
    let (token, name) = (Arc::<str>::from(token), Arc::<str>::from(name));
    let mut last = Instant::now();
    loop {
        if gone.load(Ordering::SeqCst) {
            return Ok(()); // Zebra closed our connection
        }
        match listener.accept() {
            Ok((stream, _)) => {
                last = Instant::now();
                // On macOS an accepted socket inherits the listener's
                // O_NONBLOCK; a read could then fail before the request
                // arrives. Blocking, with timeouts, instead.
                if stream.set_nonblocking(false).is_err()
                    || stream.set_read_timeout(Some(CLIENT_TIMEOUT)).is_err()
                    || stream.set_write_timeout(Some(CLIENT_TIMEOUT)).is_err()
                    || active.load(Ordering::SeqCst) >= MAX_CLIENTS
                {
                    continue; // dropped: that zebractl connects directly
                }
                active.fetch_add(1, Ordering::SeqCst);
                let (up, act, gone2, tok, nm) = (upstream.clone(), active.clone(), gone.clone(), token.clone(), name.clone());
                let spawned = std::thread::Builder::new().name("zebractl-relay-client".into()).spawn(move || {
                    if serve(stream, &up, &tok, &nm).is_err() {
                        gone2.store(true, Ordering::SeqCst);
                    }
                    act.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    active.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let busy = active.load(Ordering::SeqCst) > 0;
                if busy {
                    last = Instant::now();
                }
                // A call in progress holds the connection: it isn't closed.
                let closed = upstream.try_lock().map(|u| upstream_closed(&u)).unwrap_or(false);
                if (!busy && last.elapsed() > IDLE) || closed {
                    return Ok(());
                }
                // Sleep until a zebractl call connects; wake once a second
                // to notice Zebra closing the connection or the idle limit.
                let mut p = libc::pollfd { fd: lfd, events: libc::POLLIN, revents: 0 };
                // SAFETY: one valid pollfd for the listening socket.
                unsafe { libc::poll(&mut p, 1, 1000) };
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// True once Zebra has closed the relay's connection.
fn upstream_closed(c: &Client) -> bool {
    c.peer_closed()
}

struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Answer one zebractl process: its `hello` locally (the relay already
/// said hello), everything else forwarded. Err = Zebra is gone.
fn serve(stream: UnixStream, upstream: &Mutex<Client>, token: &str, name: &str) -> Result<(), ()> {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return Ok(()),
    });
    let mut out = stream;
    let mut line = String::new();
    // Nothing is forwarded until this process has shown the token, as on
    // Zebra's own socket.
    let mut authed = false;
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            // The zebractl process finished, or said nothing for
            // CLIENT_TIMEOUT.
            Ok(0) | Err(_) => return Ok(()),
            Ok(_) => {}
        }
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
        // Only an object is a message; a stray array or number is skipped.
        if !msg.is_object() {
            continue;
        }
        let id = msg.get("id").cloned();
        let unauthenticated = |id: &Option<Value>| json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32001, "message": "wrong or missing token", "data": {"code": "unauthenticated"}}});
        let reply = if msg.get("method").and_then(Value::as_str) == Some("hello") {
            let p = msg.get("params").cloned().unwrap_or(Value::Null);
            if p.get("token").and_then(Value::as_str) != Some(token) {
                unauthenticated(&id)
            } else {
                authed = true;
                json!({"jsonrpc": "2.0", "id": id, "result": {"server": "zebra", "name": name, "relay": true}})
            }
        } else if !authed {
            unauthenticated(&id)
        } else {
            let r = upstream.lock().unwrap_or_else(|e| e.into_inner()).raw(msg);
            match r {
                Ok(v) => v,
                Err(crate::client::ClientError::Rpc(e)) => json!({"jsonrpc": "2.0", "id": id, "error": e.to_json()}),
                Err(_) => return Err(()),
            }
        };
        if id.is_some() && writeln!(out, "{reply}").is_err() {
            return Ok(());
        }
    }
}
