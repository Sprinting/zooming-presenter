//! Undo deltas: each history step stores the change as a pair of
//! [RFC 6902 JSON Patches](https://www.rfc-editor.org/rfc/rfc6902) (forward
//! and backward), not a copy of the document.
//!
//! Diffing and patching use the `json-patch` crate. Two adaptations keep
//! the patches small and the work proportional to the change, not to the
//! document:
//!
//! * Elements live in an id-keyed map. Only elements that differ (by
//!   `PartialEq`) are serialised, and `json_patch::diff` runs on just those,
//!   so a move stores `/elements/f3/transform/x` and nothing else.
//! * `json_patch::diff` compares arrays index by index, so moving one layer
//!   to the front of `z_order` would rewrite every entry. For the two
//!   ordered lists (`z_order`, `path`) the edit script comes from Myers'
//!   diff (`similar` crate) and is written out as RFC 6902 `add`/`remove`
//!   operations.
//!
//! Patches are applied with `json_patch::patch` to a pruned JSON view that
//! holds only the fields and elements the patch touches, then written back.
//!
//! Image/media assets are not part of the patch: they are content
//! addressed and only ever added during a session, so every history step
//! can refer to them by key.
use std::collections::{BTreeMap, BTreeSet};


use json_patch::{AddOperation, Patch, PatchOperation, RemoveOperation, ReplaceOperation};
use jsonptr::PointerBuf;
use serde::Serialize;
use serde_json::{Map, Value};
use similar::{capture_diff_slices, Algorithm, DiffOp};

use crate::model::{Document, Element, ElementId, PathStep};

/// One undoable change.
#[derive(Debug, Clone, PartialEq)]
pub struct Delta {
    pub forward: Patch,
    pub backward: Patch,
}

impl Delta {
    pub fn between(before: &Document, after: &Document) -> Delta {
        let ids = changed_ids(before, after);
        Delta { forward: diff_with(before, after, &ids), backward: diff_with(after, before, &ids) }
    }
    pub fn is_empty(&self) -> bool {
        self.forward.0.is_empty()
    }
    /// Rough heap size, for diagnostics.
    pub fn approx_bytes(&self) -> usize {
        fn ops(p: &Patch) -> usize {
            p.0.iter().map(|op| serde_json::to_string(op).map(|s| s.len()).unwrap_or(0)).sum()
        }
        ops(&self.forward) + ops(&self.backward)
    }
}

fn to_value<T: Serialize>(t: &T) -> Value {
    serde_json::to_value(t).expect("document types serialise")
}

fn ptr(parts: &[&str]) -> PointerBuf {
    PointerBuf::from_tokens(parts.iter().copied())
}

/// Ids of elements added, removed or changed between `a` and `b`.
fn changed_ids<'a>(a: &'a Document, b: &'a Document) -> Vec<&'a ElementId> {
    // Both maps are sorted: walk them together.
    let (mut x, mut y) = (a.elements.iter().peekable(), b.elements.iter().peekable());
    let mut ids = vec![];
    loop {
        match (x.peek(), y.peek()) {
            (Some((ka, ea)), Some((kb, eb))) => match ka.cmp(kb) {
                std::cmp::Ordering::Equal => {
                    if ea != eb {
                        ids.push(*ka);
                    }
                    x.next();
                    y.next();
                }
                std::cmp::Ordering::Less => ids.push(x.next().unwrap().0),
                std::cmp::Ordering::Greater => ids.push(y.next().unwrap().0),
            },
            (Some(_), None) => ids.push(x.next().unwrap().0),
            (None, Some(_)) => ids.push(y.next().unwrap().0),
            (None, None) => return ids,
        }
    }
}

/// A patch that turns `a` into `b`.
pub fn diff(a: &Document, b: &Document) -> Patch {
    diff_with(a, b, &changed_ids(a, b))
}

