//! The command catalogue: every Command / ViewCommand / Query variant with
//! its JSON schema (from schemars) and a one-line example. The same data
//! feeds the `commands` method, MCP tool schemas and `zebractl commands
//! --markdown`, so docs can't drift from the code.
use std::sync::OnceLock;

use serde_json::{Map, Value, json};
use zebra_core::{Command, Query, ViewCommand};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Command,
    View,
    Query,
}

impl Kind {
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Command | Kind::View => "op",
            Kind::Query => "q",
        }
    }
    pub fn method(self) -> &'static str {
        match self {
            Kind::Command => "apply",
            Kind::View => "view",
            Kind::Query => "query",
        }
    }
}

/// One-line examples. A unit test checks each one deserializes and that
/// every variant has one.
pub const EXAMPLES: &[(Kind, &str)] = &[
    // ---- document commands
    (Kind::Command, r#"{"op":"add_text","text":"Hello world","x":0,"y":-200,"style":"title"}"#),
    (Kind::Command, r#"{"op":"add_text","markdown":true,"text":"Why **zebras** have stripes\n\n- flies get *confused*\n- [more](https://example.com)","x":0,"y":200}"#),
    (Kind::Command, r#"{"op":"add_frame","x":0,"y":0,"w":800,"h":600,"style":"bracket","name":"Intro","add_to_path":true}"#),
    (Kind::Command, r#"{"op":"add_shape","shape":"arrow","x":500,"y":0,"w":300,"h":60,"fill":"theme:accent"}"#),
    (Kind::Command, r#"{"op":"add_shape","shape":"speech_bubble","x":0,"y":0,"w":260,"h":160}"#),
    (Kind::Command, r#"{"op":"add_connector","from":{"element":"s1"},"to":{"element":"s2"},"route":"elbow","end":"arrow","label":"then"}"#),
    (Kind::Command, r#"{"op":"add_icon","name":"lightbulb","x":0,"y":-300,"size":120,"color":"theme:accent"}"#),
    (Kind::Command, r#"{"op":"add_image","path":"/Users/me/logo.png","x":0,"y":300,"width":400,"alt":"Logo"}"#),
    (Kind::Command, r#"{"op":"add_video","path":"/Users/me/demo.mp4","x":0,"y":0,"width":960,"alt":"Product demo","loop":false}"#),
    (Kind::Command, r#"{"op":"replace_image","id":"i1","path":"/Users/me/new-logo.png"}"#),
    (Kind::Command, r#"{"op":"set_poster","id":"v1","path":"/Users/me/poster.jpg"}"#),
    (Kind::Command, r#"{"op":"set_soundtrack","path":"/Users/me/music.m4a","volume":0.4,"loop":true}"#),
    (Kind::Command, r#"{"op":"remove_soundtrack"}"#),
    (Kind::Command, r#"{"op":"set_voiceover","index":0,"path":"/Users/me/step1.m4a"}"#),
    (Kind::Command, r#"{"op":"add_ink","points":[[0,0],[50,40],[120,10]],"color":"theme:accent","width":6}"#),
    (Kind::Command, r#"{"op":"add_element","element":{"x":0,"y":0,"kind":{"type":"shape","shape":"ellipse"},"w":150,"h":150}}"#),
    (Kind::Command, r##"{"op":"update","id":"t1","patch":{"text":"New text","bold":true,"color":"#cc3333"}}"##),
    (Kind::Command, r##"{"op":"format_text","id":"t1","find":"zebras","format":{"bold":true,"color":"theme:accent"}}"##),
    (Kind::Command, r#"{"op":"format_paragraphs","id":"t1","first":1,"last":3,"list":"bullet","indent_by":1}"#),
    (Kind::Command, r#"{"op":"update","id":"i1","patch":{"crop":{"x":0.1,"y":0,"w":0.8,"h":1},"mask":"circle"}}"#),
    (Kind::Command, r#"{"op":"update","id":"v1","patch":{"start":2.5,"end":20,"muted":true,"autoplay":true}}"#),
    (Kind::Command, r#"{"op":"move","ids":["f1"],"dx":200,"dy":0}"#),
    (Kind::Command, r#"{"op":"scale","ids":["t1"],"factor":0.5}"#),
    (Kind::Command, r#"{"op":"rotate","ids":["f2"],"degrees":-15}"#),
    (Kind::Command, r#"{"op":"delete","ids":["s3"]}"#),
    (Kind::Command, r#"{"op":"duplicate","ids":["f1"],"dx":1000,"dy":0}"#),
    (Kind::Command, r#"{"op":"z_order","ids":["i1"],"order":"back"}"#),
    (Kind::Command, r#"{"op":"wrap_in_frame","ids":["t1","t2"],"style":"circle","add_to_path":true}"#),
    (Kind::Command, r#"{"op":"arrange","ids":["f1","f2","f3"],"layout":"row"}"#),
    (Kind::Command, r#"{"op":"align","ids":["t1","t2"],"edge":"left"}"#),
    (Kind::Command, r#"{"op":"add_text","text":"Ship on Friday","on":"s2"}"#),
    (Kind::Command, r#"{"op":"group","ids":["s1","t2","n1"]}"#),
    (Kind::Command, r#"{"op":"ungroup","ids":["s1"]}"#),
    (Kind::Command, r#"{"op":"add_step","target":{"kind":"element","id":"f1"},"builds":["t2","t3"],"notes":"Say hi"}"#),
    (Kind::Command, r#"{"op":"remove_step","index":2}"#),
    (Kind::Command, r#"{"op":"move_step","from":3,"to":0}"#),
    (Kind::Command, r#"{"op":"update_step","index":0,"notes":"Open with a question","duration":2.5,"dwell":8}"#),
    (Kind::Command, r#"{"op":"clear_path"}"#),
    (Kind::Command, r#"{"op":"auto_path"}"#),
    (Kind::Command, r#"{"op":"set_theme","theme":"chalkboard"}"#),
    (Kind::Command, r#"{"op":"set_title","title":"Quarterly review"}"#),
    (Kind::Command, r#"{"op":"set_playback","autoplay":10,"loop":true}"#),
    (Kind::Command, r##"{"op":"save_theme","apply":true,"theme":{"id":"my-theme","name":"My theme","background":"#101820","title":{"font":"Avenir Next","color":"#ffffff","size":72},"subtitle":{"font":"Sans","color":"#f2aa4c","size":40},"body":{"font":"Sans","color":"#dddddd","size":24},"frame":"#f2aa4c","frame_fill":"#1b2733","accent":"#f2aa4c","shapes":["#f2aa4c","#5fb3b3","#ffffff","#888888"],"layers":[{"asset":"<key from add_asset>","depth":0.2}]}}"##),
    (Kind::Command, r#"{"op":"delete_theme","id":"my-theme"}"#),
    (Kind::Command, r#"{"op":"add_asset","data_base64":"iVBORw0KGgo..."}"#),
    (Kind::Command, r#"{"op":"apply_template","template":"big_picture"}"#),
    (Kind::Command, r#"{"op":"add_layout","layout":"two_columns","x":4000,"y":0,"width":1600}"#),
    // ---- view commands
    (Kind::View, r#"{"op":"set_camera","camera":{"cx":0,"cy":0,"zoom":0.5,"rotation":0},"animate":true}"#),
    (Kind::View, r#"{"op":"zoom","factor":2}"#),
    (Kind::View, r#"{"op":"pan","dx":-200,"dy":0}"#),
    (Kind::View, r#"{"op":"zoom_to","id":"f1"}"#),
    (Kind::View, r#"{"op":"fit_all"}"#),
    (Kind::View, r#"{"op":"zoom_to_selection"}"#),
    (Kind::View, r#"{"op":"select","ids":["t1","t2"]}"#),
    (Kind::View, r#"{"op":"add_to_selection","ids":["s1"]}"#),
    (Kind::View, r#"{"op":"clear_selection"}"#),
    (Kind::View, r#"{"op":"select_all"}"#),
    (Kind::View, r#"{"op":"set_tool","tool":"pen"}"#),
    (Kind::View, r#"{"op":"set_frame_style","style":"circle"}"#),
    (Kind::View, r#"{"op":"set_mode","mode":"path_edit"}"#),
    (Kind::View, r#"{"op":"present","step":0}"#),
    (Kind::View, r#"{"op":"next"}"#),
    (Kind::View, r#"{"op":"prev"}"#),
    (Kind::View, r#"{"op":"goto_step","step":2}"#),
    (Kind::View, r#"{"op":"overview"}"#),
    (Kind::View, r#"{"op":"blank","blank":"black"}"#),
    (Kind::View, r#"{"op":"exit_present"}"#),
    (Kind::View, r#"{"op":"set_viewport","width":1280,"height":800}"#),
    (Kind::View, r#"{"op":"set_reduce_motion","on":true}"#),
    (Kind::View, r#"{"op":"follow_agent","on":true}"#),
    (Kind::View, r#"{"op":"pause_agents","paused":true}"#),
    (Kind::View, r#"{"op":"set_agent_permission","permission":"full"}"#),
    (Kind::View, r#"{"op":"pause_autoplay","paused":true}"#),
    (Kind::View, r#"{"op":"set_status","text":"Rewriting the intro"}"#),
    // ---- queries
    (Kind::Query, r#"{"q":"describe"}"#),
    (Kind::Query, r#"{"q":"state"}"#),
    (Kind::Query, r#"{"q":"document"}"#),
    (Kind::Query, r#"{"q":"icons","query":"idea","limit":10}"#),
    (Kind::Query, r#"{"q":"shapes"}"#),
    (Kind::Query, r#"{"q":"element","id":"t1"}"#),
    (Kind::Query, r#"{"q":"elements","kind":"text","within":"f1"}"#),
    (Kind::Query, r#"{"q":"hit_test","x":120,"y":-40}"#),
    (Kind::Query, r#"{"q":"hit_test_screen","x":640,"y":400}"#),
    (Kind::Query, r#"{"q":"path"}"#),
    (Kind::Query, r#"{"q":"themes"}"#),
    (Kind::Query, r#"{"q":"history"}"#),
    (Kind::Query, r#"{"q":"lint"}"#),
    (Kind::Query, r#"{"q":"templates"}"#),
    (Kind::Query, r#"{"q":"fonts"}"#),
    (Kind::Query, r#"{"q":"perf"}"#),
];

pub struct Schemas {
    pub command: Value,
    pub view: Value,
    pub query: Value,
}

pub fn schemas() -> &'static Schemas {
    static S: OnceLock<Schemas> = OnceLock::new();
    S.get_or_init(|| Schemas {
        command: schemars::schema_for!(Command).to_value(),
        view: schemars::schema_for!(ViewCommand).to_value(),
        query: schemars::schema_for!(Query).to_value(),
    })
}

pub fn schema_of(kind: Kind) -> &'static Value {
    let s = schemas();
    match kind {
        Kind::Command => &s.command,
        Kind::View => &s.view,
        Kind::Query => &s.query,
    }
}

/// The variant schemas of a tagged enum, as (name, schema) pairs.
pub fn variants(kind: Kind) -> Vec<(String, Value)> {
    let root = schema_of(kind);
    let tag = kind.tag();
    let list = root.get("oneOf").or(root.get("anyOf")).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    list.into_iter()
        .filter_map(|v| {
            let p = v.get("properties")?.get(tag)?;
            let name = p
                .get("const")
                .and_then(|c| c.as_str())
                .or_else(|| p.get("enum").and_then(|e| e.get(0)).and_then(|c| c.as_str()))?
                .to_string();
            Some((name, v))
        })
        .collect()
}

pub fn op_names(kind: Kind) -> Vec<String> {
    static CACHE: OnceLock<[Vec<String>; 3]> = OnceLock::new();
    let c = CACHE.get_or_init(|| {
        [
            variants(Kind::Command).into_iter().map(|v| v.0).collect(),
            variants(Kind::View).into_iter().map(|v| v.0).collect(),
            variants(Kind::Query).into_iter().map(|v| v.0).collect(),
        ]
    });
    c[kind as usize].clone()
}

pub fn example_for(op: &str) -> Option<&'static str> {
    EXAMPLES.iter().map(|(_, e)| *e).find(|e| {
        let v: Value = serde_json::from_str(e).unwrap_or(Value::Null);
        v.get("op").or(v.get("q")).and_then(|x| x.as_str()) == Some(op)
    })
}

fn example_in(kind: Kind, op: &str) -> Option<&'static str> {
    EXAMPLES.iter().filter(|(k, _)| *k == kind).find_map(|(_, e)| {
        let v: Value = serde_json::from_str(e).ok()?;
        (v.get(kind.tag()).and_then(|x| x.as_str()) == Some(op)).then_some(*e)
    })
}

/// Human/LLM-readable parameter summary of a variant schema:
/// `text: string, x?: number, style?: TextStyle`.
pub fn param_summary(kind: Kind, v: &Value) -> String {
    let Some(props) = v.get("properties").and_then(|p| p.as_object()) else { return String::new() };
    let req: Vec<&str> =
        v.get("required").and_then(|r| r.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    props
        .iter()
        .filter(|(k, _)| k.as_str() != kind.tag())
        .map(|(k, s)| format!("{k}{}: {}", if req.contains(&k.as_str()) { "" } else { "?" }, type_name(s)))
        .collect::<Vec<_>>()
        .join(", ")
        + if v.get("properties").and_then(|p| p.get("data_base64")).is_some() { " (or path: string, an absolute file path, instead of data_base64)" } else { "" }
}

fn type_name(s: &Value) -> String {
    if let Some(r) = s.get("$ref").and_then(|r| r.as_str()) {
        return r.rsplit('/').next().unwrap_or(r).to_string();
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(a) = s.get(key).and_then(|a| a.as_array()) {
            let parts: Vec<String> =
                a.iter().filter(|x| x.get("type").and_then(|t| t.as_str()) != Some("null")).map(type_name).collect();
            return parts.join(" | ");
        }
    }
    match s.get("type") {
        Some(Value::String(t)) if t == "array" => format!("{}[]", s.get("items").map(type_name).unwrap_or_else(|| "any".into())),
        Some(Value::String(t)) => match t.as_str() {
            "number" => "number".into(),
            "integer" => "integer".into(),
            other => other.to_string(),
        },
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).filter(|x| *x != "null").collect::<Vec<_>>().join("|"),
        _ => "any".into(),
    }
}

/// The `commands` method result.
pub fn catalogue_json() -> Value {
    let mut groups = Map::new();
    for kind in [Kind::Command, Kind::View, Kind::Query] {
        let list: Vec<Value> = variants(kind)
            .into_iter()
            .map(|(name, schema)| {
                json!({
                    "name": name,
                    "method": kind.method(),
                    "description": schema.get("description").cloned().unwrap_or(Value::Null),
                    "params": param_summary(kind, &schema),
                    "example": example_in(kind, &name).and_then(|e| serde_json::from_str::<Value>(e).ok()),
                    "schema": schema,
                })
            })
            .collect();
        let key = match kind {
            Kind::Command => "commands",
            Kind::View => "view_commands",
            Kind::Query => "queries",
        };
        groups.insert(key.into(), Value::Array(list));
    }
    let s = schemas();
    let mut defs = Map::new();
    for root in [&s.command, &s.view, &s.query] {
        if let Some(d) = root.get("$defs").and_then(|d| d.as_object()) {
            for (k, v) in d {
                defs.insert(k.clone(), v.clone());
            }
        }
    }
    groups.insert("$defs".into(), Value::Object(defs));
    groups.insert("api_version".into(), json!(zebra_core::API_VERSION));
    Value::Object(groups)
}

/// The `schema` method result: full JSON schemas of the three enums.
pub fn schema_json() -> Value {
    let s = schemas();
    json!({
        "api_version": zebra_core::API_VERSION,
        "command": s.command,
        "view_command": s.view,
        "query": s.query,
        "capture_target": capture_target_schema(),
        "events": crate::EVENT_NAMES,
        "methods": crate::protocol::METHODS,
    })
}

pub fn capture_target_schema() -> Value {
    json!({
        "description": "What to render. Short forms: \"viewport\" (the human's window), \"agent_view\" (your private camera), \"overview\" (everything), {\"step\": n, \"build\"?: k} (a path step, 0-based), {\"element\": \"f1\"} (fit one element), {\"camera\": {cx, cy, zoom, rotation}}.",
        "anyOf": [
            { "type": "string", "enum": ["viewport", "agent_view", "overview"] },
            { "type": "object", "properties": { "step": { "type": "integer", "minimum": 0 }, "build": { "type": "integer", "minimum": 0 } }, "required": ["step"] },
            { "type": "object", "properties": { "element": { "type": "string" } }, "required": ["element"] },
            { "type": "object", "properties": { "camera": { "type": "object", "properties": { "cx": {"type":"number"}, "cy": {"type":"number"}, "zoom": {"type":"number"}, "rotation": {"type":"number"} }, "required": ["cx","cy","zoom"] } }, "required": ["camera"] }
        ]
    })
}

/// A JSON schema (with `$defs` inlined at the root) for "one value of `kind`",
/// suitable for embedding as e.g. MCP tool input property under `#/$defs/<Name>`.
/// Returns (root_defs, name) so callers can reference `#/$defs/<name>`.
pub fn defs_with_root(kinds: &[Kind]) -> Map<String, Value> {
    let mut defs = Map::new();
    for &kind in kinds {
        let mut root = schema_of(kind).clone();
        if let Some(d) = root.as_object_mut().and_then(|o| o.remove("$defs")) {
            if let Some(d) = d.as_object() {
                for (k, v) in d {
                    defs.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(o) = root.as_object_mut() {
            o.remove("$schema");
        }
        defs.insert(root_name(kind).into(), root);
    }
    defs
}

pub fn root_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Command => "Command",
        Kind::View => "ViewCommand",
        Kind::Query => "Query",
    }
}

/// Compact text catalogue (for LLMs: `list_commands`).
pub fn catalogue_text() -> String {
    let mut out = String::new();
    for (kind, title) in [
        (Kind::Command, "Document commands (apply / batch / MCP apply_commands; undoable)"),
        (Kind::View, "View commands (view / MCP view; not undoable)"),
        (Kind::Query, "Queries (query / MCP query; read-only)"),
    ] {
        out.push_str(&format!("## {title}\n"));
        for (name, schema) in variants(kind) {
            let desc = schema.get("description").and_then(|d| d.as_str()).unwrap_or("").replace('\n', " ");
            out.push_str(&format!("- {name}({})", param_summary(kind, &schema)));
            if !desc.is_empty() {
                out.push_str(&format!(" — {desc}"));
            }
            if let Some(e) = example_in(kind, &name) {
                out.push_str(&format!("\n  e.g. {e}"));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    let s = schemas();
    out.push_str("## Types\n");
    let mut seen = std::collections::BTreeMap::new();
    for root in [&s.command, &s.view, &s.query] {
        if let Some(d) = root.get("$defs").and_then(|d| d.as_object()) {
            for (k, v) in d {
                seen.insert(k.clone(), v.clone());
            }
        }
    }
    for (k, v) in seen {
        out.push_str(&format!("- {k}: {}\n", def_summary(&v)));
    }
    out
}

fn def_summary(v: &Value) -> String {
    if let Some(e) = v.get("enum").and_then(|e| e.as_array()) {
        return e.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" | ");
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(a) = v.get(key).and_then(|a| a.as_array()) {
            let parts: Vec<String> = a
                .iter()
                .map(|x| {
                    if let Some(c) = x.get("const") {
                        return c.to_string();
                    }
                    if let Some(e) = x.get("enum").and_then(|e| e.as_array()) {
                        return e.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" | ");
                    }
                    if let Some(props) = x.get("properties").and_then(|p| p.as_object()) {
                        let inner: Vec<String> = props
                            .iter()
                            .map(|(k, s)| match s.get("const") {
                                Some(c) => format!("{k}: {c}"),
                                None => format!("{k}: {}", type_name(s)),
                            })
                            .collect();
                        return format!("{{{}}}", inner.join(", "));
                    }
                    type_name(x)
                })
                .collect();
            return parts.join(" | ");
        }
    }
    if let Some(props) = v.get("properties").and_then(|p| p.as_object()) {
        let req: Vec<&str> =
            v.get("required").and_then(|r| r.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
        let inner: Vec<String> = props
            .iter()
            .map(|(k, s)| format!("{k}{}: {}", if req.contains(&k.as_str()) { "" } else { "?" }, type_name(s)))
            .collect();
        return format!("{{{}}}", inner.join(", "));
    }
    type_name(v)
}

/// Markdown reference (for docs/API.md): `zebractl commands --markdown`.
pub fn catalogue_markdown() -> String {
    let mut out = String::new();
    for (kind, title, how) in [
        (Kind::Command, "Document commands", "Send with `apply {\"command\": ...}` or inside `batch {\"commands\": [...]}`. Undoable."),
        (Kind::View, "View commands", "Send with `view {\"command\": ...}`. Session state, not undoable."),
        (Kind::Query, "Queries", "Send with `query {\"query\": ...}`. Read-only."),
    ] {
        out.push_str(&format!("### {title}\n\n{how}\n\n| op | params | description | example |\n|---|---|---|---|\n"));
        for (name, schema) in variants(kind) {
            let desc = schema.get("description").and_then(|d| d.as_str()).unwrap_or("").replace('\n', " ").replace('|', "\\|");
            let ex = example_in(kind, &name).map(|e| format!("`{e}`")).unwrap_or_default().replace('|', "\\|");
            out.push_str(&format!("| `{name}` | {} | {desc} | {ex} |\n", param_summary(kind, &schema).replace('|', "\\|")));
        }
        out.push('\n');
    }
    out
}

/// Nearest strings by edit distance (for "did you mean").
pub fn nearest(target: &str, known: impl Iterator<Item = String>) -> Vec<String> {
    let mut v: Vec<(usize, String)> = known.map(|k| (edit_distance(&k, target), k)).collect();
    v.sort();
    v.into_iter().take(3).map(|x| x.1).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_valid_example() {
        for kind in [Kind::Command, Kind::View, Kind::Query] {
            let names = op_names(kind);
            assert!(!names.is_empty(), "no variants parsed for {kind:?}");
            for n in &names {
                assert!(example_in(kind, n).is_some(), "{kind:?} `{n}` has no example");
            }
        }
        for (kind, e) in EXAMPLES {
            let v: Value = serde_json::from_str(e).unwrap();
            let ok = match kind {
                Kind::Command => crate::protocol::parse_command(v.clone()).is_ok()
                    || v["op"] == "add_image", // example path does not exist here
                Kind::View => serde_json::from_value::<ViewCommand>(v).is_ok(),
                Kind::Query => serde_json::from_value::<Query>(v).is_ok(),
            };
            assert!(ok, "example does not parse: {e}");
        }
    }

    #[test]
    fn catalogue_renders() {
        let t = catalogue_text();
        assert!(t.contains("add_text("), "{t}");
        assert!(catalogue_markdown().contains("| `zoom_to` |"));
        assert!(catalogue_json()["commands"].as_array().unwrap().len() >= 20);
    }
}
