//! Spatial index: which elements are in, under or around a rectangle.
//!
//! Answers "which frame is this in?", "what is inside this frame?", hit
//! tests, culling and where a new frame goes in the draw order without
//! walking every element. It is derived state: the [`Editor`] owns it,
//! keeps it in step with every change (each command, undo, redo, load) and
//! rebuilds it on load. It is never saved and never part of undo.
//!
//! Every answer is the same as the brute-force functions on
//! [`Document`] give (`parent_frame`, `frame_contents`, `hit_test`); the
//! index only narrows the candidates, and the same exact tests decide.
//! When the index can't be trusted (a host changed the document behind the
//! editor's back), the editor falls back to those functions.
//!
//! Structure: a hierarchical ("loose") grid. Each element sits in exactly
//! one cell, on the level whose cells are the smallest power of two at
//! least as big as its longer side, in the cell holding its centre, so it
//! lies within that cell grown by half a cell all round. A query visits,
//! on each level in use, the cells near the query rectangle (or every cell
//! of that level, when that is fewer). Chosen over an R-tree (`rstar`):
//! no new dependency, O(1) insert and remove per changed element (an edit
//! touches few, an R-tree rebalances), and decks zoom across 10^7 in
//! size, which one uniform grid can't serve but levels can.
//!
//! Draw order is kept as a rank per element (gapped, so inserting one
//! element between two others rarely renumbers), which sorts query
//! answers back to front without searching `z_order`.
use std::collections::{BTreeMap, HashMap};

use crate::editor::Editor;
use crate::geom::{Rect, Vec2};
use crate::model::{Document, Element, ElementId, ElementKind, FRAME_EDGE_TOLERANCE, GroupVerdicts, frame_holds_memo};

/// How far an element may draw past its box (strokes, arrowheads, a
/// connector's label), in world units. The renderer culls with it.
pub fn draw_overhang(e: &Element) -> f64 {
    let local = match &e.kind {
        ElementKind::Ink(i) => i.width,
        ElementKind::Shape(s) => s.stroke_width,
        ElementKind::Icon(i) => i.stroke_width * e.size.w.min(e.size.h) / 24.0,
        // Arrowheads and the label reach past the route's box.
        ElementKind::Connector(c) => (c.width * 4.0).max(10.0) + if c.label.is_empty() { 0.0 } else { 200.0 },
        _ => 0.0,
    };
    local * e.transform.scale
}

/// (level, column, row)
type Key = (i32, i64, i64);
const MIN_LEVEL: i32 = -60;
const MAX_LEVEL: i32 = 900;
/// Cell indices stay exactly representable as f64.
const MAX_CELL: f64 = (1u64 << 52) as f64;
/// Rank spacing after a renumber: room for 32 levels of halving.
const GAP: u64 = 1 << 32;
/// Rank of an element that isn't in `z_order` (never drawn, never found).
const NOT_DRAWN: u64 = u64::MAX;

fn cell_size(level: i32) -> f64 {
    (level as f64).exp2()
}

/// The cell for a rectangle, or `None` (always a candidate) when it isn't
/// finite.
fn key_of(r: &Rect) -> Option<Key> {
    let (w, h) = (r.x1 - r.x0, r.y1 - r.y0);
    let c = r.center();
    if !(w.is_finite() && h.is_finite() && c.x.is_finite() && c.y.is_finite()) {
        return None;
    }
    let m = w.max(h).max(0.0);
    let mut level = if m > 0.0 { (m.log2().ceil() as i32).clamp(MIN_LEVEL, MAX_LEVEL) } else { MIN_LEVEL };
    let far = c.x.abs().max(c.y.abs());
    while cell_size(level) < m || far / cell_size(level) > MAX_CELL {
        if level >= MAX_LEVEL {
            return None;
        }
        level += 1;
    }
    let s = cell_size(level);
    Some((level, (c.x / s).floor() as i64, (c.y / s).floor() as i64))
}