fn diff_with(a: &Document, b: &Document, ids: &[&ElementId]) -> Patch {
    let mut ops = vec![];
    let mut scalar = |name: &str, x: Value, y: Value| {
        if x != y {
            ops.push(PatchOperation::Replace(ReplaceOperation { path: ptr(&[name]), value: y }));
        }
    };
    scalar("format_version", to_value(&a.format_version), to_value(&b.format_version));
    scalar("title", to_value(&a.title), to_value(&b.title));
    scalar("theme", to_value(&a.theme), to_value(&b.theme));
    scalar("next_id", to_value(&a.next_id), to_value(&b.next_id));
    scalar("themes", to_value(&a.themes), to_value(&b.themes));
    scalar("play", to_value(&a.play), to_value(&b.play));

    // Elements: diff only the ones that differ.
    let (mut left, mut right) = (Map::new(), Map::new());
    for &id in ids {
        let (x, y) = (a.elements.get(id), b.elements.get(id));
        if x != y {
            if let Some(x) = x {
                left.insert(id.clone(), to_value(x));
            }
            if let Some(y) = y {
                right.insert(id.clone(), to_value(y));
            }
        }
    }
    if !left.is_empty() || !right.is_empty() {
        let mut l = Map::new();
        l.insert("elements".into(), Value::Object(left));
        let mut r = Map::new();
        r.insert("elements".into(), Value::Object(right));
        ops.extend(json_patch::diff(&Value::Object(l), &Value::Object(r)).0);
    }

    if a.soundtrack != b.soundtrack {
        // Absent when None, so add/remove rather than replace.
        match &b.soundtrack {
            Some(t) => ops.push(PatchOperation::Add(AddOperation { path: ptr(&["soundtrack"]), value: to_value(t) })),
            None => ops.push(PatchOperation::Remove(RemoveOperation { path: ptr(&["soundtrack"]) })),
        }
    }
    if a.z_order != b.z_order {
        list_ops(&mut ops, "z_order", &a.z_order, &b.z_order, |s| s.clone(), |s| Value::String(s.clone()));
    }
    if a.path != b.path {
        list_ops(&mut ops, "path", &a.path, &b.path, |s: &PathStep| serde_json::to_string(s).unwrap_or_default(), to_value);
    }
    Patch(ops)
}

/// RFC 6902 add/remove operations for an ordered list, from Myers' diff.
fn list_ops<T, K: Eq + std::hash::Hash>(
    ops: &mut Vec<PatchOperation>,
    field: &str,
    a: &[T],
    b: &[T],
    key: impl Fn(&T) -> K,
    value: impl Fn(&T) -> Value,
) {
    let (ka, kb): (Vec<K>, Vec<K>) = (a.iter().map(&key).collect(), b.iter().map(&key).collect());
    // Walking the ops in order, the list so far equals b[..new_index]
    // followed by a[old_index..], so every edit lands at `new_index`.
    for op in capture_diff_slices(Algorithm::Myers, &ka, &kb) {
        let (new_index, old_len, new_len) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete { new_index, old_len, .. } => (new_index, old_len, 0),
            DiffOp::Insert { new_index, new_len, .. } => (new_index, 0, new_len),
            DiffOp::Replace { new_index, old_len, new_len, .. } => (new_index, old_len, new_len),
        };
        let at = new_index.to_string();
        for _ in 0..old_len {
            ops.push(PatchOperation::Remove(RemoveOperation { path: ptr(&[field, &at]) }));
        }
        for k in 0..new_len {
            let i = (new_index + k).to_string();
            ops.push(PatchOperation::Add(AddOperation { path: ptr(&[field, &i]), value: value(&b[new_index + k]) }));
        }
    }
}

