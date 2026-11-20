//! Unknown-field checks. serde ignores fields it doesn't know, so a typo
//! (`"colour"`) or a field a command doesn't have (`add_ink {"id": ...}`)
//! used to be dropped without a word. `deny_unknown_fields` can't be used on
//! the core types (internally tagged enums, `#[serde(flatten)]`, and old
//! files must keep loading), so requests are checked here instead: commands,
//! view commands and queries against their catalogue schema (nested objects
//! too), and method params against [`METHOD_PARAMS`].
use serde_json::{Map, Value};

use crate::catalogue::{self, Kind};
use crate::protocol::RpcError;

/// An unknown field: where it is and what would have been accepted there.
#[derive(Debug, Clone, PartialEq)]
pub struct Unknown {
    /// Dotted path from the command, e.g. `patch.colour` or `kind.radius`.
    pub field: String,
    /// The fields accepted in the object holding it, sorted.
    pub accepted: Vec<String>,
}

/// The params each method accepts (besides `document`, which every method
/// takes and the server removes before parsing). For `apply`, `view` and
/// `query` the command itself may also be sent flat (`{"op": ...}`): those
/// are checked against the command's own fields.
pub const METHOD_PARAMS: &[(&str, &[&str])] = &[
    ("hello", &["name", "token"]),
    ("ping", &[]),
    ("methods", &[]),
    ("commands", &[]),
    ("schema", &[]),
    ("subscribe", &["events"]),
    ("unsubscribe", &[]),
    ("documents", &[]),
    ("new_document", &[]),
    ("close_document", &["discard"]),
    ("state", &[]),
    ("describe", &[]),
    ("query", &["query"]),
    ("apply", &["command", "label", "if_revision"]),
    ("batch", &["commands", "label", "if_revision"]),
    ("copy_to", &["ids", "to", "at", "dx", "dy"]),
    ("import", &["path", "from", "to"]),
    ("begin", &["label"]),
    ("commit", &[]),
    ("abort", &[]),
    ("undo", &[]),
    ("redo", &[]),
    ("view", &["command"]),
    ("capture", &["target", "width", "height", "path"]),
    ("wait", &["until", "timeout_ms"]),
    ("open", &["path", "force"]),
    ("save", &["path"]),
    ("save_template", &["name"]),
    ("export", &["format", "path", "target", "width", "height"]),
    ("disconnect", &[]),
];

pub fn method_params(method: &str) -> Option<&'static [&'static str]> {
    METHOD_PARAMS.iter().find(|(m, _)| *m == method).map(|(_, p)| *p)
}

/// Refuse keys of `o` not in `accepted`. `what` names the thing in the
/// message ("method `capture`", "capture target").
pub fn check_keys(what: &str, o: &Map<String, Value>, accepted: &[&str]) -> Result<(), RpcError> {
    match o.keys().find(|k| !accepted.contains(&k.as_str())) {
        None => Ok(()),
        Some(k) => Err(unknown_error(what, k, accepted.iter().map(|s| s.to_string()).collect())),
    }
}

pub fn unknown_error(what: &str, field: &str, mut accepted: Vec<String>) -> RpcError {
    accepted.sort();
    let list = if accepted.is_empty() { "none".to_string() } else { accepted.join(", ") };
    let mut e = RpcError::invalid(format!("unknown field `{field}` in {what}; accepted fields: {list}"));
    e.data["field"] = Value::String(field.to_string());
    e.values(accepted)
}

/// Check a command / view command / query (`{"op": ...}` or `{"q": ...}`)
/// for unknown fields, at any depth. Unknown ops pass: deserializing them
/// gives the better error.
pub fn check(kind: Kind, v: &Value) -> Result<(), Unknown> {
    let root = catalogue::schema_of(kind);
    let defs = root.get("$defs").and_then(|d| d.as_object());
    let mut w = Walker { defs, tag: kind.tag() };
    w.walk(root, v, "")
}

/// [`check`] as an API error naming the field and the op.
pub fn check_rpc(kind: Kind, v: &Value) -> Result<(), RpcError> {
    check(kind, v).map_err(|u| {
        let op = v.get(kind.tag()).and_then(|o| o.as_str()).unwrap_or("?");
        let what = match kind {
            Kind::Command => format!("command `{op}`"),
            Kind::View => format!("view command `{op}`"),
            Kind::Query => format!("query `{op}`"),
        };
        let mut e = unknown_error(&what, &u.field, u.accepted);
        if let Some(ex) = catalogue::example_for(op) {
            e = e.hint(format!("example: {ex}"));
        }
        e
    })
}

struct Walker<'a> {
    defs: Option<&'a Map<String, Value>>,
    tag: &'static str,
}

