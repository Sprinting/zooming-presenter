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
//! The two ordered lists are not patched as JSON at all: their edit script
//! is kept as [`Splice`]s (remove this run, insert these items) and applied
//! to the `Vec` in one pass. Undo of a change in a 100,000-element deck
//! used to serialise and re-parse the whole draw order and path.
//!
//! An element whose `kind` didn't change (a move, a resize, a rename) is
//! diffed and patched without its `kind`, which holds most of its bytes
//! (a text's runs, an ink stroke's points): moving 100,000 elements and
//! undoing it no longer serialises every text twice.
//!
//! The editor doesn't copy the document to make a delta: it keeps the old
//! values of what a command touched (a [`Before`]) and
//! [`Delta::from_before`] diffs those against the document as it is now.
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
    pub forward: Change,
    pub backward: Change,
}

/// One direction of a [`Delta`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Change {
    /// Everything but the two ordered lists.
    pub patch: Patch,
    pub z_order: Vec<Splice<ElementId>>,
    pub path: Vec<Splice<PathStep>>,
}

/// Replace `remove` items at `at` (an index in the list *before* the
/// change) with `insert`. A change's splices are in rising order of `at`
/// and never overlap.
#[derive(Debug, Clone, PartialEq)]
pub struct Splice<T> {
    pub at: usize,
    pub remove: usize,
    pub insert: Vec<T>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.patch.0.is_empty() && self.z_order.is_empty() && self.path.is_empty()
    }
    /// Operations in all (a list edit counts one per item moved).
    pub fn op_count(&self) -> usize {
        fn n<T>(v: &[Splice<T>]) -> usize {
            v.iter().map(|s| s.remove + s.insert.len()).sum()
        }
        self.patch.0.len() + n(&self.z_order) + n(&self.path)
    }
    /// Ids of the elements this change adds, removes or edits; `None`
    /// when it rewrites the element map as a whole.
    pub fn element_ids(&self) -> Option<BTreeSet<String>> {
        let mut ids = BTreeSet::new();
        for op in &self.patch.0 {
            let path = match op {
                PatchOperation::Add(o) => &o.path,
                PatchOperation::Remove(o) => &o.path,
                PatchOperation::Replace(o) => &o.path,
                PatchOperation::Test(o) => &o.path,
                PatchOperation::Move(o) => &o.path,
                PatchOperation::Copy(o) => &o.path,
            };
            let mut t = path.tokens();
            if t.next().is_some_and(|f| f.decoded() == "elements") {
                ids.insert(t.next()?.decoded().into_owned());
            }
        }
        Some(ids)
    }
    fn approx_bytes(&self) -> usize {
        let ops: usize = self.patch.0.iter().map(|op| serde_json::to_string(op).map(|s| s.len()).unwrap_or(0)).sum();
        let z: usize = self.z_order.iter().map(|s| 24 + s.insert.iter().map(|i| i.len() + 24).sum::<usize>()).sum();
        let p: usize = self.path.iter().map(|s| 24 + s.insert.iter().map(|i| serde_json::to_string(i).map(|s| s.len()).unwrap_or(0)).sum::<usize>()).sum();
        ops + z + p
    }
}

/// What a change touched, as it was before: the editor's record of a
/// command, instead of a copy of the whole document.
#[derive(Debug, Clone, Copy)]
pub struct Before<'a> {
    /// Every field but `elements`, `z_order` and `path`, which are ignored.
    pub meta: &'a Document,
    /// Every element that may have changed, as it was (`None`: it didn't
    /// exist). Elements not listed are the same before and after.
    pub elements: &'a BTreeMap<ElementId, Option<Element>>,
    /// The draw order as it was, or `None` when it is unchanged.
    pub z_order: Option<&'a [ElementId]>,
    /// The path as it was, or `None` when it is unchanged.
    pub path: Option<&'a [PathStep]>,
}

/// One side of a diff.
struct Side<'a> {
    meta: &'a Document,
    z_order: &'a [ElementId],
    path: &'a [PathStep],
    elements: Elements<'a>,
}