/// Apply a patch made by [`diff`] to `doc`. On error `doc` is unchanged.
pub fn apply(doc: &mut Document, patch: &Patch) -> Result<(), String> {
    if patch.0.is_empty() {
        return Ok(());
    }
    // Which parts of the document the patch touches.
    let mut fields = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for op in &patch.0 {
        let path = match op {
            PatchOperation::Add(o) => &o.path,
            PatchOperation::Remove(o) => &o.path,
            PatchOperation::Replace(o) => &o.path,
            PatchOperation::Test(o) => &o.path,
            PatchOperation::Move(_) | PatchOperation::Copy(_) => return Err("move/copy operations are not used in undo deltas".into()),
        };
        let mut t = path.tokens();
        let field = t.next().map(|t| t.decoded().into_owned()).ok_or("empty path")?;
        if field == "elements" {
            if let Some(id) = t.next() {
                ids.insert(id.decoded().into_owned());
            }
        }
        fields.insert(field);
    }

    // Pruned JSON view: just those fields and elements.
    let mut view = Map::new();
    for f in &fields {
        if f == "soundtrack" {
            // Absent when None (add/remove ops).
            if let Some(t) = &doc.soundtrack {
                view.insert(f.clone(), to_value(t));
            }
            continue;
        }
        let v = match f.as_str() {
            "format_version" => to_value(&doc.format_version),
            "title" => to_value(&doc.title),
            "theme" => to_value(&doc.theme),
            "next_id" => to_value(&doc.next_id),
            "themes" => to_value(&doc.themes),
            "play" => to_value(&doc.play),
            "z_order" => to_value(&doc.z_order),
            "path" => to_value(&doc.path),
            "elements" => Value::Object(ids.iter().filter_map(|id| doc.elements.get(id).map(|e| (id.clone(), to_value(e)))).collect()),
            other => return Err(format!("unknown document field '{other}'")),
        };
        view.insert(f.clone(), v);
    }
    let mut view = Value::Object(view);
    json_patch::patch(&mut view, patch).map_err(|e| e.to_string())?;

    // Decode everything before touching `doc`, so a failure changes nothing.
    let mut view = match view {
        Value::Object(m) => m,
        _ => return Err("patched view is not an object".into()),
    };
    fn take<T: serde::de::DeserializeOwned>(m: &mut Map<String, Value>, k: &str) -> Result<Option<T>, String> {
        m.remove(k).map(|v| serde_json::from_value(v).map_err(|e| format!("{k}: {e}"))).transpose()
    }
    let format_version: Option<u32> = take(&mut view, "format_version")?;
    let title: Option<String> = take(&mut view, "title")?;
    let theme: Option<String> = take(&mut view, "theme")?;
    let next_id: Option<u64> = take(&mut view, "next_id")?;
    let themes: Option<Vec<crate::theme::Theme>> = take(&mut view, "themes")?;
    let play: Option<crate::model::PlaySettings> = take(&mut view, "play")?;
    let z_order: Option<Vec<ElementId>> = take(&mut view, "z_order")?;
    let path: Option<Vec<PathStep>> = take(&mut view, "path")?;
    let elements: Option<BTreeMap<ElementId, Element>> = take(&mut view, "elements")?;
    let soundtrack: Option<crate::model::Soundtrack> = take(&mut view, "soundtrack")?;
    if fields.contains("soundtrack") {
        doc.soundtrack = soundtrack;
    }

    if let Some(v) = format_version {
        doc.format_version = v;
    }
    if let Some(v) = title {
        doc.title = v;
    }
    if let Some(v) = theme {
        doc.theme = v;
    }
    if let Some(v) = next_id {
        doc.next_id = v;
    }
    if let Some(v) = themes {
        doc.themes = v;
    }
    if let Some(v) = play {
        doc.play = v;
    }
    if let Some(v) = z_order {
        doc.z_order = v;
    }
    if let Some(v) = path {
        doc.path = v;
    }
    if let Some(mut els) = elements {
        for id in ids {
            match els.remove(&id) {
                Some(e) => {
                    doc.elements.insert(id, e);
                }
                None => {
                    doc.elements.remove(&id);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck() -> Document {
        let mut d = Document::default();
        let v: Value = serde_json::json!({
            "id": "x", "transform": {"x": 0.0, "y": 0.0, "scale": 1.0, "rotation": 0.0},
            "size": {"w": 10.0, "h": 10.0}, "kind": {"type": "shape", "shape": "rectangle"}
        });
        for i in 0..50 {
            let mut v = v.clone();
            v["id"] = format!("e{i}").into();
            if let Ok(e) = serde_json::from_value::<Element>(v) {
                d.elements.insert(e.id.clone(), e);
                d.z_order.push(format!("e{i}"));
            }
        }
        d
    }

    #[test]
    fn bring_to_front_is_two_ops_not_fifty() {
        let a = deck();
        assert_eq!(a.elements.len(), 50, "fixture must build");
        let mut b = a.clone();
        let id = b.z_order.remove(10);
        b.z_order.push(id);
        let p = diff(&a, &b);
        assert_eq!(p.0.len(), 2, "{p:?}");
        let mut c = a.clone();
        apply(&mut c, &p).unwrap();
        assert_eq!(c, b);
    }

    #[test]
    fn move_one_element_stores_only_its_coordinates() {
        let a = deck();
        let mut b = a.clone();
        b.elements.get_mut("e7").unwrap().transform.x = 42.0;
        let d = Delta::between(&a, &b);
        assert_eq!(d.forward.0.len(), 1);
        assert_eq!(d.forward.0[0].to_string().contains("/elements/e7/transform/x"), true);
        let mut c = b.clone();
        apply(&mut c, &d.backward).unwrap();
        assert_eq!(c, a);
    }

    #[test]
    fn add_and_delete_round_trip() {
        let a = deck();
        let mut b = a.clone();
        b.elements.remove("e3");
        b.z_order.retain(|z| z != "e3");
        let mut e = a.elements["e4"].clone();
        e.id = "new/one~".into(); // needs JSON-pointer escaping
        b.elements.insert(e.id.clone(), e);
        b.z_order.insert(0, "new/one~".into());
        b.title = "T".into();
        let d = Delta::between(&a, &b);
        let mut c = a.clone();
        apply(&mut c, &d.forward).unwrap();
        assert_eq!(c, b);
        apply(&mut c, &d.backward).unwrap();
        assert_eq!(c, a);
    }
}