impl<'a> Walker<'a> {
    /// `s` and what its `$ref` chain points to: one schema, read together
    /// (a `$ref` may sit beside its own keys, e.g. a tagged enum's
    /// `{"$ref": "#/$defs/ShapeData", "properties": {"type": ...}}`).
    fn chain(&self, s: &'a Value) -> Vec<&'a Value> {
        let mut out = vec![s];
        let mut cur = s;
        // `$ref` chains are short; a bound keeps a bad schema from looping.
        for _ in 0..16 {
            let Some(name) = cur.get("$ref").and_then(|r| r.as_str()).and_then(|r| r.strip_prefix("#/$defs/")) else { break };
            let Some(t) = self.defs.and_then(|d| d.get(name)) else { break };
            out.push(t);
            cur = t;
        }
        out
    }

    fn alternatives(&self, s: &'a Value) -> Option<&'a Vec<Value>> {
        self.chain(s).into_iter().find_map(alternatives)
    }

    fn walk(&mut self, s: &'a Value, v: &Value, path: &str) -> Result<(), Unknown> {
        if let Some(alts) = self.alternatives(s) {
            let cands: Vec<&Value> = alts.iter().filter(|a| self.compatible(a, v)).collect();
            // No branch fits the value's type or tag: deserializing says why.
            let mut first = None;
            for a in cands {
                match self.walk_with(Some(s), a, v, path) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        first.get_or_insert(e);
                    }
                }
            }
            return first.map_or(Ok(()), Err);
        }
        self.walk_with(None, s, v, path)
    }