#[derive(Debug, Clone, Default)]
struct Grid {
    levels: BTreeMap<i32, HashMap<(i64, i64), Vec<u32>>>,
    /// Elements with no cell: offered to every query.
    wild: Vec<u32>,
}

impl Grid {
    fn insert(&mut self, k: Option<Key>, h: u32) {
        match k {
            Some((l, i, j)) => self.levels.entry(l).or_default().entry((i, j)).or_default().push(h),
            None => self.wild.push(h),
        }
    }

    fn remove(&mut self, k: Option<Key>, h: u32) {
        fn take(v: &mut Vec<u32>, h: u32) {
            if let Some(p) = v.iter().position(|&x| x == h) {
                v.swap_remove(p);
            }
        }
        match k {
            Some((l, i, j)) => {
                let Some(cells) = self.levels.get_mut(&l) else { return };
                if let Some(v) = cells.get_mut(&(i, j)) {
                    take(v, h);
                    if v.is_empty() {
                        cells.remove(&(i, j));
                    }
                }
                if cells.is_empty() {
                    self.levels.remove(&l);
                }
            }
            None => take(&mut self.wild, h),
        }
    }

    /// Every handle whose rectangle may meet `q` (a superset), skipping
    /// levels whose cells are smaller than `min_size` (nothing there is
    /// that big).
    fn query(&self, q: &Rect, min_size: f64, out: &mut Vec<u32>) {
        out.extend_from_slice(&self.wild);
        for (&l, cells) in &self.levels {
            let s = cell_size(l);
            if s < min_size {
                continue;
            }
            // A whole cell of margin: the element reaches half a cell past
            // its own, and the rest absorbs rounding.
            let (x0, x1) = (((q.x0 - s) / s).floor(), ((q.x1 + s) / s).floor());
            let (y0, y1) = (((q.y0 - s) / s).floor(), ((q.y1 + s) / s).floor());
            let span = (x1 - x0 + 1.0) * (y1 - y0 + 1.0);
            if span <= cells.len() as f64 && x0.abs() <= MAX_CELL && x1.abs() <= MAX_CELL && y0.abs() <= MAX_CELL && y1.abs() <= MAX_CELL {
                for i in x0 as i64..=x1 as i64 {
                    for j in y0 as i64..=y1 as i64 {
                        if let Some(v) = cells.get(&(i, j)) {
                            out.extend_from_slice(v);
                        }
                    }
                }
            } else {
                // Fewer cells in use than the query covers (or a query
                // that isn't finite): look at each.
                for (&(i, j), v) in cells {
                    let (i, j) = (i as f64, j as f64);
                    if i >= x0 && i <= x1 && j >= y0 && j <= y1 {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Slot {
    id: ElementId,
    /// `Element::bounds`.
    bounds: Rect,
    /// `Element::visual_bounds` for frames, else `bounds`.
    vbounds: Rect,
    /// `bounds` grown by [`draw_overhang`]; the grid key.
    reach: Rect,
    frame: bool,
    /// `Element::drawn_area` for frames (which of two is smaller).
    area: f64,
    group: Option<String>,
    rank: u64,
    key: Option<Key>,
    /// Key in the frame grid (frames only).
    fkey: Option<Key>,
}

fn geometry(e: &Element) -> (Rect, Rect, Rect) {
    let b = e.bounds();
    let vb = if e.is_frame() { e.visual_bounds() } else { b };
    let pad = draw_overhang(e);
    let reach = if pad > 0.0 { b.inflate(pad) } else { b };
    (b, vb, reach)
}

/// The index itself. See the module docs.
#[derive(Debug, Clone, Default)]
pub struct SpatialIndex {
    slots: Vec<Option<Slot>>,
    free: Vec<u32>,
    by_id: HashMap<ElementId, u32>,
    /// Every element, by `reach`.
    grid: Grid,
    /// Frames, by visual bounds.
    frames: Grid,
    /// Changed since the last [`SpatialIndex::sync`].
    dirty: Vec<ElementId>,
    all_dirty: bool,
    ranks_ok: bool,
    top_rank: u64,
    /// Bumped when ranks are renumbered or dropped, and on full rebuilds.
    rank_epoch: u64,
    full_epoch: u64,
    /// Ids touched since `begin_journal`, with the epochs then.
    journal: Option<(Vec<ElementId>, u64, u64)>,
    /// Every connector, so a change refreshes those and not every element.
    connectors: std::collections::BTreeSet<ElementId>,
    /// Each group's members: a frame holds a group whole or not at all.
    groups: HashMap<String, Vec<u32>>,
}

impl SpatialIndex {
    pub fn build(doc: &Document) -> SpatialIndex {
        let mut ix = SpatialIndex { slots: Vec::with_capacity(doc.elements.len()), ..Default::default() };
        ix.by_id.reserve(doc.elements.len());
        for id in doc.elements.keys() {
            ix.refresh(doc, id);
        }
        ix.renumber(doc);
        ix
    }

    fn rebuild(&mut self, doc: &Document) {
        let journal = self.journal.take();
        let (rank_epoch, full_epoch) = (self.rank_epoch, self.full_epoch);
        *self = SpatialIndex::build(doc);
        self.rank_epoch = rank_epoch + 1;
        self.full_epoch = full_epoch + 1;
        self.journal = journal;
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// `id` changed (added, moved, resized, restyled or removed); applied
    /// at the next [`SpatialIndex::sync`].
    pub fn touch(&mut self, id: &str) {
        self.dirty.push(id.to_string());
        if let Some((j, ..)) = &mut self.journal {
            j.push(id.to_string());
        }
    }

    /// Everything may have changed (a theme change remeasures all text).
    pub fn touch_all(&mut self) {
        self.all_dirty = true;
    }

    /// The draw order changed in a way other than an insert noted with
    /// [`SpatialIndex::z_inserted`] or a removal.
    pub fn z_changed(&mut self) {
        if self.ranks_ok {
            self.ranks_ok = false;
            self.rank_epoch += 1;
        }
    }

    /// `doc.z_order[pos]` was just inserted (element and draw order).
    /// Indexes it now and gives it a rank between its neighbours.
    pub fn z_inserted(&mut self, doc: &Document, pos: usize) {
        let Some(id) = doc.z_order.get(pos) else { return };
        let (was_ok, epoch) = (self.ranks_ok, self.rank_epoch);
        self.touch(id);
        self.refresh(doc, id);
        if !was_ok {
            return;
        }
        // `refresh` dropped the ranks for a newcomer; it gets one here.
        (self.ranks_ok, self.rank_epoch) = (true, epoch);
        let rank_of = |ix: &SpatialIndex, i: usize| ix.by_id.get(&doc.z_order[i]).and_then(|&h| ix.slots[h as usize].as_ref()).map(|s| s.rank).filter(|&r| r != NOT_DRAWN);
        let prev = if pos == 0 { Some(0) } else { rank_of(self, pos - 1) };
        let next = if pos + 1 == doc.z_order.len() { Some(self.top_rank.saturating_add(2 * GAP)) } else { rank_of(self, pos + 1) };
        let rank = match (prev, next) {
            (Some(p), Some(n)) if n > p && n - p >= 2 => p + (n - p).min(2 * GAP) / 2,
            _ => {
                self.z_changed();
                return;
            }
        };
        if rank >= NOT_DRAWN - 1 {
            self.z_changed();
            return;
        }
        let h = self.by_id[id];
        self.slots[h as usize].as_mut().expect("indexed").rank = rank;
        self.top_rank = self.top_rank.max(rank);
    }

    /// Apply every pending change.
    pub fn sync(&mut self, doc: &Document) {
        if self.all_dirty {
            self.rebuild(doc);
            return;
        }
        for id in std::mem::take(&mut self.dirty) {
            self.refresh(doc, &id);
        }
        if self.by_id.len() != doc.elements.len() {
            // Changed behind the editor's back.
            self.rebuild(doc);
            return;
        }
        if !self.ranks_ok {
            self.renumber(doc);
        }
    }

    /// Start recording touched ids, so [`SpatialIndex::rollback`] can undo
    /// a batch that failed.
    pub fn begin_journal(&mut self) {
        self.journal = Some((vec![], self.rank_epoch, self.full_epoch));
    }
    pub fn end_journal(&mut self) {
        self.journal = None;
    }
    /// The document went back to what it was at `begin_journal`.
    pub fn rollback(&mut self, doc: &Document) {
        let Some((ids, rank_epoch, full_epoch)) = self.journal.take() else {
            self.rebuild(doc);
            return;
        };
        if full_epoch != self.full_epoch {
            self.rebuild(doc);
            return;
        }
        self.dirty.extend(ids);
        if rank_epoch != self.rank_epoch {
            self.z_changed();
        }
        self.sync(doc);
    }

    /// Every connector, when the index can be trusted for `doc`.
    pub(crate) fn connectors(&self, doc: &Document) -> Option<&std::collections::BTreeSet<ElementId>> {
        self.usable(doc).then_some(&self.connectors)
    }

    /// Can answers be trusted for `doc`? Cheap; catches a host adding or
    /// removing elements without telling the editor.
    pub fn usable(&self, doc: &Document) -> bool {
        !self.all_dirty && self.dirty.is_empty() && self.ranks_ok && self.by_id.len() == doc.elements.len()
    }

    fn refresh(&mut self, doc: &Document, id: &str) {
        match doc.elements.get(id) {
            Some(e) if matches!(e.kind, ElementKind::Connector(_)) => {
                if !self.connectors.contains(id) {
                    self.connectors.insert(id.to_string());
                }
            }
            _ => {
                self.connectors.remove(id);
            }
        }
        let Some(e) = doc.elements.get(id) else {
            if let Some(h) = self.by_id.remove(id) {
                let s = self.slots[h as usize].take().expect("indexed");
                self.grid.remove(s.key, h);
                self.ungroup(s.group.as_deref(), h);
                if s.frame {
                    self.frames.remove(s.fkey, h);
                }
                self.free.push(h);
            }
            return;
        };
        let (bounds, vbounds, reach) = geometry(e);
        let frame = e.is_frame();
        let area = if frame { e.drawn_area() } else { 0.0 };
        let h = match self.by_id.get(id) {
            Some(&h) => {
                let s = self.slots[h as usize].as_mut().expect("indexed");
                if s.bounds == bounds && s.vbounds == vbounds && s.reach == reach && s.frame == frame && s.area.to_bits() == area.to_bits() && s.group == e.group {
                    return;
                }
                let (key, fkey, was_frame) = (s.key, s.fkey, s.frame);
                let old_group = s.group.take();
                self.ungroup(old_group.as_deref(), h);
                self.grid.remove(key, h);
                if was_frame {
                    self.frames.remove(fkey, h);
                }
                h
            }
            None => {
                let h = self.free.pop().unwrap_or_else(|| {
                    self.slots.push(None);
                    (self.slots.len() - 1) as u32
                });
                self.by_id.insert(id.to_string(), h);
                // Its place in the draw order isn't known here.
                self.slots[h as usize] = Some(Slot { id: id.to_string(), bounds, vbounds, reach, frame, area, group: None, rank: NOT_DRAWN, key: None, fkey: None });
                self.z_changed();
                h
            }
        };
        let key = key_of(&reach);
        let fkey = if frame { key_of(&vbounds) } else { None };
        let s = self.slots[h as usize].as_mut().expect("indexed");
        (s.bounds, s.vbounds, s.reach, s.frame, s.area, s.group, s.key, s.fkey) = (bounds, vbounds, reach, frame, area, e.group.clone(), key, fkey);
        if let Some(g) = &e.group {
            self.groups.entry(g.clone()).or_default().push(h);
        }
        self.grid.insert(key, h);
        if frame {
            self.frames.insert(fkey, h);
        }
    }

    fn ungroup(&mut self, g: Option<&str>, h: u32) {
        let Some(g) = g else { return };
        if let Some(v) = self.groups.get_mut(g) {
            v.retain(|&x| x != h);
            if v.is_empty() {
                self.groups.remove(g);
            }
        }
    }

    /// Ranks from `z_order` (spaced out, so later inserts fit between).
    fn renumber(&mut self, doc: &Document) {
        for s in self.slots.iter_mut().flatten() {
            s.rank = NOT_DRAWN;
        }
        let mut n = 0u64;
        for id in &doc.z_order {
            if let Some(&h) = self.by_id.get(id) {
                let s = self.slots[h as usize].as_mut().expect("indexed");
                // A repeated id keeps its first place, as `ordered` finds it first.
                if s.rank == NOT_DRAWN {
                    n += 1;
                    s.rank = n * GAP;
                }
            }
        }
        self.top_rank = n * GAP;
        self.ranks_ok = true;
        self.rank_epoch += 1;
    }

    fn slot(&self, h: u32) -> &Slot {
        self.slots[h as usize].as_ref().expect("indexed")
    }

    /// Drawn elements whose reach (box plus overhang) meets `q`, in no order.
    fn near<'a>(&'a self, q: &Rect) -> impl Iterator<Item = &'a Slot> + 'a {
        let q = *q;
        let mut hs = vec![];
        self.grid.query(&q, f64::NAN, &mut hs);
        hs.into_iter().map(|h| self.slot(h)).filter(move |s| s.rank != NOT_DRAWN && s.reach.intersects(&q))
    }

    /// Drawn frames whose visual bounds meet `q`, in no order, skipping
    /// any that can't be as wide or as tall as `min_size`.
    fn frames_near<'a>(&'a self, q: &Rect, min_size: f64) -> impl Iterator<Item = &'a Slot> + 'a {
        let q = *q;
        let mut hs = vec![];
        self.frames.query(&q, min_size, &mut hs);
        hs.into_iter().map(|h| self.slot(h)).filter(move |s| s.rank != NOT_DRAWN && s.vbounds.intersects(&q))
    }

    /// [`frame_holds`](crate::model::frame_holds), with group members
    /// from the index.
    /// `memo` lives for one query, so each group is judged once per frame.
    fn holds<'d>(&self, doc: &'d Document, f: &'d Element, e: &'d Element, memo: &mut GroupVerdicts<'d>) -> bool {
        frame_holds_memo(f, e, |g| self.groups.get(g).into_iter().flatten().filter_map(|&h| doc.elements.get(&self.slot(h).id)), memo)
    }

    /// Smallest frame (then lowest in the draw order) that holds `e`
    /// ([`frame_holds`](crate::model::frame_holds)). Same answer as
    /// [`Document::parent_frame`].
    fn parent_of<'d>(&'d self, doc: &'d Document, e: &'d Element, memo: &mut GroupVerdicts<'d>) -> Option<&'d Slot> {
        // A frame holding `e` holds its points, so its box is at least as
        // wide and as tall as theirs, less the edge tolerance (a few parts
        // in 10^9 of its size).
        let b = Rect::from_points(e.held_points())?;
        let min = b.width().max(b.height()) * (1.0 - 1e3 * FRAME_EDGE_TOLERANCE);
        self.frames_near(&b, min)
            .filter(|f| f.id != e.id && doc.elements.get(&f.id).is_some_and(|fe| self.holds(doc, fe, e, memo)))
            .min_by(|x, y| x.area.total_cmp(&y.area).then(x.rank.cmp(&y.rank)))
    }

    /// Panics unless the index matches a fresh build for `doc` (tests and
    /// debug builds).
    pub fn assert_matches(&self, doc: &Document) {
        assert!(self.usable(doc), "spatial index has pending changes or is missing elements");
        let fresh = SpatialIndex::build(doc);
        assert_eq!(self.connectors, fresh.connectors, "spatial index's connectors");
        let members = |ix: &SpatialIndex| -> BTreeMap<String, std::collections::BTreeSet<String>> {
            ix.groups.iter().map(|(g, v)| (g.clone(), v.iter().map(|&h| ix.slot(h).id.clone()).collect())).collect()
        };
        assert_eq!(members(self), members(&fresh), "spatial index's groups");
        for (id, &h) in &fresh.by_id {
            let a = fresh.slot(h);
            let Some(&mine) = self.by_id.get(id) else { panic!("spatial index lacks {id}") };
            let b = self.slot(mine);
            let same = |x: &Rect, y: &Rect| x == y || (format!("{x:?}") == format!("{y:?}"));
            assert!(same(&a.bounds, &b.bounds) && same(&a.vbounds, &b.vbounds) && same(&a.reach, &b.reach) && a.frame == b.frame && a.area.to_bits() == b.area.to_bits() && a.group == b.group, "spatial index is stale for {id}: {b:?} vs {a:?}");
            assert_eq!(a.rank == NOT_DRAWN, b.rank == NOT_DRAWN, "draw state of {id}");
        }
        // Ranks rise along the draw order.
        let mut last = 0;
        for id in &doc.z_order {
            let r = self.slot(self.by_id[id]).rank;
            if r == NOT_DRAWN {
                continue;
            }
            assert!(r > last, "ranks out of draw order at {id}");
            last = r;
        }
    }
}

impl Editor {
    fn spatial(&self) -> Option<&SpatialIndex> {
        self.index.usable(&self.doc).then_some(&self.index)
    }

    /// Rebuild the spatial index. Only for a host that changed `doc`
    /// directly instead of through [`Editor::apply`].
    pub fn reindex(&mut self) {
        self.index = SpatialIndex::build(&self.doc);
    }

    /// [`Document::frame_contents`], from the spatial index.
    pub fn frame_contents(&self, frame_id: &str) -> Vec<ElementId> {
        let Some(ix) = self.spatial() else { return self.doc.frame_contents(frame_id) };
        let Some(f) = self.doc.elements.get(frame_id) else { return vec![] };
        let fb = f.visual_bounds();
        let mut memo = GroupVerdicts::default();
        let mut v: Vec<(u64, &str)> = ix
            .near(&fb)
            .filter(|s| s.id != f.id && self.doc.elements.get(&s.id).is_some_and(|e| ix.holds(&self.doc, f, e, &mut memo)))
            .map(|s| (s.rank, s.id.as_str()))
            .collect();
        v.sort_unstable_by_key(|x| x.0);
        v.into_iter().map(|(_, id)| id.to_string()).collect()
    }

    /// [`Document::parent_frame`], from the spatial index.
    pub fn parent_frame(&self, id: &str) -> Option<ElementId> {
        let Some(ix) = self.spatial() else { return self.doc.parent_frame(id) };
        let e = self.doc.elements.get(id)?;
        ix.parent_of(&self.doc, e, &mut GroupVerdicts::default()).map(|f| f.id.clone())
    }

    /// Every element's parent frame at once (elements in no frame are
    /// left out): for queries that need them all.
    pub fn parent_frames(&self) -> HashMap<&str, &str> {
        if let Some(ix) = self.spatial() {
            let mut memo = GroupVerdicts::default();
            return self.doc.elements.values().filter_map(|e| Some((e.id.as_str(), ix.parent_of(&self.doc, e, &mut memo)?.id.as_str()))).collect();
        }
        // Frames smallest first (then back to front): the first that holds
        // an element is its parent.
        let mut frames: Vec<(f64, usize, &Element)> = self.doc.ordered().enumerate().filter(|(_, f)| f.is_frame()).map(|(z, f)| (f.drawn_area(), z, f)).collect();
        frames.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let groups = self.doc.group_members();
        let mut memo = GroupVerdicts::default();
        self.doc
            .elements
            .values()
            .filter_map(|e| {
                let f = frames.iter().find(|(_, _, f)| frame_holds_memo(f, e, |g| groups.get(g).into_iter().flatten().copied(), &mut memo))?;
                Some((e.id.as_str(), f.2.id.as_str()))
            })
            .collect()
    }

    /// Elements that may be under a world point, back to front: every one
    /// [`Element::hit`] could accept with this tolerance, and maybe more.
    pub fn elements_at(&self, p: Vec2, tolerance_world: f64) -> Vec<&Element> {
        // A rotated box grown by `tol` along its own axes stays within its
        // axis-aligned bounds grown by tol * sqrt(2).
        let t = tolerance_world.abs() * 1.5;
        self.elements_in(Rect { x0: p.x - t, y0: p.y - t, x1: p.x + t, y1: p.y + t })
    }

    /// Elements that may show in `r`, back to front: every one whose
    /// bounds, grown by what it draws past them ([`draw_overhang`]), meet
    /// `r`, and maybe more. Callers make their exact test.
    pub fn elements_in(&self, r: Rect) -> Vec<&Element> {
        let Some(ix) = self.spatial() else { return self.doc.ordered().collect() };
        let mut v: Vec<(u64, &str)> = ix.near(&r).map(|s| (s.rank, s.id.as_str())).collect();
        if v.len() * 2 > ix.len() {
            // Most of the deck: walking the draw order is as quick.
            return self.doc.ordered().collect();
        }
        v.sort_unstable_by_key(|x| x.0);
        v.into_iter().filter_map(|(_, id)| self.doc.elements.get(id)).collect()
    }

    /// [`Element::own_label`], with a frame's contents from the index.
    pub fn own_label(&self, e: &Element) -> Option<String> {
        e.own_label_in(&self.doc, || self.frame_contents(&e.id))
    }

    /// [`Element::label`], with a frame's contents from the index.
    pub fn label(&self, e: &Element) -> String {
        self.own_label(e).unwrap_or_else(|| e.kind.type_name().into())
    }

    /// [`Document::hit_test`], from the spatial index.
    pub fn hit_test(&self, p: Vec2, tolerance_world: f64) -> Option<&Element> {
        self.elements_at(p, tolerance_world).into_iter().rev().find(|e| e.hit(p, tolerance_world, false))
    }

    /// Where a new frame `f` goes in the draw order: behind the lowest
    /// element it will hold ([`frame_holds`](crate::model::frame_holds)),
    /// else on top.
    pub(crate) fn frame_insert_pos(&self, f: &Element) -> usize {
        let len = self.doc.z_order.len();
        let Some(ix) = self.spatial() else {
            let groups = self.doc.group_members();
            let mut memo = GroupVerdicts::default();
            return self
                .doc
                .z_order
                .iter()
                .position(|o| self.doc.get(o).is_some_and(|x| frame_holds_memo(f, x, |g| groups.get(g).into_iter().flatten().copied(), &mut memo)))
                .unwrap_or(len);
        };
        let mut memo = GroupVerdicts::default();
        let Some(lowest) = ix.near(&f.visual_bounds()).filter(|s| self.doc.elements.get(&s.id).is_some_and(|x| ix.holds(&self.doc, f, x, &mut memo))).map(|s| s.rank).min() else {
            return len;
        };
        // z_order rises in rank: binary search for it.
        let rank = |id: &ElementId| ix.by_id.get(id).map(|&h| ix.slot(h).rank).unwrap_or(NOT_DRAWN);
        let z = &self.doc.z_order;
        let pos = z.partition_point(|id| rank(id) < lowest);
        if z.get(pos).is_some_and(|id| rank(id) == lowest) {
            pos
        } else {
            // Not where ranks say (shouldn't happen): look for it.
            z.iter().position(|id| rank(id) == lowest).unwrap_or(len)
        }
    }
}
