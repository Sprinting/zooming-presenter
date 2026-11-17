//! A small blocking client for the Zebra socket (used by `zebractl`, the MCP
//! bridge and tests).
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::discovery;
use crate::protocol::RpcError;

#[derive(Debug)]
pub enum ClientError {
    /// Connection problem (the app went away, ...).
    Io(std::io::Error),
    /// The server answered with an error.
    Rpc(RpcError),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "connection error: {e}"),
            ClientError::Rpc(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for ClientError {}
impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        ClientError::Io(e)
    }
}

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
    pending: Vec<u8>,
    /// Event notifications received while waiting for responses.
    pub events: VecDeque<Value>,
    pub socket: PathBuf,
}

impl Client {
    pub fn connect(socket: &Path) -> std::io::Result<Client> {
        let s = UnixStream::connect(socket)
            .map_err(|e| std::io::Error::new(e.kind(), format!("cannot connect to {}: {e}", socket.display())))?;
        Ok(Client {
            reader: BufReader::new(s.try_clone()?),
            writer: s,
            next_id: 1,
            pending: vec![],
            events: VecDeque::new(),
            socket: socket.to_path_buf(),
        })
    }

    /// Connect and authenticate in one go.
    pub fn connect_hello(socket: &Path, name: &str, token: &str) -> Result<Client, ClientError> {
        let mut c = Client::connect(socket)?;
        c.call("hello", json!({ "name": name, "token": token }))?;
        Ok(c)
    }

    /// Find a running instance via discovery (newest, or by pid / socket),
    /// connect and say hello. `token` overrides the discovery file's.
    pub fn discover(pid: Option<u32>, socket: Option<&Path>, token: Option<&str>, name: &str) -> anyhow::Result<Client> {
        let (sock, tok) = match (socket, token) {
            (Some(s), Some(t)) => (s.to_path_buf(), t.to_string()),
            _ => {
                let inst = discovery::find(pid, socket)?;
                (inst.socket, token.map(str::to_string).unwrap_or(inst.token))
            }
        };
        Client::connect_hello(&sock, name, &tok).map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn read_message(&mut self) -> Result<Option<Value>, ClientError> {
        loop {
            match self.reader.read_until(b'\n', &mut self.pending) {
                Ok(0) => {
                    return Err(ClientError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Zebra closed the connection")));
                }
                Ok(_) => {}
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) => return Err(e.into()),
            }
            if self.pending.last() != Some(&b'\n') {
                continue;
            }
            let line = std::mem::take(&mut self.pending);
            let text = String::from_utf8_lossy(&line);
            if text.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(text.trim())
                .map_err(|e| ClientError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
            return Ok(Some(v));
        }
    }

    /// Send a raw JSON-RPC message (adds `jsonrpc` if missing) and return the
    /// whole response object (or `null` for a notification).
    pub fn raw(&mut self, mut msg: Value) -> Result<Value, ClientError> {
        if msg.get("jsonrpc").is_none() {
            msg["jsonrpc"] = json!("2.0");
        }
        let id = msg.get("id").cloned();
        self.send_line(&msg.to_string())?;
        let Some(id) = id else { return Ok(Value::Null) };
        self.wait_response(&id)
    }

    fn send_line(&mut self, s: &str) -> std::io::Result<()> {
        self.writer.write_all(s.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }

    fn wait_response(&mut self, id: &Value) -> Result<Value, ClientError> {
        self.reader.get_ref().set_read_timeout(None)?;
        loop {
            let Some(m) = self.read_message()? else { continue };
            if m.get("method").is_some() && m.get("id").is_none() {
                self.events.push_back(m.get("params").cloned().unwrap_or(Value::Null));
                if self.events.len() > 10_000 {
                    self.events.pop_front();
                }
                continue;
            }
            if m.get("id") == Some(id) || m.get("id") == Some(&Value::Null) {
                return Ok(m);
            }
        }
    }

    /// Call a method; returns `result` or the server's error.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let resp = self.raw(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        if let Some(e) = resp.get("error") {
            return Err(ClientError::Rpc(RpcError::from_json(e)));
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Next event notification's params (`{"event": "...", ...}`), waiting up
    /// to `timeout` (None = forever). Requires `subscribe` first.
    pub fn next_event(&mut self, timeout: Option<Duration>) -> Result<Option<Value>, ClientError> {
        if let Some(e) = self.events.pop_front() {
            return Ok(Some(e));
        }
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        loop {
            let left = match deadline {
                Some(d) => {
                    let now = std::time::Instant::now();
                    if now >= d {
                        return Ok(None);
                    }
                    Some((d - now).max(Duration::from_millis(1)))
                }
                None => None,
            };
            self.reader.get_ref().set_read_timeout(left)?;
            match self.read_message()? {
                Some(m) if m.get("method").and_then(|x| x.as_str()) == Some("event") => {
                    self.reader.get_ref().set_read_timeout(None)?;
                    return Ok(Some(m.get("params").cloned().unwrap_or(Value::Null)));
                }
                Some(_) => continue,
                None => {
                    if deadline.is_none() {
                        continue;
                    }
                }
            }
        }
    }
}
