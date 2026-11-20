//! Objects copied out of one document to go into another (or the same one):
//! the elements plus the pictures, videos and posters they show, so a copy
//! in a document that never had those bytes still draws.
use std::collections::{BTreeMap, BTreeSet};

use crate::command::{Command, NewElement};
use crate::editor::{Editor, Outcome};
use crate::error::{ZError, ZResult};
use crate::geom::Rect;
use crate::model::{Asset, Element, ElementId, ElementKind};
use crate::Origin;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clip {
    /// Back to front.
    pub elements: Vec<Element>,
    pub assets: BTreeMap<String, Asset>,
}

impl Clip {
    /// `ids` with their groups and, for frames, what's inside them.
    pub fn of(editor: &Editor, ids: &[ElementId]) -> Clip {
        let doc = &editor.doc;
        let wanted: BTreeSet<ElementId> = editor.expand(&doc.expand_groups(ids), true).into_iter().collect();
        let elements: Vec<Element> = doc.z_order.iter().filter(|id| wanted.contains(*id)).filter_map(|id| doc.get(id).cloned()).collect();
        let mut assets = BTreeMap::new();
        for e in &elements {
            for key in asset_keys(e) {
                if let Some(a) = doc.assets.get(key) {
                    assets.insert(key.to_string(), a.clone());
                }
            }
        }
        Clip { elements, assets }
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn bounds(&self) -> Option<Rect> {
        self.elements.iter().map(|e| e.bounds()).reduce(|a, b| a.union(&b))
    }
}

fn asset_keys(e: &Element) -> Vec<&str> {
    match &e.kind {
        ElementKind::Image(i) => std::iter::once(i.asset.as_str()).chain(i.page.as_ref().map(|p| p.pdf.as_str())).collect(),
        ElementKind::Video(v) => std::iter::once(v.asset.as_str()).chain(v.poster.as_deref()).collect(),
        _ => vec![],
    }
}

impl Editor {
    /// Add a copy of `clip`, moved by (`dx`, `dy`), as one undo step. The
    /// copies get fresh ids; a connector stays stuck to an end that was
    /// copied with it and comes loose from one that wasn't; groups stay
    /// grouped.
    pub fn paste_clip(&mut self, clip: &Clip, dx: f64, dy: f64, label: Option<String>, origin: Origin) -> ZResult<Outcome> {
        self.paste_clip_then(clip, dx, dy, label, origin, |_| vec![])
    }

    /// [`Editor::paste_clip`], with more commands in the same undo step:
    /// `then` gets the ids the copies were given (by their id in `clip`)
    /// and returns commands that run after the copies are in.
    pub fn paste_clip_then(
        &mut self,
        clip: &Clip,
        dx: f64,
        dy: f64,
        label: Option<String>,
        origin: Origin,
        then: impl FnOnce(&BTreeMap<&str, ElementId>) -> Vec<Command>,
    ) -> ZResult<Outcome> {
        if clip.is_empty() {
            return Err(ZError::invalid("nothing to paste"));
        }
        // Fresh ids, decided up front so connectors can point at the copies.
        let mut taken: BTreeSet<ElementId> = self.doc.elements.keys().cloned().collect();
        let mut n = self.doc.next_id;
        let mut new_id: BTreeMap<&str, ElementId> = BTreeMap::new();
        for e in &clip.elements {
            let id = loop {
                let id = format!("{}{n}", e.kind.id_prefix());
                n = n.wrapping_add(1);
                if !taken.contains(&id) {
                    break id;
                }
            };
            taken.insert(id.clone());
            new_id.insert(e.id.as_str(), id);
        }
        let mut cmds = vec![];
        for e in &clip.elements {
            let mut kind = e.kind.clone();
            if let ElementKind::Connector(c) = &mut kind {
                for end in [&mut c.from, &mut c.to] {
                    end.element = end.element.as_deref().and_then(|id| new_id.get(id).cloned());
                }
            }
            // Text that sizes itself keeps doing so.
            let keep_size = !matches!(&e.kind, ElementKind::Text(t) if !t.fixed_width);
            cmds.push(Command::AddElement {
                element: NewElement {
                    id: Some(new_id[e.id.as_str()].clone()),
                    x: e.transform.x + dx,
                    y: e.transform.y + dy,
                    scale: e.transform.scale,
                    rotation: e.transform.rotation,
                    w: keep_size.then_some(e.size.w),
                    h: keep_size.then_some(e.size.h),
                    name: e.name.clone(),
                    kind,
                },
            });
        }
        let mut groups: BTreeMap<&str, Vec<ElementId>> = BTreeMap::new();
        for e in &clip.elements {
            if let Some(g) = &e.group {
                groups.entry(g.as_str()).or_default().push(new_id[e.id.as_str()].clone());
            }
        }
        for ids in groups.into_values().filter(|v| v.len() > 1) {
            cmds.push(Command::Group { ids, id: None });
        }
        cmds.extend(then(&new_id));
        // The bytes come first: an image can't point at a picture the
        // document doesn't have. Content-addressed, so a key already here
        // holds the same bytes.
        let added: Vec<String> = clip.assets.keys().filter(|k| !self.doc.assets.contains_key(*k)).cloned().collect();
        for k in &added {
            self.doc.assets.insert(k.clone(), clip.assets[k].clone());
        }
        let r = self.apply_batch(cmds, label, origin);
        if r.is_err() {
            for k in &added {
                self.doc.assets.remove(k);
            }
        }
        let mut out = r?;
        // Only the copies, not the elements a group command touched again.
        out.created.retain(|id| new_id.values().any(|n| n == id));
        out.changed.retain(|id| !out.created.contains(id));
        Ok(out)
    }
}
