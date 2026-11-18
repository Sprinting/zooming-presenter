//! A minimal MCP (Model Context Protocol) stdio server that bridges to a
//! running Zebra. Hand-rolled subset: `initialize`, `ping`, `tools/list`,
//! `tools/call`, and notifications (ignored). Newline-delimited JSON-RPC.
use std::io::{BufRead, Write};

use serde_json::{Map, Value, json};

use crate::catalogue::{self, Kind};
use crate::client::{Client, ClientError};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
const KNOWN_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The primer every model gets (MCP `instructions`).
pub const GUIDE: &str = r##"Zebra is a zooming presentation app (classic Prezi style): one infinite canvas, content placed at any position, size and rotation; a "path" of steps that the camera flies through when presenting. A human may be watching and editing the same document live. Be a polite co-editor: you have your own camera and selection unless granted 'full' permission; never fight the human (errors 'busy', 'paused', 'presenting' mean wait and retry later).

How to work:
1. describe → read the outline (ids, kinds, positions, the path). Ids are short strings like f1 (frame), t3 (text), s2 (shape), i1 (image), k1 (ink). Use them exactly.
2. apply_commands → make changes. A call is atomic and one undo step for the human; give it a short human-readable label ("Add agenda section").
3. capture → look at your work (a PNG of a step, an element, the overview, or your own camera). Rendering never moves the human's view.
4. undo if you made a mess.

Coordinates: world units, y grows DOWN, x/y are the CENTRE of an element. Default frame is 800x600 at scale 1; text sizes come from the theme (title 72, subtitle 40, body 24 at scale 1). Make things smaller (scale/size) to nest them inside others: zooming is the storytelling. A frame carries everything inside it when moved/scaled/rotated. Use "place": {"relative_to": "f1", "side": "inside"|"below"|"right"|...} on add_text to position relative to existing elements.

Path: steps are 0-based. add_frame with "add_to_path": true appends the frame as the next step; add_step {target:{kind:"element",id}} for anything else; builds = elements that fade in one by one at that step; auto_path orders all frames top-left to bottom-right.