    /// Check `v` against `s`, plus the properties `outer` (the schema
    /// holding the `anyOf`/`oneOf` that `s` is a branch of) shares with
    /// every branch.
    fn walk_with(&mut self, outer: Option<&'a Value>, s: &'a Value, v: &Value, path: &str) -> Result<(), Unknown> {
        if outer.is_some() && self.alternatives(s).is_some() {
            return self.walk(s, v, path);
        }
        match v {
            Value::Object(o) => {
                let mut props: Vec<(&'a str, &'a Value)> = vec![];
                let mut open = false;
                let mut extra: Option<&'a Value> = None;
                self.collect(s, &mut props, &mut open, &mut extra);
                if let Some(outer) = outer {
                    for part in self.chain(outer) {
                        if let Some(p) = part.get("properties").and_then(|p| p.as_object()) {
                            props.extend(p.iter().map(|(k, v)| (k.as_str(), v)));
                        }
                    }
                }
                if props.is_empty() && extra.is_none() {
                    // A free-form value (`serde_json::Value`, `{}`).
                    open = true;
                }
                for (k, val) in o {
                    let sub = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                    match props.iter().find(|(p, _)| *p == k) {
                        Some((_, ps)) => self.walk(ps, val, &sub)?,
                        None => match extra {
                            Some(e) => self.walk(e, val, &sub)?,
                            None if open => {}
                            None => {
                                let top = path.is_empty();
                                let mut accepted: Vec<String> =
                                    props.iter().map(|(p, _)| p.to_string()).filter(|p| !(top && p == self.tag)).collect();
                                accepted.sort();
                                accepted.dedup();
                                return Err(Unknown { field: sub, accepted });
                            }
                        },
                    }
                }
                Ok(())
            }
            Value::Array(a) => {
                for part in self.chain(s) {
                    if let Some(prefix) = part.get("prefixItems").and_then(|p| p.as_array()) {
                        for (i, (x, ps)) in a.iter().zip(prefix).enumerate() {
                            self.walk(ps, x, &format!("{path}[{i}]"))?;
                        }
                        break;
                    }
                    if let Some(items) = part.get("items") {
                        if let Some(list) = items.as_array() {
                            for (i, (x, ps)) in a.iter().zip(list).enumerate() {
                                self.walk(ps, x, &format!("{path}[{i}]"))?;
                            }
                        } else {
                            for (i, x) in a.iter().enumerate() {
                                self.walk(items, x, &format!("{path}[{i}]"))?;
                            }
                        }
                        break;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Properties of `s` (with its `$ref` chain) and of its `allOf` parts.
    fn collect(&self, s: &'a Value, props: &mut Vec<(&'a str, &'a Value)>, open: &mut bool, extra: &mut Option<&'a Value>) {
        for part in self.chain(s) {
            if let Some(p) = part.get("properties").and_then(|p| p.as_object()) {
                props.extend(p.iter().map(|(k, v)| (k.as_str(), v)));
            }
            match part.get("additionalProperties") {
                None | Some(Value::Bool(false)) => {}
                Some(Value::Bool(true)) => *open = true,
                Some(e) => *extra = Some(e),
            }
            if part.get("patternProperties").is_some() {
                *open = true;
            }
            if let Some(parts) = part.get("allOf").and_then(|a| a.as_array()) {
                for p in parts {
                    if self.alternatives(p).is_some() {
                        // A flattened enum: its fields depend on the
                        // branch; don't guess, accept anything.
                        *open = true;
                    }
                    self.collect(p, props, open, extra);
                }
            }
        }
    }

    /// Could `v` be a value of schema `a`? Judged by JSON type and by
    /// constant (tag) properties, not by field names.
    fn compatible(&self, a: &'a Value, v: &Value) -> bool {
        if let Value::Bool(b) = a {
            return *b;
        }
        if let Some(alts) = self.alternatives(a) {
            if !alts.iter().any(|x| self.compatible(x, v)) {
                return false;
            }
        }
        let ty = json_type(v);
        for part in self.chain(a) {
            if let Some(c) = part.get("const") {
                if c != v {
                    return false;
                }
            }
            if let Some(e) = part.get("enum").and_then(|e| e.as_array()) {
                if !e.contains(v) {
                    return false;
                }
            }
            let type_ok = match part.get("type") {
                None => !(part.get("properties").is_some() && ty != "object"),
                Some(Value::String(t)) => type_matches(t, ty),
                Some(Value::Array(ts)) => ts.iter().filter_map(|t| t.as_str()).any(|t| type_matches(t, ty)),
                Some(_) => true,
            };
            if !type_ok {
                return false;
            }
            if let (Value::Object(o), Some(props)) = (v, part.get("properties").and_then(|p| p.as_object())) {
                for (k, ps) in props {
                    let fixed = self.chain(ps).into_iter().find_map(|p| {
                        p.get("const").or_else(|| p.get("enum").and_then(|e| e.as_array()).filter(|e| e.len() == 1).and_then(|e| e.first()))
                    });
                    if let Some(c) = fixed {
                        if o.get(k) != Some(c) {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}

fn alternatives(s: &Value) -> Option<&Vec<Value>> {
    s.get("anyOf").or_else(|| s.get("oneOf")).and_then(|a| a.as_array())
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_matches(schema_ty: &str, value_ty: &str) -> bool {
    schema_ty == value_ty || (schema_ty == "number" && value_ty == "integer")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn top_level_unknown_field_is_named() {
        let e = check(Kind::Command, &json!({"op":"add_ink","points":[[0,0],[1,1]],"colour":"#ff0000"})).unwrap_err();
        assert_eq!(e.field, "colour");
        assert!(e.accepted.contains(&"points".to_string()), "{e:?}");
        assert!(!e.accepted.contains(&"op".to_string()), "{e:?}");
    }

    #[test]
    fn known_fields_pass() {
        for (kind, ex) in catalogue::EXAMPLES {
            let mut v: Value = serde_json::from_str(ex).unwrap();
            if v["op"] == "add_image" {
                // parse_command reads `path` into `data_base64` first.
                let o = v.as_object_mut().unwrap();
                o.remove("path");
                o.insert("data_base64".into(), json!(""));
            }
            assert_eq!(check(*kind, &v), Ok(()), "{ex}");
        }
    }

    #[test]
    fn nested_unknown_fields_are_found() {
        let e = check(Kind::Command, &json!({"op":"update","id":"t1","patch":{"text":"x","colour":"#ff0000"}})).unwrap_err();
        assert_eq!(e.field, "patch.colour");
        assert!(e.accepted.contains(&"color".to_string()), "{e:?}");
        // A tagged enum whose branch is a `$ref` beside its tag.
        let e = check(Kind::Command, &json!({"op":"add_element","element":{"kind":{"type":"shape","shape":"ellipse","radius":3}}})).unwrap_err();
        assert_eq!(e.field, "element.kind.radius");
        assert!(e.accepted.contains(&"shape".to_string()) && e.accepted.contains(&"type".to_string()), "{e:?}");
        // Inside an array.
        let e = check(Kind::Command, &json!({"op":"add_step","target":{"kind":"element","id":"f1","zoom":2}})).unwrap_err();
        assert_eq!(e.field, "target.zoom");
        // Maps take any key.
        let ok = json!({"op":"save_theme","theme":{"id":"t","name":"T","fonts":{"anything":["Avenir"]}}});
        assert!(check(Kind::Command, &ok).is_ok() || serde_json::from_value::<zebra_core::Command>(ok).is_err());
    }

    #[test]
    fn view_commands_and_queries_too() {
        assert_eq!(check(Kind::View, &json!({"op":"zoom_to","id":"f1","speed":2})).unwrap_err().field, "speed");
        assert_eq!(check(Kind::Query, &json!({"q":"elements","kind":"text","inside":"f1"})).unwrap_err().field, "inside");
        assert_eq!(check(Kind::Query, &json!({"q":"path","verbose":true})).unwrap_err().accepted, Vec::<String>::new());
    }

    #[test]
    fn unknown_op_is_left_to_serde() {
        assert_eq!(check(Kind::Command, &json!({"op":"add_txet","text":"x"})), Ok(()));
    }
}
