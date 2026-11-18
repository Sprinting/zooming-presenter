//! Wire-level types: JSON-RPC errors, method names, and parsing of method
//! params into typed calls (done on the connection thread, so the UI thread
//! only ever sees validated requests).
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use zebra_core::{Camera, Command, Query, ViewCommand, ZError};
use zebra_render::CaptureTarget;

/// Every method the server understands, in documentation order.
pub const METHODS: &[&str] = &[
    "hello", "ping", "methods", "commands", "schema", "subscribe", "unsubscribe", "documents", "new_document", "close_document", "state", "describe", "query",
    "apply", "batch", "copy_to", "begin", "commit", "abort", "undo", "redo", "view", "capture", "wait", "open", "save",
    "export", "disconnect", "quit",
];

/// JSON-RPC error codes used by Zebra.
pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    /// Application errors (the string `data.code` says which).
    pub const APP_ERROR: i64 = -32000;
    pub const UNAUTHENTICATED: i64 = -32001;
}

/// A JSON-RPC error. `data` always carries a string `code` plus optional
/// `hint` and `valid_values`, mirroring [`ZError`].
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Value,
}

impl RpcError {
    pub fn new(code: i64, zcode: &str, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into(), data: json!({ "code": zcode }) }
    }
    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.data["hint"] = Value::String(h.into());
        self
    }
    pub fn values(mut self, v: Vec<String>) -> Self {
        if !v.is_empty() {
            self.data["valid_values"] = json!(v);
        }
        self
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID_PARAMS, "invalid_params", message)
    }
    pub fn app(zcode: &str, message: impl Into<String>) -> Self {
        Self::new(codes::APP_ERROR, zcode, message)
    }
    /// The string error code (`busy`, `paused`, `not_found`, ...).
    pub fn kind(&self) -> &str {
        self.data.get("code").and_then(|c| c.as_str()).unwrap_or("")
    }
    pub fn hint_text(&self) -> Option<&str> {
        self.data.get("hint").and_then(|c| c.as_str())
    }
    pub fn to_json(&self) -> Value {
        json!({ "code": self.code, "message": self.message, "data": self.data })
    }
    pub fn from_json(v: &Value) -> Self {
        RpcError {
            code: v.get("code").and_then(|c| c.as_i64()).unwrap_or(codes::APP_ERROR),
            message: v.get("message").and_then(|c| c.as_str()).unwrap_or("error").to_string(),
            data: v.get("data").cloned().unwrap_or_else(|| json!({})),
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind(), self.message)?;
        if let Some(h) = self.hint_text() {
            write!(f, " (hint: {h})")?;
        }
        if let Some(v) = self.data.get("valid_values").and_then(|v| v.as_array()) {
            let v: Vec<String> = v.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect();
            write!(f, " [valid: {}]", v.join(", "))?;
        }
        Ok(())
    }
}

impl std::error::Error for RpcError {}

impl From<ZError> for RpcError {
    fn from(z: ZError) -> Self {
        let code = if z.code == "invalid_params" { codes::INVALID_PARAMS } else { codes::APP_ERROR };
        let mut e = RpcError::new(code, z.code, z.message);
        if let Some(h) = z.hint {
            e = e.hint(h);
        }
        e.values(z.valid_values)
    }
}