Rich text: add_text with "markdown": true takes **bold**, *italic*, ~~strike~~, [links](https://…), "- " bullets and "1. " numbers (indent to nest). format_text styles part of a text (by "find" or a character range); format_paragraphs sets lists, indent and alignment. describe shows rich text back as md"…".
Shapes and icons: the shapes query lists ~40 shapes (stars, clouds, speech bubbles, sticky notes, block arrows, cylinders…); add_shape takes any of them, with corner_radius and dash ("dashed"/"dotted") via update. The icons query searches ~2,000 line icons by keyword; add_icon places one (recolour with update color, swap with update icon). add_connector joins two elements with a line that follows them when they move (route straight/elbow/curved, heads none/arrow/open/dot, optional label); an end can also be a free x, y point. To write on a shape or sticky note, add_text with "on": shape id (centred, wrapped, grouped with it). group/ungroup bind elements so selecting one selects all.

Themes: classic, chalkboard, blueprint, paper, night (set_theme). Colours: "#rrggbb" or theme slots "theme:accent", "theme:title", "theme:frame", "theme:shape1".."theme:shape4" (prefer slots so themes restyle your work). Frame styles: bracket, circle, rectangle, invisible.

Call list_commands for every command with parameters and an example. Errors include a code, a hint and valid values: read them and self-correct."##;

fn tools() -> Value {
    let cmd_defs = catalogue::defs_with_root(&[Kind::Command]);
    let view_defs = catalogue::defs_with_root(&[Kind::View]);
    let query_defs = catalogue::defs_with_root(&[Kind::Query]);
    let mut v = json!([
        {
            "name": "describe",
            "title": "Describe presentation",
            "description": "Compact text outline of the whole presentation: title, theme, every frame with the elements inside it (id, kind, short text, centre x,y, size), loose elements, and the path (step order). START HERE before editing; it's much cheaper than a screenshot.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "get_state",
            "title": "Get app state",
            "description": "Session state: mode (edit/path_edit/present), current step while presenting, human selection, your selection, cameras, document revision, dirty flag, your permission (observe/edit/full), whether agents are paused or the human is mid-edit.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "apply_commands",
            "title": "Edit the presentation",
            "description": "Apply one or more document commands atomically (all or nothing) as ONE undo step labelled `label` in the human's history. Each command is an object with \"op\" (add_text, add_frame, add_shape, add_image {path}, update, move, scale, rotate, delete, duplicate, wrap_in_frame, arrange, align, add_step, update_step, auto_path, set_theme, set_title, ...). Returns the created/changed/deleted ids and the new revision. Example: {\"label\":\"Add intro\",\"commands\":[{\"op\":\"add_frame\",\"id\":\"intro\",\"x\":0,\"y\":0,\"name\":\"Intro\",\"add_to_path\":true},{\"op\":\"add_text\",\"text\":\"Hello\",\"style\":\"title\",\"place\":{\"relative_to\":\"intro\",\"side\":\"inside\"}}]}. You may choose ids yourself (\"id\" field) so later commands in the same call can refer to them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "commands": { "type": "array", "minItems": 1, "items": { "$ref": "#/$defs/Command" } },
                    "label": { "type": "string", "description": "Short undo label the human sees, e.g. \"Add timeline section\"." }
                },
                "required": ["commands"],
                "$defs": cmd_defs
            }
        },
        {
            "name": "view",
            "title": "Camera, selection, presenting",
            "description": "Session commands: zoom_to {id}, fit_all, set_camera, zoom, pan, select {ids}, present {step}, next, prev, goto_step, overview, exit_present, ... With 'edit' permission these move YOUR private camera/selection (the human's view is untouched) and present-mode commands are refused; with 'full' they drive the visible window. By default waits until the camera flight finishes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "$ref": "#/$defs/ViewCommand" },
                    "wait": { "type": "boolean", "default": true, "description": "Wait for camera animation to finish before returning." }
                },
                "required": ["command"],
                "$defs": view_defs
            }
        },
        {
            "name": "query",
            "title": "Query",
            "description": "Read-only questions: {\"q\":\"element\",\"id\"} (full detail), {\"q\":\"elements\",\"kind\"?,\"text\"?,\"within\"?} (search), {\"q\":\"path\"}, {\"q\":\"themes\"}, {\"q\":\"history\"}, {\"q\":\"lint\"} (overlaps, unreadable text, empty steps: run before you finish), {\"q\":\"hit_test\",\"x\",\"y\"}, {\"q\":\"document\"} (full JSON).",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "$ref": "#/$defs/Query" } },
                "required": ["query"],
                "$defs": query_defs
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "capture",
            "title": "Screenshot",
            "description": "Render a PNG off-screen (never moves the human's camera). target: {\"step\":n} (what the audience sees at path step n, 0-based), {\"element\":\"f1\"}, \"overview\" (everything), \"agent_view\" (your camera), \"viewport\" (the human's window). Default size 1024x640.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": catalogue::capture_target_schema(),
                    "width": { "type": "integer", "minimum": 16, "maximum": 4096 },
                    "height": { "type": "integer", "minimum": 16, "maximum": 4096 }
                }
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "undo",
            "title": "Undo / redo",
            "description": "Undo the latest step in the shared history (whoever made it — check describe/history first if the human is active), or redo with {\"redo\": true}. `steps` repeats.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "redo": { "type": "boolean", "default": false },
                    "steps": { "type": "integer", "minimum": 1, "maximum": 50, "default": 1 }
                }
            }
        },
        {
            "name": "list_commands",
            "title": "Command reference",
            "description": "Every document command, view command and query with parameters, descriptions and a one-line example, plus the value types (styles, shapes, layouts...).",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "list_documents",
            "title": "Open documents",
            "description": "The presentations open in Zebra: id, name, path, which one is in front, unsaved changes. Every other tool takes an optional `document` (id or name); without it, tools act on the front document.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "new_document",
            "title": "New document",
            "description": "Open a new, empty presentation in a window of its own (the person's window stays in front). Returns its id: pass it as `document` to every other tool to work in it.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "close_document",
            "title": "Close a document",
            "description": "Close an open document. Refused if it has unsaved changes, unless `discard` is true (needs full permission, and you must have read its latest version): then the changes are lost. Save first to keep them.",
            "inputSchema": {
                "type": "object",
                "properties": { "discard": { "type": "boolean", "default": false } },
                "required": ["document"]
            }
        },
        {
            "name": "copy_to",
            "title": "Copy into another document",
            "description": "Copy elements from `document` (default: the front one) into the open document `to`, as one undo step there. Brings along their groups, everything inside a copied frame, and the pictures and videos they show. Same position unless `at` (centre of the copies) or `dx`/`dy` is given. Doesn't touch the clipboard.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "to": { "type": "string", "description": "Id or name of the document to copy into" },
                    "at": { "type": "object", "properties": { "x": { "type": "number" }, "y": { "type": "number" } }, "required": ["x", "y"] },
                    "dx": { "type": "number" },
                    "dy": { "type": "number" }
                },
                "required": ["ids", "to"]
            }
        }
    ]);
    // Every tool that reaches a document can name one.
    for t in v.as_array_mut().into_iter().flatten() {
        if matches!(t["name"].as_str(), Some("list_commands" | "list_documents" | "new_document")) {
            continue;
        }
        if let Some(props) = t["inputSchema"]["properties"].as_object_mut() {
            props.insert("document".into(), json!({ "type": "string", "description": "Id or name of the document (see list_documents). Default: the front one." }));
        }
    }
    v
}