enum Elements<'a> {
    All(&'a BTreeMap<ElementId, Element>),
    Touched(&'a BTreeMap<ElementId, Option<Element>>),
}

impl<'a> Side<'a> {
    fn of(doc: &'a Document) -> Side<'a> {
        Side { meta: doc, z_order: &doc.z_order, path: &doc.path, elements: Elements::All(&doc.elements) }
    }
    fn element(&self, id: &str) -> Option<&'a Element> {
        match self.elements {
            Elements::All(m) => m.get(id),
            Elements::Touched(m) => m.get(id).and_then(|e| e.as_ref()),
        }
    }
}

impl Delta {
    pub fn between(before: &Document, after: &Document) -> Delta {
        let ids = changed_ids(before, after);
        let (a, b) = (Side::of(before), Side::of(after));
        Delta { forward: diff_sides(&a, &b, &ids, true), backward: diff_sides(&b, &a, &ids, true) }
    }

    /// The change from `before` (what was touched, as it was) to `after`.
    /// The same as [`Delta::between`] the whole old document and `after`,
    /// at a cost that follows the change, not the document.
    pub fn from_before(before: &Before, after: &Document) -> Delta {
        let ids: Vec<&ElementId> = before.elements.iter().filter(|(id, old)| old.as_ref() != after.elements.get(*id)).map(|(id, _)| id).collect();
        let a = Side { meta: before.meta, z_order: before.z_order.unwrap_or(&after.z_order), path: before.path.unwrap_or(&after.path), elements: Elements::Touched(before.elements) };
        let b = Side::of(after);
        let lists = before.z_order.is_some() || before.path.is_some();
        Delta { forward: diff_sides(&a, &b, &ids, lists), backward: diff_sides(&b, &a, &ids, lists) }
    }
    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }
    /// Rough heap size, for diagnostics.
    pub fn approx_bytes(&self) -> usize {
        self.forward.approx_bytes() + self.backward.approx_bytes()
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

/// Elements that may look different in `b` than in `a`: those added,
/// removed or edited, and those that moved in the draw order. For caches
/// of drawings (path thumbnails), which redraw only where these are.
pub fn visual_changes(a: &Document, b: &Document) -> Vec<ElementId> {
    let mut ids: Vec<ElementId> = changed_ids(a, b).into_iter().cloned().collect();
    if a.z_order != b.z_order {
        for s in splices(&a.z_order, &b.z_order, |s| s.as_str()) {
            ids.extend(a.z_order[s.at..s.at + s.remove].iter().cloned());
            ids.extend(s.insert);
        }
    }
    ids
}

/// A change that turns `a` into `b`.
pub fn diff(a: &Document, b: &Document) -> Change {
    diff_sides(&Side::of(a), &Side::of(b), &changed_ids(a, b), true)
}

/// An element without its `kind`, read as [`Element`] reads it.
#[derive(serde::Deserialize)]
struct Head {
    id: ElementId,
    transform: crate::geom::Transform,
    size: crate::model::Size,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    name: String,
    #[serde(default)]
    placeholder: bool,
    #[serde(default)]
    group: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn head_value(e: &Element) -> Value {
    #[derive(Serialize)]
    struct HeadRef<'a> {
        id: &'a ElementId,
        transform: &'a crate::geom::Transform,
        size: &'a crate::model::Size,
        locked: bool,
        #[serde(skip_serializing_if = "String::is_empty")]
        name: &'a String,
        #[serde(skip_serializing_if = "is_false")]
        placeholder: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        group: &'a Option<String>,
    }
    to_value(&HeadRef { id: &e.id, transform: &e.transform, size: &e.size, locked: e.locked, name: &e.name, placeholder: e.placeholder, group: &e.group })
}

/// Set `e`'s head fields (everything but `kind`).
fn set_head(e: &mut Element, h: Head) {
    (e.id, e.transform, e.size, e.locked, e.name, e.placeholder, e.group) = (h.id, h.transform, h.size, h.locked, h.name, h.placeholder, h.group);
}

/// Two numbers that differ (as JSON would see them: NaN serialises as
/// null, the same as another NaN).
fn differ(a: f64, b: f64) -> bool {
    a != b && !(a.is_nan() && b.is_nan())
}

/// The ops a JSON diff of `x`'s and `y`'s heads would give (same kind),
/// without serialising either: a move of 100,000 elements is 200,000 small
/// ops, not 200,000 elements turned into JSON and compared.
fn head_ops(id: &str, x: &Element, y: &Element, ops: &mut Vec<PatchOperation>) {
    let at = |f: &[&str]| PointerBuf::from_tokens(["elements", id].iter().chain(f).copied());
    let replace = |ops: &mut Vec<PatchOperation>, f: &[&str], value: Value| ops.push(PatchOperation::Replace(ReplaceOperation { path: at(f), value }));
    // A field left out of the JSON when it has its default.
    let optional = |ops: &mut Vec<PatchOperation>, f: &str, a: Option<Value>, b: Option<Value>| match (a, b) {
        (None, Some(v)) => ops.push(PatchOperation::Add(AddOperation { path: at(&[f]), value: v })),
        (Some(_), None) => ops.push(PatchOperation::Remove(RemoveOperation { path: at(&[f]) })),
        (Some(a), Some(b)) if a != b => replace(ops, &[f], b),
        _ => {}
    };
    if x.id != y.id {
        replace(ops, &["id"], to_value(&y.id));
    }
    let (tx, ty) = (&x.transform, &y.transform);
    for (f, a, b) in [("rotation", tx.rotation, ty.rotation), ("scale", tx.scale, ty.scale), ("x", tx.x, ty.x), ("y", tx.y, ty.y)] {
        if differ(a, b) {
            replace(ops, &["transform", f], to_value(&b));
        }
    }
    for (f, a, b) in [("h", x.size.h, y.size.h), ("w", x.size.w, y.size.w)] {
        if differ(a, b) {
            replace(ops, &["size", f], to_value(&b));
        }
    }
    if x.locked != y.locked {
        replace(ops, &["locked"], to_value(&y.locked));
    }
    let name = |e: &Element| (!e.name.is_empty()).then(|| to_value(&e.name));
    optional(ops, "name", name(x), name(y));
    let placeholder = |e: &Element| e.placeholder.then(|| to_value(&true));
    optional(ops, "placeholder", placeholder(x), placeholder(y));
    optional(ops, "group", x.group.as_ref().map(to_value), y.group.as_ref().map(to_value));
}

/// Apply one op made by [`head_ops`] (`field` is the path after
/// `/elements/<id>`) straight to `e`. `None`: not one of those ops.
fn head_op(e: &mut Element, field: &[String], op: &PatchOperation) -> Option<Result<(), String>> {
    fn get<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, String> {
        T::deserialize(v).map_err(|err| format!("elements: {err}"))
    }
    let f: Vec<&str> = field.iter().map(String::as_str).collect();
    let r = match (op, f.as_slice()) {
        (PatchOperation::Replace(o), ["id"]) => get(&o.value).map(|v| e.id = v),
        (PatchOperation::Replace(o), ["transform", k]) => get(&o.value).and_then(|v| match *k {
            "x" => Ok(e.transform.x = v),
            "y" => Ok(e.transform.y = v),
            "scale" => Ok(e.transform.scale = v),
            "rotation" => Ok(e.transform.rotation = v),
            _ => Err(format!("elements: no transform field '{k}'")),
        }),
        (PatchOperation::Replace(o), ["size", k]) => get(&o.value).and_then(|v| match *k {
            "w" => Ok(e.size.w = v),
            "h" => Ok(e.size.h = v),
            _ => Err(format!("elements: no size field '{k}'")),
        }),
        (PatchOperation::Replace(o), ["locked"]) => get(&o.value).map(|v| e.locked = v),
        (PatchOperation::Add(o), ["name"]) => get(&o.value).map(|v| e.name = v),
        (PatchOperation::Replace(o), ["name"]) => get(&o.value).map(|v| e.name = v),
        (PatchOperation::Remove(_), ["name"]) => Ok(e.name.clear()),
        (PatchOperation::Add(o), ["placeholder"]) => get(&o.value).map(|v| e.placeholder = v),
        (PatchOperation::Replace(o), ["placeholder"]) => get(&o.value).map(|v| e.placeholder = v),
        (PatchOperation::Remove(_), ["placeholder"]) => Ok(e.placeholder = false),
        (PatchOperation::Add(o), ["group"]) => get(&o.value).map(|v| e.group = Some(v)),
        (PatchOperation::Replace(o), ["group"]) => get(&o.value).map(|v| e.group = Some(v)),
        (PatchOperation::Remove(_), ["group"]) => Ok(e.group = None),
        _ => return None,
    };
    Some(r)
}

fn diff_sides(a: &Side, b: &Side, ids: &[&ElementId], lists: bool) -> Change {
    let sides = (a, b);
    let (a_doc, b_doc) = (a.meta, b.meta);
    let mut ops = vec![];
    let mut scalar = |name: &str, x: Value, y: Value| {
        if x != y {
            ops.push(PatchOperation::Replace(ReplaceOperation { path: ptr(&[name]), value: y }));
        }
    };
    let (a, b) = (a_doc, b_doc);
    scalar("format_version", to_value(&a.format_version), to_value(&b.format_version));
    scalar("title", to_value(&a.title), to_value(&b.title));
    scalar("theme", to_value(&a.theme), to_value(&b.theme));
    scalar("next_id", to_value(&a.next_id), to_value(&b.next_id));
    if a.themes != b.themes {
        scalar("themes", to_value(&a.themes), to_value(&b.themes));
    }
    scalar("play", to_value(&a.play), to_value(&b.play));
    let (a, b) = (sides.0, sides.1);

    // Elements: diff only the ones that differ, and only their heads when
    // their kinds are the same.
    let (mut left, mut right) = (Map::new(), Map::new());
    for &id in ids {
        let (x, y) = (a.element(id), b.element(id));
        match (x, y) {
            (Some(x), Some(y)) if x == y => {}
            (Some(x), Some(y)) if x.kind == y.kind => head_ops(id, x, y, &mut ops),
            (x, y) => {
                if let Some(x) = x {
                    left.insert(id.clone(), to_value(x));
                }
                if let Some(y) = y {
                    right.insert(id.clone(), to_value(y));
                }
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

    if a_doc.soundtrack != b_doc.soundtrack {
        // Absent when None, so add/remove rather than replace.
        match &b_doc.soundtrack {
            Some(t) => ops.push(PatchOperation::Add(AddOperation { path: ptr(&["soundtrack"]), value: to_value(t) })),
            None => ops.push(PatchOperation::Remove(RemoveOperation { path: ptr(&["soundtrack"]) })),
        }
    }
    let z_order = if lists && a.z_order != b.z_order { splices(a.z_order, b.z_order, |s| s.as_str()) } else { vec![] };
    let path = if lists && a.path != b.path { splices(a.path, b.path, |s: &PathStep| serde_json::to_string(s).unwrap_or_default()) } else { vec![] };
    Change { patch: Patch(ops), z_order, path }
}

/// The edit script for an ordered list, from Myers' diff (a layer brought
/// to the front is one removal and one insertion, not a rewrite).
fn splices<'a, T: Clone, K: Eq + std::hash::Hash + Ord>(a: &'a [T], b: &'a [T], key: impl Fn(&'a T) -> K) -> Vec<Splice<T>> {
    // The usual edit touches one end or one run: trim what both lists
    // share at the ends before diffing, so the diff sees only the middle.
    let pre = a.iter().zip(b).take_while(|(x, y)| key(x) == key(y)).count();
    let (a2, b2) = (&a[pre..], &b[pre..]);
    let suf = a2.iter().rev().zip(b2.iter().rev()).take_while(|(x, y)| key(x) == key(y)).count();
    let (a2, b2) = (&a2[..a2.len() - suf], &b2[..b2.len() - suf]);
    if a2.is_empty() || b2.is_empty() {
        return vec![Splice { at: pre, remove: a2.len(), insert: b2.to_vec() }];
    }
    let (ka, kb): (Vec<K>, Vec<K>) = (a2.iter().map(&key).collect(), b2.iter().map(&key).collect());
    let mut out = vec![];
    for op in capture_diff_slices(Algorithm::Myers, &ka, &kb) {
        let (old_index, old_len, new_index, new_len) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete { old_index, old_len, new_index } => (old_index, old_len, new_index, 0),
            DiffOp::Insert { old_index, new_index, new_len } => (old_index, 0, new_index, new_len),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => (old_index, old_len, new_index, new_len),
        };
        out.push(Splice { at: pre + old_index, remove: old_len, insert: b2[new_index..new_index + new_len].to_vec() });
    }
    out
}

/// Do `splices` fit `list` (in order, inside it, not overlapping)? When
/// they don't, it isn't the list they were made from.
fn splices_fit<T>(list: &[T], splices: &[Splice<T>]) -> bool {
    let mut cur = 0;
    for s in splices {
        match s.at.checked_add(s.remove) {
            Some(end) if s.at >= cur && end <= list.len() => cur = end,
            _ => return false,
        }
    }
    true
}

/// Apply `splices` (which must fit, see [`splices_fit`]) to `list` in
/// place. Kept items are moved, not copied: undoing an edit in a
/// 200,000-element deck clones only what the splices insert.
fn splice_in_place<T: Clone>(list: &mut Vec<T>, splices: &[Splice<T>]) {
    if let [s] = splices {
        // The usual case: one run (an add, a delete, bring to front).
        list.splice(s.at..s.at + s.remove, s.insert.iter().cloned());
        return;
    }
    let grow: usize = splices.iter().map(|s| s.insert.len()).sum();
    let old = std::mem::take(list);
    let mut out = Vec::with_capacity(old.len() + grow);
    let mut it = old.into_iter();
    let mut cur = 0;
    for s in splices {
        out.extend(it.by_ref().take(s.at - cur));
        it.by_ref().take(s.remove).for_each(drop);
        out.extend(s.insert.iter().cloned());
        cur = s.at + s.remove;
    }
    out.extend(it);
    *list = out;
}

/// Apply a change made by [`diff`] to `doc`. On error `doc` is unchanged.
pub fn apply(doc: &mut Document, change: &Change) -> Result<(), String> {
    // Check that everything fits before anything is written.
    if !splices_fit(&doc.z_order, &change.z_order) {
        return Err("the draw order isn't the one this step was made from".into());
    }
    if !splices_fit(&doc.path, &change.path) {
        return Err("the path isn't the one this step was made from".into());
    }
    apply_patch(doc, &change.patch)?;
    if !change.z_order.is_empty() {
        splice_in_place(&mut doc.z_order, &change.z_order);
    }
    if !change.path.is_empty() {
        splice_in_place(&mut doc.path, &change.path);
    }
    Ok(())
}

fn apply_patch(doc: &mut Document, patch: &Patch) -> Result<(), String> {
    if patch.0.is_empty() {
        return Ok(());
    }
    // Each op's path, split: (field, element id, the rest).
    let mut split = Vec::with_capacity(patch.0.len());
    // Elements a patch replaces or changes the kind of; for the others only
    // the head (everything but `kind`) is patched.
    let mut whole = BTreeSet::new();
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
        let mut id = None;
        let mut rest: Vec<String> = vec![];
        if field == "elements" {
            if let Some(i) = t.next() {
                let i = i.decoded().into_owned();
                rest = t.map(|t| t.decoded().into_owned()).collect();
                if rest.first().is_none_or(|f| f == "kind") {
                    whole.insert(i.clone());
                }
                id = Some(i);
            }
        }
        split.push((field, id, rest));
    }

    // Head changes made by `head_ops` are applied straight to copies of
    // the heads (most of an undo of a big move); anything else goes
    // through a JSON view.
    let mut heads: BTreeMap<ElementId, Element> = BTreeMap::new();
    let mut by_json: BTreeSet<ElementId> = BTreeSet::new();
    for (op, (_, id, rest)) in patch.0.iter().zip(&split) {
        let Some(id) = id.as_ref().filter(|i| !whole.contains(*i) && !by_json.contains(*i)) else { continue };
        if !heads.contains_key(id) {
            let Some(e) = doc.elements.get(id) else { return Err(format!("elements: no element '{id}' to change")) };
            let head = Element {
                id: e.id.clone(),
                transform: e.transform,
                size: e.size,
                locked: e.locked,
                name: e.name.clone(),
                placeholder: e.placeholder,
                group: e.group.clone(),
                kind: crate::model::ElementKind::Frame(Default::default()),
            };
            heads.insert(id.clone(), head);
        }
        match head_op(heads.get_mut(id).expect("just added"), rest, op) {
            Some(r) => r?,
            None => {
                heads.remove(id);
                by_json.insert(id.clone());
            }
        }
    }
    let mut fields = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut json_ops = vec![];
    for (op, (field, id, _)) in patch.0.iter().zip(&split) {
        if id.as_ref().is_some_and(|i| heads.contains_key(i)) {
            continue;
        }
        if let Some(i) = id {
            ids.insert(i.clone());
        }
        fields.insert(field.clone());
        json_ops.push(op.clone());
    }
    let patch = &Patch(json_ops);

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
            "elements" => Value::Object(
                ids.iter().filter_map(|id| doc.elements.get(id).map(|e| (id.clone(), if whole.contains(id) { to_value(e) } else { head_value(e) }))).collect(),
            ),
            other => return Err(format!("unknown document field '{other}'")),
        };
        view.insert(f.clone(), v);
    }
    let mut view = Value::Object(view);
    if !patch.0.is_empty() {
        json_patch::patch(&mut view, patch).map_err(|e| e.to_string())?;
    }

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
    // Whole elements, and patched heads to put on the elements as they are.
    let mut elements: Option<BTreeMap<ElementId, Element>> = None;
    let mut json_heads: Vec<(ElementId, Head)> = vec![];
    if let Some(v) = view.remove("elements") {
        let Value::Object(m) = v else { return Err("elements: not an object".into()) };
        let mut els = BTreeMap::new();
        for (id, v) in m {
            if whole.contains(&id) {
                els.insert(id, serde_json::from_value(v).map_err(|e| format!("elements: {e}"))?);
            } else {
                if !doc.elements.contains_key(&id) {
                    return Err(format!("elements: no element '{id}' to change"));
                }
                json_heads.push((id, serde_json::from_value(v).map_err(|e| format!("elements: {e}"))?));
            }
        }
        elements = Some(els);
    }
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
    for (id, h) in json_heads {
        if let Some(e) = doc.elements.get_mut(&id) {
            set_head(e, h);
        }
    }
    for (id, h) in heads {
        if let Some(e) = doc.elements.get_mut(&id) {
            (e.id, e.transform, e.size, e.locked, e.name, e.placeholder, e.group) = (h.id, h.transform, h.size, h.locked, h.name, h.placeholder, h.group);
        }
    }
    if let Some(mut els) = elements {
        for id in whole {
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

    /// Any reordering, with additions and removals, goes there and back.
    #[test]
    fn list_edits_round_trip() {
        let a = deck();
        let mut seed = 7u64;
        let mut rnd = move |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % n.max(1)
        };
        for round in 0..200 {
            let mut b = a.clone();
            for _ in 0..(1 + round % 7) {
                match rnd(4) {
                    0 if !b.z_order.is_empty() => {
                        let id = b.z_order.remove(rnd(b.z_order.len()));
                        b.elements.remove(&id);
                    }
                    1 if !b.z_order.is_empty() => {
                        let id = b.z_order.remove(rnd(b.z_order.len()));
                        let at = rnd(b.z_order.len() + 1);
                        b.z_order.insert(at, id);
                    }
                    2 => b.z_order.reverse(),
                    _ => {
                        let i = rnd(b.z_order.len() + 1);
                        let target = crate::model::StepTarget::Overview;
                        b.path.insert(i.min(b.path.len()), PathStep { target, builds: vec![], notes: format!("n{round}-{i}"), duration: None, dwell: None, hide_builds: false, voiceover: None });
                    }
                }
            }
            let d = Delta::between(&a, &b);
            let mut c = a.clone();
            apply(&mut c, &d.forward).unwrap();
            assert_eq!(c, b, "forward, round {round}");
            apply(&mut c, &d.backward).unwrap();
            assert_eq!(c, a, "backward, round {round}");
        }
    }

    #[test]
    fn a_change_that_does_not_fit_leaves_the_document_alone() {
        let a = deck();
        let mut b = a.clone();
        b.z_order.truncate(10);
        b.title = "T".into();
        let d = Delta::between(&a, &b);
        // Applied to a document it wasn't made from.
        let mut other = Document::default();
        let before = other.clone();
        assert!(apply(&mut other, &d.forward).is_err());
        assert_eq!(other, before);
    }

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
        assert_eq!(p.op_count(), 2, "{p:?}");
        assert!(p.patch.0.is_empty());
        let mut c = a.clone();
        apply(&mut c, &p).unwrap();
        assert_eq!(c, b);
    }

    #[test]
    fn visual_changes_name_edited_added_removed_and_restacked_elements() {
        let a = deck();
        assert!(visual_changes(&a, &a.clone()).is_empty());
        let mut b = a.clone();
        b.elements.get_mut("e3").unwrap().transform.x = 5.0;
        let id = b.z_order.remove(10);
        b.z_order.push(id);
        b.z_order.retain(|i| i != "e20");
        b.elements.remove("e20");
        b.title = "not drawn".into();
        let mut v = visual_changes(&a, &b);
        v.sort();
        v.dedup();
        assert_eq!(v, vec!["e10".to_string(), "e20".into(), "e3".into()], "the restacked one, not the 39 it passed");
    }

    #[test]
    fn move_one_element_stores_only_its_coordinates() {
        let a = deck();
        let mut b = a.clone();
        b.elements.get_mut("e7").unwrap().transform.x = 42.0;
        let d = Delta::between(&a, &b);
        assert_eq!(d.forward.op_count(), 1);
        assert_eq!(d.forward.patch.0[0].to_string().contains("/elements/e7/transform/x"), true);
        let mut c = b.clone();
        apply(&mut c, &d.backward).unwrap();
        assert_eq!(c, a);
    }

    /// The head of an element is the element's JSON without `kind`, so a
    /// field added to `Element` and not to the head fails here.
    #[test]
    fn head_is_the_element_without_its_kind() {
        let mut e = deck().elements["e1"].clone();
        (e.locked, e.name, e.placeholder, e.group) = (true, "Intro".into(), true, Some("g1".into()));
        e.transform.rotation = 30.0;
        let mut whole = to_value(&e);
        whole.as_object_mut().unwrap().remove("kind");
        assert_eq!(head_value(&e), whole);
        let mut back = deck().elements["e2"].clone();
        set_head(&mut back, serde_json::from_value(head_value(&e)).unwrap());
        assert_eq!(Element { kind: e.kind.clone(), ..back }, e);
    }

    /// A move of an element with a long text stores and patches only its
    /// coordinates, and a change of kind still goes there and back.
    #[test]
    fn head_and_kind_changes_round_trip() {
        let a = deck();
        let mut b = a.clone();
        b.elements.get_mut("e1").unwrap().transform.x = 5.0;
        b.elements.get_mut("e2").unwrap().name = "Two".into();
        b.elements.get_mut("e3").unwrap().kind = crate::model::ElementKind::Frame(Default::default());
        b.elements.get_mut("e4").unwrap().group = Some("g".into());
        let d = Delta::between(&a, &b);
        assert!(d.forward.patch.0.iter().all(|op| !op.to_string().contains("/elements/e1/kind")), "{:?}", d.forward);
        let mut c = a.clone();
        apply(&mut c, &d.forward).unwrap();
        assert_eq!(c, b);
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