pub fn response_ok(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}
pub fn response_err(id: &Value, err: &RpcError) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": err.to_json() }).to_string()
}
pub fn notification(method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

// ---------------------------------------------------------------------------
// Typed calls handled by the Dispatcher (on the thread that owns the Editor).

#[derive(Debug, Clone)]
pub enum Call {
    Apply { command: Command, if_revision: Option<u64> },
    Batch { commands: Vec<Command>, label: Option<String>, if_revision: Option<u64> },
    Begin { label: String },
    Commit,
    Abort,
    View(ViewCommand),
    Query(Query),
    Describe,
    State,
    /// The open documents (front one first).
    Documents,
    /// A new, empty document in a window of its own.
    NewDocument,
    /// Close the document; with unsaved changes only when `discard`.
    CloseDocument { discard: bool },
    /// Copy elements (with their groups, frame contents and pictures) into
    /// another open document, moved by `offset` or centred on `at`.
    CopyTo { ids: Vec<String>, to: String, at: Option<(f64, f64)>, offset: (f64, f64) },
    Capture(CaptureReq),
    Undo,
    Redo,
    Open { path: PathBuf, force: bool },
    Save { path: Option<PathBuf> },
    Export { format: ExportFormat, path: Option<PathBuf>, capture: CaptureReq },
    Wait { until: WaitUntil, timeout: Duration },
    Disconnect,
    Quit,
}

impl Call {
    /// Calls that change the document and therefore yield to a human gesture.
    pub fn mutates(&self) -> bool {
        matches!(
            self,
            Call::Apply { .. } | Call::Batch { .. } | Call::Begin { .. } | Call::Undo | Call::Redo | Call::Open { .. } | Call::CopyTo { .. } | Call::CloseDocument { .. }
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct CaptureReq {
    /// None = default (agent's view for limited agents, viewport for full).
    pub target: Option<CaptureTarget>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Pdf,
    Png,
    Json,
    Zebra,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WaitUntil {
    /// No camera flight and no human gesture in progress.
    Idle,
    /// Presenting and arrived (flight finished) at this step.
    Step(usize),
    /// Document revision >= n.
    Revision(u64),
    /// Mode equals this (`edit`, `path_edit`, `present`).
    Mode(zebra_core::Mode),
}

/// Methods handled by the connection thread itself (no Editor access).
#[derive(Debug)]
pub enum Local {
    Hello { name: String, token: String },
    Ping,
    Methods,
    Commands,
    Schema,
    Subscribe { events: Option<Vec<String>> },
    Unsubscribe,
}

pub enum Parsed {
    Local(Local),
    Call(Call),
}

fn params_of<T: DeserializeOwned>(method: &str, params: &Value, example: &str) -> Result<T, RpcError> {
    serde_json::from_value(params.clone()).map_err(|e| {
        RpcError::invalid(format!("bad params for `{method}`: {e}")).hint(format!("example params: {example}"))
    })
}

fn obj(params: &Value) -> Value {
    if params.is_null() { json!({}) } else { params.clone() }
}

/// Parse a method + params. Reading image files for `add_image {path}`
/// happens here, off the UI thread.
pub fn parse(method: &str, params: &Value) -> Result<Parsed, RpcError> {
    let p = obj(params);
    let call = match method {
        "hello" => {
            #[derive(Deserialize)]
            struct H {
                #[serde(default)]
                name: Option<String>,
                #[serde(default)]
                token: Option<String>,
            }
            let h: H = params_of(method, &p, r#"{"name":"my-agent","token":"<from the discovery file>"}"#)?;
            return Ok(Parsed::Local(Local::Hello {
                name: agent_name(h.name),
                token: h.token.unwrap_or_default(),
            }));
        }
        "ping" => return Ok(Parsed::Local(Local::Ping)),
        "methods" => return Ok(Parsed::Local(Local::Methods)),
        "commands" => return Ok(Parsed::Local(Local::Commands)),
        "schema" => return Ok(Parsed::Local(Local::Schema)),
        "subscribe" => {
            #[derive(Deserialize)]
            struct S {
                #[serde(default)]
                events: Option<Vec<String>>,
            }
            let s: S = params_of(method, &p, r#"{"events":["document_changed","step_changed"]} or {}"#)?;
            if let Some(evs) = &s.events {
                for e in evs {
                    if !crate::EVENT_NAMES.contains(&e.as_str()) {
                        return Err(RpcError::invalid(format!("unknown event '{e}'"))
                            .values(crate::EVENT_NAMES.iter().map(|s| s.to_string()).collect()));
                    }
                }
            }
            return Ok(Parsed::Local(Local::Subscribe { events: s.events }));
        }
        "unsubscribe" => return Ok(Parsed::Local(Local::Unsubscribe)),
        "apply" => {
            let cmd_v = match p.get("command") {
                Some(c) => c.clone(),
                None if p.get("op").is_some() => p.clone(),
                None => {
                    return Err(RpcError::invalid("`apply` needs {\"command\": {\"op\": ...}}")
                        .hint(r#"example: {"command":{"op":"add_text","text":"Hello","x":0,"y":0,"style":"title"}}; for several commands use `batch`"#));
                }
            };
            if cmd_v.is_array() {
                return Err(RpcError::invalid("`apply` takes one command; use `batch {\"commands\": [...]}` for a list"));
            }
            let command = parse_command(cmd_v)?;
            // A label makes it its own named undo step, as for a batch.
            match p.get("label") {
                None | Some(Value::Null) => Call::Apply { command, if_revision: if_rev(&p)? },
                Some(Value::String(l)) => Call::Batch { commands: vec![command], label: Some(l.clone()), if_revision: if_rev(&p)? },
                Some(_) => return Err(RpcError::invalid("`label` must be a string")),
            }
        }
        "batch" => {
            let Some(arr) = p.get("commands").and_then(|c| c.as_array()) else {
                return Err(RpcError::invalid("`batch` needs {\"commands\": [ ... ], \"label\"?: \"...\"}")
                    .hint(r#"example: {"label":"Add intro","commands":[{"op":"add_frame","x":0,"y":0,"add_to_path":true}]}"#));
            };
            let mut commands = vec![];
            for (i, c) in arr.iter().enumerate() {
                commands.push(parse_command(c.clone()).map_err(|mut e| {
                    e.message = format!("commands[{i}]: {}", e.message);
                    e
                })?);
            }
            let label = p.get("label").and_then(|l| l.as_str()).map(str::to_string);
            Call::Batch { commands, label, if_revision: if_rev(&p)? }
        }
        "begin" => {
            let label = p.get("label").and_then(|l| l.as_str()).unwrap_or("Agent changes").to_string();
            Call::Begin { label }
        }
        "commit" => Call::Commit,
        "abort" => Call::Abort,
        "view" => {
            let v = match p.get("command") {
                Some(c) => c.clone(),
                None if p.get("op").is_some() => p.clone(),
                None => return Err(RpcError::invalid("`view` needs {\"command\": {\"op\": ...}}").hint(r#"example: {"command":{"op":"zoom_to","id":"f1"}}"#)),
            };
            Call::View(serde_json::from_value(v).map_err(|e| {
                RpcError::invalid(format!("bad view command: {e}"))
                    .hint("call `commands` for every view op with an example")
                    .values(crate::catalogue::op_names(crate::catalogue::Kind::View))
            })?)
        }
        "query" => {
            let v = match p.get("query") {
                Some(Value::String(s)) => json!({ "q": s }),
                Some(c) => c.clone(),
                None if p.get("q").is_some() => p.clone(),
                None => return Err(RpcError::invalid("`query` needs {\"query\": {\"q\": ...}}").hint(r#"example: {"query":{"q":"elements","kind":"frame"}}"#)),
            };
            Call::Query(serde_json::from_value(v).map_err(|e| {
                RpcError::invalid(format!("bad query: {e}"))
                    .hint("call `commands` for every query with an example")
                    .values(crate::catalogue::op_names(crate::catalogue::Kind::Query))
            })?)
        }
        "describe" => Call::Describe,
        "state" => Call::State,
        "documents" => Call::Documents,
        "new_document" => Call::NewDocument,
        "close_document" => Call::CloseDocument { discard: p.get("discard").and_then(|f| f.as_bool()).unwrap_or(false) },
        "copy_to" => {
            const EX: &str = r#"example: {"document":"d1","ids":["f1"],"to":"d2"} copies frame f1 and what's in it from d1 into d2 at the same place; add "at":{"x":0,"y":0} to centre the copies there"#;
            let ids: Vec<String> = match p.get("ids") {
                Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                Some(Value::String(s)) => vec![s.clone()],
                _ => vec![],
            };
            if ids.is_empty() {
                return Err(RpcError::invalid("`copy_to` needs {\"ids\": [...], \"to\": \"<document>\"}").hint(EX));
            }
            let Some(to) = p.get("to").and_then(|t| t.as_str()).map(str::to_string) else {
                return Err(RpcError::invalid("`copy_to` needs \"to\": the id or name of the document to copy into").hint(EX));
            };
            let num = |v: &Value, k: &str| v.get(k).and_then(|n| n.as_f64());
            let at = match p.get("at") {
                None | Some(Value::Null) => None,
                Some(v) => match (num(v, "x"), num(v, "y")) {
                    (Some(x), Some(y)) => Some((x, y)),
                    _ => return Err(RpcError::invalid("`at` needs {\"x\": number, \"y\": number}").hint(EX)),
                },
            };
            let offset = (p.get("dx").and_then(|n| n.as_f64()).unwrap_or(0.0), p.get("dy").and_then(|n| n.as_f64()).unwrap_or(0.0));
            Call::CopyTo { ids, to, at, offset }
        }
        "capture" => Call::Capture(parse_capture(&p)?),
        "undo" => Call::Undo,
        "redo" => Call::Redo,
        "open" => {
            let path = abs_path(p.get("path"), "open")?.ok_or_else(|| RpcError::invalid("`open` needs {\"path\": \"/abs/file.zebra\"}"))?;
            Call::Open { path, force: p.get("force").and_then(|f| f.as_bool()).unwrap_or(false) }
        }
        "save" => Call::Save { path: abs_path(p.get("path"), "save")? },
        "export" => {
            let format = match p.get("format").and_then(|f| f.as_str()) {
                Some("pdf") => ExportFormat::Pdf,
                Some("png") => ExportFormat::Png,
                Some("json") => ExportFormat::Json,
                Some("zebra") => ExportFormat::Zebra,
                other => {
                    return Err(RpcError::invalid(format!("unknown export format {other:?}"))
                        .values(vec!["pdf".into(), "png".into(), "json".into(), "zebra".into()]));
                }
            };
            Call::Export { format, path: abs_path(p.get("path"), "export")?, capture: parse_capture(&p)? }
        }
        "wait" => {
            let until = parse_until(p.get("until").unwrap_or(&json!("idle")))?;
            let ms = p.get("timeout_ms").and_then(|t| t.as_u64()).unwrap_or(10_000).min(600_000);
            Call::Wait { until, timeout: Duration::from_millis(ms) }
        }
        "disconnect" => Call::Disconnect,
        "quit" => Call::Quit,
        _ => {
            let near = crate::catalogue::nearest(method, METHODS.iter().map(|s| s.to_string()));
            let hint = if crate::catalogue::op_names(crate::catalogue::Kind::Command).iter().any(|o| o == method) {
                format!("`{method}` is a command op, not a method: call apply {{\"command\":{{\"op\":\"{method}\",...}}}}")
            } else {
                format!("did you mean `{}`?", near.first().cloned().unwrap_or_default())
            };
            return Err(RpcError::new(codes::METHOD_NOT_FOUND, "method_not_found", format!("unknown method '{method}'"))
                .hint(hint)
                .values(METHODS.iter().map(|s| s.to_string()).collect()));
        }
    };
    Ok(Parsed::Call(call))
}

fn if_rev(p: &Value) -> Result<Option<u64>, RpcError> {
    match p.get("if_revision") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| RpcError::invalid("if_revision must be a non-negative integer")),
    }
}

fn abs_path(v: Option<&Value>, method: &str) -> Result<Option<PathBuf>, RpcError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let p = PathBuf::from(expand_tilde(s));
            if !p.is_absolute() {
                return Err(RpcError::invalid(format!("`{method}`: path '{s}' must be absolute"))
                    .hint("the app's working directory is not yours; pass an absolute path (zebractl does this for you)"));
            }
            Ok(Some(p))
        }
        Some(other) => Err(RpcError::invalid(format!("`{method}`: path must be a string, got {other}"))),
    }
}

fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest).display().to_string();
        }
    }
    s.to_string()
}

const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

/// Deserialize a [`Command`], accepting `{"op":"add_image","path":"/abs.png"}`.
pub fn parse_command(mut v: Value) -> Result<Command, RpcError> {
    let Some(o) = v.as_object_mut() else {
        return Err(RpcError::invalid(format!("a command must be an object like {{\"op\":\"add_text\",...}}, got {v}")));
    };
    if o.get("op").and_then(|x| x.as_str()) == Some("add_image") && o.contains_key("path") && !o.contains_key("data_base64") {
        let path = abs_path(o.get("path"), "add_image")?.unwrap_or_default();
        let meta = std::fs::metadata(&path)
            .map_err(|e| RpcError::app("io", format!("cannot read image '{}': {e}", path.display())))?;
        if meta.len() > MAX_IMAGE_BYTES {
            return Err(RpcError::invalid(format!("image is {} bytes; the limit is {MAX_IMAGE_BYTES}", meta.len())));
        }
        let bytes = std::fs::read(&path).map_err(|e| RpcError::app("io", format!("cannot read image '{}': {e}", path.display())))?;
        o.remove("path");
        if !o.contains_key("alt") {
            if let Some(stem) = path.file_stem() {
                o.insert("alt".into(), Value::String(stem.to_string_lossy().into_owned()));
            }
        }
        o.insert("data_base64".into(), Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)));
    }
    // Media commands read their file in the app (videos are copied, not
    // loaded); the path must be absolute because the agent's working
    // directory isn't the app's.
    if matches!(o.get("op").and_then(|x| x.as_str()), Some("add_video" | "replace_image" | "set_poster" | "set_soundtrack" | "set_voiceover")) {
        let op = o.get("op").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        if let Some(p) = abs_path(o.get("path"), &op)? {
            o.insert("path".into(), Value::String(p.to_string_lossy().into_owned()));
        }
    }
    let op = o.get("op").and_then(|x| x.as_str()).map(str::to_string);
    serde_json::from_value::<Command>(v).map_err(|e| {
        let ops = crate::catalogue::op_names(crate::catalogue::Kind::Command);
        let mut err = RpcError::invalid(format!("bad command: {e}"));
        match op {
            Some(op) if ops.contains(&op) => {
                err = err.hint(format!("example: {}", crate::catalogue::example_for(&op).unwrap_or_default()));
            }
            Some(op) if ViewCommandOps::contains(&op) => {
                err = err.hint(format!("`{op}` is a view command: call `view {{\"command\":{{\"op\":\"{op}\",...}}}}`"));
            }
            _ => {
                err = err.hint("call `commands` for every op with an example").values(ops);
            }
        }
        err
    })
}

struct ViewCommandOps;
impl ViewCommandOps {
    fn contains(op: &str) -> bool {
        crate::catalogue::op_names(crate::catalogue::Kind::View).iter().any(|o| o == op)
    }
}

pub fn parse_capture(p: &Value) -> Result<CaptureReq, RpcError> {
    let dim = |k: &str| -> Result<Option<u32>, RpcError> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_u64() {
                Some(n) if (16..=8192).contains(&n) => Ok(Some(n as u32)),
                _ => Err(RpcError::invalid(format!("{k} must be an integer between 16 and 8192"))),
            },
        }
    };
    Ok(CaptureReq {
        target: match p.get("target") {
            None | Some(Value::Null) => None,
            Some(t) => Some(parse_target(t)?),
        },
        width: dim("width")?,
        height: dim("height")?,
        path: abs_path(p.get("path"), "capture")?,
    })
}

const TARGET_HELP: &str = r#"targets: "viewport" | "agent_view" | "overview" | {"step":0,"build"?:1} | {"element":"f1"} | {"camera":{"cx":0,"cy":0,"zoom":1,"rotation":0}} (or the long form {"kind":"step","index":0})"#;

/// Accepts short forms: `"overview"`, `3` (a step), `{"step":3}`,
/// `{"element":"f1"}`, `{"camera":{..}}`, or the long `{"kind":...}` form.
pub fn parse_target(t: &Value) -> Result<CaptureTarget, RpcError> {
    let bad = |m: String| RpcError::invalid(m).hint(TARGET_HELP).values(vec![
        "viewport".into(),
        "agent_view".into(),
        "overview".into(),
        "step".into(),
        "element".into(),
        "camera".into(),
    ]);
    match t {
        Value::String(s) => match s.as_str() {
            "viewport" => Ok(CaptureTarget::Viewport),
            "agent_view" | "agent" => Ok(CaptureTarget::AgentView),
            "overview" => Ok(CaptureTarget::Overview),
            other => Err(bad(format!("unknown capture target '{other}'"))),
        },
        Value::Number(n) => Ok(CaptureTarget::Step { index: n.as_u64().unwrap_or(0) as usize, build: None }),
        Value::Object(o) => {
            if o.contains_key("kind") {
                return serde_json::from_value(t.clone()).map_err(|e| bad(format!("bad capture target: {e}")));
            }
            if let Some(s) = o.get("step") {
                let index = s.as_u64().ok_or_else(|| bad("step must be an integer".into()))? as usize;
                let build = o.get("build").and_then(|b| b.as_u64()).map(|b| b as usize);
                return Ok(CaptureTarget::Step { index, build });
            }
            if let Some(id) = o.get("element").or(o.get("id")).and_then(|e| e.as_str()) {
                return Ok(CaptureTarget::Element { id: id.to_string() });
            }
            if let Some(c) = o.get("camera") {
                let camera: Camera = serde_json::from_value(c.clone()).map_err(|e| bad(format!("bad camera: {e}")))?;
                return Ok(CaptureTarget::Camera { camera });
            }
            Err(bad(format!("unrecognised capture target {t}")))
        }
        _ => Err(bad(format!("unrecognised capture target {t}"))),
    }
}

pub fn parse_until(u: &Value) -> Result<WaitUntil, RpcError> {
    let bad = || {
        RpcError::invalid(format!("bad `until`: {u}"))
            .hint(r#"use "idle", {"step":2}, {"revision":17} or {"mode":"present"}"#)
            .values(vec!["idle".into(), "step".into(), "revision".into(), "mode".into()])
    };
    match u {
        Value::String(s) if s == "idle" => Ok(WaitUntil::Idle),
        Value::Object(o) => {
            if o.get("idle").is_some() {
                return Ok(WaitUntil::Idle);
            }
            if let Some(n) = o.get("step").and_then(|n| n.as_u64()) {
                return Ok(WaitUntil::Step(n as usize));
            }
            if let Some(n) = o.get("revision").and_then(|n| n.as_u64()) {
                return Ok(WaitUntil::Revision(n));
            }
            if let Some(m) = o.get("mode") {
                return serde_json::from_value(m.clone()).map(WaitUntil::Mode).map_err(|_| bad());
            }
            Err(bad())
        }
        _ => Err(bad()),
    }
}

/// The name an agent goes by in the status bar and the history: its own,
/// trimmed to one short line. "You" is how the person's own steps are
/// shown, so an agent can't take it.
pub(crate) fn agent_name(name: Option<String>) -> String {
    let n: String = name.unwrap_or_default().chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(40).collect();
    let n = n.trim().to_string();
    if n.is_empty() {
        "agent".into()
    } else if n.eq_ignore_ascii_case("you") || n.eq_ignore_ascii_case("human") {
        format!("agent \"{n}\"")
    } else {
        n
    }
}
