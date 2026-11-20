//! What a batch of commands (or a person's drag) changed, with the old
//! values: enough to record the change as an undo step, check it, or put
//! the document back, without copying the whole document first.
//!
//! The editor used to clone the document before every command, compare
//! the whole of it afterwards and diff every element to find what changed.
//! In a 200,000-element deck that was hundreds of milliseconds per edit.
//! Now every change to an element, the draw order or the path goes
//! through the editor's journaled accessors, which keep the old value the
//! first time something is touched. The other fields are small and are
//! copied when the journal starts.
//!
//! Debug builds keep a full copy as well (for decks of up to
//! [`CHECK_LIMIT`] elements) and check that the journal's change is
//! exactly the whole-document one, so a change that bypasses the journal
//! fails the tests instead of corrupting undo.
use std::collections::BTreeMap;

use crate::delta::{Before, Delta};
use crate::model::{Document, Element, ElementId, PathStep};

/// Largest deck the debug check copies.
pub(crate) const CHECK_LIMIT: usize = 5000;

#[derive(Debug, Clone)]
pub(crate) struct Journal {
    /// The document's own fields as they were. Its `elements`, `z_order`
    /// and `path` are empty; `assets` is kept so a rollback drops assets
    /// a failed batch added.
    meta: Document,
    /// The draw order as it was, once something changed it.
    z_order: Option<Vec<ElementId>>,
    /// The path as it was, once something changed it.
    path: Option<Vec<PathStep>>,
    /// Each element touched, as it was (`None`: it didn't exist).
    elements: BTreeMap<ElementId, Option<Element>>,
    /// Debug builds: the whole document as it was (see the module docs).
    full: Option<Box<Document>>,
}

/// `doc` without its elements, draw order and path (all small fields).
fn meta_of(doc: &Document) -> Document {
    Document {
        format_version: doc.format_version,
        title: doc.title.clone(),
        theme: doc.theme.clone(),
        elements: Default::default(),
        z_order: vec![],
        path: vec![],
        assets: doc.assets.clone(),
        next_id: doc.next_id,
        themes: doc.themes.clone(),
        play: doc.play,
        soundtrack: doc.soundtrack.clone(),
    }
}

/// Same small fields? Assets are only ever added, so their keys are enough.
fn meta_eq(a: &Document, b: &Document) -> bool {
    a.format_version == b.format_version
        && a.title == b.title
        && a.theme == b.theme
        && a.next_id == b.next_id
        && a.themes == b.themes
        && a.play == b.play
        && a.soundtrack == b.soundtrack
        && a.assets.len() == b.assets.len()
        && a.assets.keys().eq(b.assets.keys())
}

impl Journal {
    pub(crate) fn start(doc: &Document) -> Journal {
        let full = (cfg!(debug_assertions) && doc.elements.len() <= CHECK_LIMIT).then(|| Box::new(doc.clone()));
        Journal { meta: meta_of(doc), z_order: None, path: None, elements: BTreeMap::new(), full }
    }

    /// `id` is about to change, be added or be removed.
    pub(crate) fn element(&mut self, doc: &Document, id: &str) {
        if !self.elements.contains_key(id) {
            self.elements.insert(id.to_string(), doc.elements.get(id).cloned());
        }
    }

    /// `old` (stored under `id`) is about to change.
    pub(crate) fn element_value(&mut self, id: &ElementId, old: &Element) {
        if !self.elements.contains_key(id) {
            self.elements.insert(id.clone(), Some(old.clone()));
        }
    }

    /// Every element may be about to change (a new theme remeasures all
    /// text, a template replaces the canvas).
    pub(crate) fn all_elements(&mut self, doc: &Document) {
        for (id, e) in &doc.elements {
            self.element_value(id, e);
        }
    }

    /// The draw order is about to change.
    pub(crate) fn z_order(&mut self, doc: &Document) {
        if self.z_order.is_none() {
            self.z_order = Some(doc.z_order.clone());
        }
    }

    /// The path is about to change.
    pub(crate) fn path(&mut self, doc: &Document) {
        if self.path.is_none() {
            self.path = Some(doc.path.clone());
        }
    }

    /// Element `id` as it was before the batch, if it was touched and existed.
    pub(crate) fn before(&self, id: &str) -> Option<&Element> {
        self.elements.get(id).and_then(Option::as_ref)
    }

    /// Did the draw order change?
    pub(crate) fn z_touched(&self) -> bool {
        self.z_order.is_some()
    }

    /// Is `doc` still as it was?
    pub(crate) fn unchanged(&self, doc: &Document) -> bool {
        let same = meta_eq(&self.meta, doc)
            && self.z_order.as_ref().is_none_or(|z| *z == doc.z_order)
            && self.path.as_ref().is_none_or(|p| *p == doc.path)
            && self.elements.iter().all(|(id, old)| old.as_ref() == doc.elements.get(id));
        if let Some(full) = &self.full {
            assert_eq!(same, **full == *doc, "journal: a change bypassed the journal (or was missed)");
        }
        same
    }

    /// Elements that differ from before (touched and not back as they were).
    pub(crate) fn changed<'a>(&'a self, doc: &'a Document) -> impl Iterator<Item = (&'a ElementId, Option<&'a Element>)> + 'a {
        self.elements.iter().filter(move |(id, old)| old.as_ref() != doc.elements.get(*id)).map(|(id, old)| (id, old.as_ref()))
    }

    /// Did the path, autoplay or soundtrack change?
    pub(crate) fn steps_changed(&self, doc: &Document) -> bool {
        self.path.as_ref().is_some_and(|p| *p != doc.path) || self.meta.play != doc.play || self.meta.soundtrack != doc.soundtrack
    }

    /// The undo step for everything since `start`.
    pub(crate) fn delta(&self, doc: &Document) -> Delta {
        let before = Before { meta: &self.meta, elements: &self.elements, z_order: self.z_order.as_deref(), path: self.path.as_deref() };
        let d = Delta::from_before(&before, doc);
        if let Some(full) = &self.full {
            assert!(d == Delta::between(full, doc), "journal: a change bypassed the journal (or was missed)");
        }
        d
    }

    /// Put `doc` back as it was at `start`. Returns the ids of elements
    /// that were touched (for the spatial index).
    pub(crate) fn restore(self, doc: &mut Document) -> Vec<ElementId> {
        let Journal { meta, z_order, path, elements, full } = self;
        let Document { format_version, title, theme, assets, next_id, themes, play, soundtrack, .. } = meta;
        (doc.format_version, doc.title, doc.theme, doc.assets, doc.next_id, doc.themes, doc.play, doc.soundtrack) =
            (format_version, title, theme, assets, next_id, themes, play, soundtrack);
        if let Some(z) = z_order {
            doc.z_order = z;
        }
        if let Some(p) = path {
            doc.path = p;
        }
        let mut ids = Vec::with_capacity(elements.len());
        for (id, old) in elements {
            match old {
                Some(e) => doc.elements.insert(id.clone(), e),
                None => doc.elements.remove(&id),
            };
            ids.push(id);
        }
        if let Some(full) = full {
            assert!(*full == *doc, "journal: a change bypassed the journal (or was missed)");
        }
        ids
    }

    /// Fold `inner` (a later journal of the same document) into this one:
    /// what this one already holds is older and wins.
    pub(crate) fn absorb(&mut self, inner: Journal) {
        if self.z_order.is_none() {
            self.z_order = inner.z_order;
        }
        if self.path.is_none() {
            self.path = inner.path;
        }
        for (id, old) in inner.elements {
            self.elements.entry(id).or_insert(old);
        }
    }
}