/// Connection to the app, made lazily so `initialize`/`tools/list` work
/// even before Zebra is running.
pub trait Connector {
    fn connect(&mut self) -> anyhow::Result<Client>;
}

impl<F: FnMut() -> anyhow::Result<Client>> Connector for F {
    fn connect(&mut self) -> anyhow::Result<Client> {
        self()
    }
}

pub struct McpServer<C: Connector> {
    connector: C,
    client: Option<Client>,
    /// The `document` of the tool call being handled.
    document: Option<String>,
}

enum ToolOut {
    Content(Vec<Value>),
    Error(String),
}

impl<C: Connector> McpServer<C> {
    pub fn new(connector: C) -> Self {
        McpServer { connector, client: None, document: None }
    }

    /// Serve until EOF on `input`.
    pub fn serve<R: BufRead, W: Write>(&mut self, input: R, mut output: W) -> anyhow::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(resp) = self.handle_line(&line) {
                output.write_all(resp.to_string().as_bytes())?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Handle one message; returns the response, if any.
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {e}")}})),
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = id?; // notifications (initialized, cancelled, ...) need no reply
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(PROTOCOL_VERSION);
                let version = if KNOWN_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "zebra", "title": "Zebra presentations", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": GUIDE,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call_tool(&name, &args) {
                    Ok(ToolOut::Content(c)) => Ok(json!({ "content": c, "isError": false })),
                    Ok(ToolOut::Error(t)) => Ok(json!({ "content": [{ "type": "text", "text": t }], "isError": true })),
                    Err(e) => Err(e),
                }
            }
            "resources/list" => Ok(json!({ "resources": [] })),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
        })
    }

    /// Call the app, reconnecting once if the connection dropped.
    fn rpc(&mut self, method: &str, mut params: Value) -> Result<Value, String> {
        if let (Some(d), Some(o)) = (&self.document, params.as_object_mut()) {
            o.insert("document".into(), json!(d));
        }
        for attempt in 0..2 {
            if self.client.is_none() {
                match self.connector.connect() {
                    Ok(c) => self.client = Some(c),
                    Err(e) => return Err(format!("cannot reach Zebra: {e}")),
                }
            }
            match self.client.as_mut().unwrap().call(method, params.clone()) {
                Ok(v) => return Ok(v),
                Err(ClientError::Rpc(e)) => return Err(format_rpc_error(&e)),
                Err(ClientError::Io(e)) => {
                    self.client = None;
                    // A call that changes something is never sent twice:
                    // the first one may have arrived (a slow batch that
                    // ran into the timeout would be applied again).
                    if !retry_is_safe(method) {
                        return Err(format!(
                            "lost connection to Zebra during `{method}`: {e}. It may or may not have been applied: read the document again (describe) before retrying."
                        ));
                    }
                    if attempt == 1 {
                        return Err(format!("lost connection to Zebra: {e}"));
                    }
                }
            }
        }
        unreachable!()
    }

    fn call_tool(&mut self, name: &str, args: &Value) -> Result<ToolOut, Value> {
        let text = |v: Value| ToolOut::Content(vec![json!({ "type": "text", "text": pretty(&v) })]);
        self.document = args.get("document").and_then(|d| d.as_str()).map(str::to_string);
        let out = match name {
            "list_documents" => self.rpc("documents", json!({})).map(text),
            "new_document" => self.rpc("new_document", json!({})).map(text),
            "close_document" => {
                if self.document.is_none() {
                    return Ok(ToolOut::Error("`document` is required: the id or name of the document to close (see list_documents)".into()));
                }
                self.rpc("close_document", json!({ "discard": args.get("discard").and_then(|d| d.as_bool()).unwrap_or(false) })).map(text)
            }
            "copy_to" => {
                let mut p = json!({});
                for k in ["ids", "to", "at", "dx", "dy"] {
                    if let Some(v) = args.get(k) {
                        p[k] = v.clone();
                    }
                }
                self.rpc("copy_to", p).map(text)
            }
            "list_commands" => Ok(ToolOut::Content(vec![json!({ "type": "text", "text": catalogue::catalogue_text() })])),
            "describe" => self.rpc("describe", json!({})).map(|v| {
                let s = v.get("outline").and_then(|o| o.as_str()).map(str::to_string).unwrap_or_else(|| pretty(&v));
                ToolOut::Content(vec![json!({ "type": "text", "text": s })])
            }),
            "get_state" => self.rpc("state", json!({})).map(text),
            "apply_commands" => {
                let Some(cmds) = args.get("commands").and_then(|c| c.as_array()).filter(|c| !c.is_empty()) else {
                    return Ok(ToolOut::Error("`commands` must be a non-empty array of {\"op\": ...} objects; call list_commands for the catalogue".into()));
                };
                let mut p = json!({ "commands": cmds });
                if let Some(l) = args.get("label") {
                    p["label"] = l.clone();
                }
                self.rpc("batch", p).map(text)
            }
            "view" => {
                let Some(cmd) = args.get("command") else {
                    return Ok(ToolOut::Error("`command` is required, e.g. {\"command\":{\"op\":\"zoom_to\",\"id\":\"f1\"}}".into()));
                };
                let r = self.rpc("view", json!({ "command": cmd }));
                match r {
                    Ok(v) => {
                        if args.get("wait").and_then(|w| w.as_bool()).unwrap_or(true) {
                            let _ = self.rpc("wait", json!({ "until": "idle", "timeout_ms": 8000 }));
                        }
                        Ok(text(v))
                    }
                    Err(e) => Err(e),
                }
            }
            "query" => {
                let q = args.get("query").cloned().unwrap_or(Value::Null);
                let q = if let Value::String(s) = q { json!({ "q": s }) } else { q };
                if q.is_null() {
                    return Ok(ToolOut::Error("`query` is required, e.g. {\"query\":{\"q\":\"path\"}}".into()));
                }
                self.rpc("query", json!({ "query": q })).map(text)
            }
            "capture" => {
                let mut p = Map::new();
                p.insert("target".into(), args.get("target").cloned().unwrap_or(json!("agent_view")));
                p.insert("width".into(), args.get("width").cloned().unwrap_or(json!(1024)));
                p.insert("height".into(), args.get("height").cloned().unwrap_or(json!(640)));
                self.rpc("capture", Value::Object(p)).map(|v| {
                    let data = v.get("data_base64").and_then(|d| d.as_str()).unwrap_or("").to_string();
                    let caption = format!(
                        "{}x{} capture of {}",
                        v["width"], v["height"], v.get("target").map(|t| t.to_string()).unwrap_or_default()
                    );
                    ToolOut::Content(vec![
                        json!({ "type": "image", "data": data, "mimeType": "image/png" }),
                        json!({ "type": "text", "text": caption }),
                    ])
                })
            }
            "undo" => {
                let redo = args.get("redo").and_then(|r| r.as_bool()).unwrap_or(false);
                let steps = args.get("steps").and_then(|s| s.as_u64()).unwrap_or(1).clamp(1, 50);
                let mut results = vec![];
                let mut err = None;
                for _ in 0..steps {
                    match self.rpc(if redo { "redo" } else { "undo" }, json!({})) {
                        Ok(v) => results.push(v),
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    }
                }
                match err {
                    Some(e) if results.is_empty() => Err(e),
                    // Some steps done, then a failure: an error, with
                    // what was done, so it isn't read as success.
                    Some(e) => Err(format!("{} of {steps} steps done ({}), then: {e}", results.len(), Value::Array(results))),
                    None => Ok(text(json!(results))),
                }
            }
            _ => {
                return Err(json!({ "code": -32602, "message": format!("unknown tool '{name}'"),
                    "data": { "valid_values": ["describe","get_state","apply_commands","view","query","capture","undo","list_commands","list_documents","new_document","close_document","copy_to"] } }));
            }
        };
        Ok(match out {
            Ok(o) => o,
            Err(e) => ToolOut::Error(e),
        })
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

fn format_rpc_error(e: &crate::protocol::RpcError) -> String {
    let mut s = format!("Error [{}]: {}", e.kind(), e.message);
    if let Some(h) = e.hint_text() {
        s.push_str(&format!("\nHint: {h}"));
    }
    if let Some(v) = e.data.get("valid_values").and_then(|v| v.as_array()) {
        let v: Vec<String> = v.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect();
        s.push_str(&format!("\nValid values: {}", v.join(", ")));
    }
    s
}

/// Calls that only read: sending one a second time after a lost
/// connection changes nothing.
fn retry_is_safe(method: &str) -> bool {
    matches!(method, "describe" | "state" | "query" | "documents" | "commands" | "schema" | "ping" | "capture" | "export" | "wait")
}

#[cfg(test)]
mod retry_tests {
    #[test]
    fn only_reads_are_sent_again() {
        for m in ["describe", "state", "query", "documents", "capture"] {
            assert!(super::retry_is_safe(m), "{m}");
        }
        for m in ["apply", "batch", "view", "undo", "redo", "begin", "commit", "abort", "new_document", "close_document", "copy_to", "open", "save"] {
            assert!(!super::retry_is_safe(m), "{m}");
        }
    }
}
