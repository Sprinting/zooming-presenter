//! Path sidebar thumbnails, drawn off the UI thread with the CPU renderer.
//!
//! The thread keeps the document it drew from last and, per step, how the
//! step was framed and its picture. After an edit (debounced, so typing
//! never waits) it compares the new document with that one and redraws a
//! step only when its framing changed or an element that changed shows in
//! it, where it was before the change or where it is now; a theme change
//! redraws them all. It draws only the steps the sidebar shows (and a few
//! either side), the others when they scroll into view, and lets go of
//! pictures far from view.
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{ColorImage, TextureHandle, TextureOptions};
use zebra_core::{Document, Editor, Element, ElementId, Rect, Vec2};
use zebra_render::Renderer;

pub const THUMB_W: u32 = 288;
pub const THUMB_H: u32 = 180;
/// Steps drawn either side of those on screen, so a short scroll finds
/// them ready.
const DRAW_MARGIN: usize = 4;
/// Pictures kept either side of the steps on screen. Farther ones are let
/// go (288x180 is 200 KB each) and drawn again when scrolled back to.
const KEEP_MARGIN: usize = 40;
/// More changed elements than this are taken as one region, their union.
const MAX_REGIONS: usize = 2048;
/// While drawing, what is ready is sent at most this often.
const REPORT_EVERY: Duration = Duration::from_millis(100);

enum Msg {
    /// The document at revision `rev`.
    Doc { epoch: u64, rev: u64, doc: Document },
    /// The steps on screen.
    Show(Range<usize>),
    /// Another document was loaded (or the path emptied): forget all.
    Clear { epoch: u64 },
}

/// One step's picture, as the thread last reported it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub id: u64,
    /// Drawn from the newest document the thread has. Else an older
    /// picture, shown until the step is redrawn.
    pub fresh: bool,
}

struct Report {
    epoch: u64,
    rev: u64,
    slots: Vec<Option<Slot>>,
    images: Vec<(u64, ColorImage)>,
}

/// What the thread knows about one step.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    /// How the step is framed: camera and viewport, bit for bit.
    frame: [u64; 6],
    /// The world it shows.
    view: Rect,
    /// The builds hidden at this step that would show in `view` (hashed).
    hidden: u64,
    pic: Option<u64>,
    fresh: bool,
    /// Drawing it failed; tried again after the next change.
    failed: bool,
}

/// Where an element may draw: its bounds and what it draws past them.
fn reach(e: &Element) -> Rect {
    e.bounds().inflate(zebra_core::spatial::draw_overhang(e))
}

/// Each step's framing in `ed`, with no picture yet.
fn framing(ed: &Editor) -> Vec<Entry> {
    let n = ed.doc.path.len();
    let mut out = Vec::with_capacity(n);
    // Builds of the steps after the one being framed: hidden at it.
    let mut later: Vec<&ElementId> = vec![];
    for i in (0..n).rev() {
        let (cam, vp) = zebra_render::cpu::step_view(ed, i, THUMB_W, THUMB_H).expect("the step exists");
        let view = cam.visible_bounds(Vec2::new(vp[0], vp[1]));
        let mut shown: Vec<&str> =
            later.iter().filter(|id| ed.doc.get(id).is_some_and(|e| reach(e).intersects(&view))).map(|id| id.as_str()).collect();
        let hidden = if shown.is_empty() {
            0
        } else {
            shown.sort_unstable();
            shown.dedup();
            let mut h = std::collections::hash_map::DefaultHasher::new();
            shown.hash(&mut h);
            h.finish() | 1
        };
        let frame = [cam.cx, cam.cy, cam.zoom, cam.rotation, vp[0], vp[1]].map(f64::to_bits);
        out.push(Entry { frame, view, hidden, pic: None, fresh: false, failed: false });
        later.extend(ed.doc.path[i].builds.iter());
    }
    out.reverse();
    out
}

/// Where the document changed between `old` and `new`: each changed
/// element's reach before and after.
fn changed_regions(old: &Editor, new: &Editor) -> Vec<Rect> {
    let ids = zebra_core::delta::visual_changes(&old.doc, &new.doc);
    let rs: Vec<Rect> = ids.iter().flat_map(|id| [old.doc.get(id), new.doc.get(id)]).flatten().map(reach).collect();
    if rs.len() > MAX_REGIONS {
        return rs.into_iter().reduce(|a, b| a.union(&b)).into_iter().collect();
    }
    rs
}

/// The steps of `new`, each keeping the picture drawn for `old` when it
/// still shows the same thing: a step framed the same way (wherever it is
/// in the path now) whose view no change reaches and whose hidden builds
/// are the same. Every other step is to be redrawn; until then it shows
/// the picture it had (stale). A theme change redraws every step.
fn carry_over(old: Option<&Editor>, old_entries: &[Entry], new: &Editor) -> Vec<Entry> {
    let mut entries = framing(new);
    let Some(old) = old else { return entries };
    let all = old.theme() != new.theme();
    let regions = if all { vec![] } else { changed_regions(old, new) };
    let mut by_frame: HashMap<[u64; 6], &Entry> = HashMap::new();
    for e in old_entries {
        // Two steps framed alike: prefer the one with a current picture.
        let better = by_frame.get(&e.frame).is_none_or(|p| (e.fresh && e.pic.is_some()) > (p.fresh && p.pic.is_some()));
        if better {
            by_frame.insert(e.frame, e);
        }
    }
    for (i, e) in entries.iter_mut().enumerate() {
        match by_frame.get(&e.frame) {
            Some(p) => {
                let same = !all && !p.failed && p.hidden == e.hidden && !regions.iter().any(|r| r.intersects(&e.view));
                e.pic = p.pic;
                e.fresh = same && p.fresh;
            }
            // Framed anew (its frame moved, say): the old picture in that
            // place stands in until it is redrawn.
            None => e.pic = old_entries.get(i).and_then(|o| o.pic),
        }
    }
    entries
}

/// The step whose drawing fails, in tests (of a document with this title,
/// so tests running alongside are not hit).
#[cfg(test)]
const FAIL_TITLE: &str = "test: a step fails";
#[cfg(test)]
static FAIL_STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

/// Draw step `i`, as raw pixels (no PNG on the way).
fn render(i: usize, ed: &Editor, renderer: &Renderer) -> Option<ColorImage> {
    #[cfg(test)]
    if FAIL_STEP.load(std::sync::atomic::Ordering::Relaxed) == i && ed.doc.title.starts_with(FAIL_TITLE) {
        panic!("test: thumbnail {i}");
    }
    let target = zebra_render::CaptureTarget::Step { index: i, build: None };
    let list = renderer.capture_display_list(ed, &target, THUMB_W, THUMB_H).ok()?;
    let rgba = zebra_render::cpu::render_rgba(&list);
    Some(ColorImage::from_rgba_unmultiplied([list.width_px as usize, list.height_px as usize], &rgba))
}

/// The thumbnail thread's state.
struct Worker {
    renderer: Arc<Renderer>,
    ed: Option<Editor>,
    epoch: u64,
    rev: u64,
    entries: Vec<Entry>,
    show: Range<usize>,
    next_id: u64,
}

impl Worker {
    fn take(&mut self, m: Msg) {
        match m {
            Msg::Show(r) => self.show = r,
            Msg::Clear { epoch } => {
                self.epoch = epoch;
                self.ed = None;
                self.entries.clear();
            }
            Msg::Doc { epoch, rev, doc } => {
                if epoch != self.epoch {
                    self.take(Msg::Clear { epoch });
                }
                // The document is already measured and repaired: no
                // `load`, which would measure every text again.
                let mut ed = Editor::new(doc);
                ed.measure = self.renderer.measure();
                ed.view.viewport = Vec2::new(THUMB_W as f64, THUMB_H as f64);
                self.entries = carry_over(self.ed.as_ref(), &self.entries, &ed);
                self.ed = Some(ed);
                self.rev = rev;
            }
        }
    }

    /// The next step to draw: those on screen top down, then the margin.
    fn next(&self) -> Option<usize> {
        let n = self.entries.len();
        let on = self.show.start.min(n)..self.show.end.min(n);
        if on.is_empty() {
            // The sidebar shows no steps (yet).
            return None;
        }
        let (lo, hi) = (on.start.saturating_sub(DRAW_MARGIN), (on.end + DRAW_MARGIN).min(n));
        on.clone().chain(lo..on.start).chain(on.end..hi).find(|&i| !self.entries[i].fresh)
    }

    fn draw(&mut self, i: usize) -> Option<(u64, ColorImage)> {
        let ed = self.ed.as_ref()?;
        // One step that can't be drawn (a panic in the renderer) leaves
        // that thumbnail empty; it used to end this thread, and no
        // thumbnail was ever drawn again.
        let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(i, ed, &self.renderer)));
        let e = &mut self.entries[i];
        e.fresh = true;
        match drawn {
            Ok(Some(img)) => {
                self.next_id += 1;
                e.pic = Some(self.next_id);
                e.failed = false;
                Some((self.next_id, img))
            }
            other => {
                if other.is_err() {
                    eprintln!("zebra: the thumbnail for step {} could not be drawn", i + 1);
                }
                e.pic = None;
                e.failed = true;
                None
            }
        }
    }

    /// Let go of pictures far from the steps on screen. True if any went.
    fn evict(&mut self) -> bool {
        let keep = self.show.start.saturating_sub(KEEP_MARGIN)..self.show.end.saturating_add(KEEP_MARGIN);
        let mut any = false;
        for (i, e) in self.entries.iter_mut().enumerate() {
            if !keep.contains(&i) && e.pic.is_some() {
                e.pic = None;
                e.fresh = false;
                any = true;
            }
        }
        any
    }

    fn slots(&self) -> Vec<Option<Slot>> {
        self.entries.iter().map(|e| e.pic.map(|id| Slot { id, fresh: e.fresh })).collect()
    }

    fn run(mut self, rx: Receiver<Msg>, tx: Sender<Report>, wake: egui::Context) {
        let mut images = vec![];
        let mut unsent = false;
        let mut sent_at = Instant::now();
        loop {
            // Nothing to draw or send: sleep until told something.
            let mut msgs: Vec<Msg> = vec![];
            if self.next().is_none() && !unsent {
                match rx.recv() {
                    Ok(m) => msgs.push(m),
                    Err(_) => return,
                }
            }
            msgs.extend(rx.try_iter());
            // Only the newest document is worth comparing with.
            let newest = msgs.iter().rposition(|m| matches!(m, Msg::Doc { .. }));
            for (k, m) in msgs.into_iter().enumerate() {
                if matches!(m, Msg::Doc { .. }) && Some(k) != newest {
                    continue;
                }
                unsent |= !matches!(m, Msg::Show(_));
                self.take(m);
            }
            unsent |= self.evict();
            if let Some(i) = self.next() {
                images.extend(self.draw(i));
                unsent = true;
            }
            if unsent && (self.next().is_none() || sent_at.elapsed() >= REPORT_EVERY) {
                let r = Report { epoch: self.epoch, rev: self.rev, slots: self.slots(), images: std::mem::take(&mut images) };
                if tx.send(r).is_err() {
                    return;
                }
                wake.request_repaint();
                unsent = false;
                sent_at = Instant::now();
            }
        }
    }
}

pub struct Thumbs {
    tx: Sender<Msg>,
    rx: Receiver<Report>,
    textures: HashMap<u64, TextureHandle>,
    slots: Vec<Option<Slot>>,
    /// Revision the slots show.
    pub shown_rev: u64,
    sent_rev: u64,
    seen_rev: u64,
    seen_at: f64,
    /// Bumped by `clear`: reports from before it are dropped.
    epoch: u64,
    show: Range<usize>,
    ctx: egui::Context,
}

impl Thumbs {
    pub fn new(ctx: egui::Context, renderer: Arc<Renderer>) -> Self {
        let (tx, jrx) = channel::<Msg>();
        let (rtx, rx) = channel();
        let wake = ctx.clone();
        let w = Worker { renderer, ed: None, epoch: 0, rev: 0, entries: vec![], show: 0..0, next_id: 0 };
        std::thread::Builder::new().name("zebra-thumbs".into()).spawn(move || w.run(jrx, rtx, wake)).expect("spawn thumbnail thread");
        Thumbs {
            tx,
            rx,
            textures: HashMap::new(),
            slots: vec![],
            shown_rev: u64::MAX,
            sent_rev: u64::MAX,
            seen_rev: u64::MAX,
            seen_at: 0.0,
            epoch: 0,
            show: 0..0,
            ctx,
        }
    }

    pub fn clear(&mut self) {
        self.epoch += 1;
        let _ = self.tx.send(Msg::Clear { epoch: self.epoch });
        self.textures.clear();
        self.slots.clear();
        self.shown_rev = u64::MAX;
        self.sent_rev = u64::MAX;
    }

    /// The steps the sidebar shows now (only those are drawn, and a few
    /// either side). Call each frame the list is drawn.
    pub fn show(&mut self, steps: Range<usize>) {
        if steps != self.show {
            self.show = steps.clone();
            let _ = self.tx.send(Msg::Show(steps));
        }
    }

    /// Call every frame.
    pub fn update(&mut self, ed: &Editor, now: f64) {
        while let Ok(r) = self.rx.try_recv() {
            if r.epoch != self.epoch {
                continue;
            }
            for (id, img) in r.images {
                self.textures.insert(id, self.ctx.load_texture(format!("thumb-{id}"), img, TextureOptions::LINEAR));
            }
            let live: std::collections::HashSet<u64> = r.slots.iter().flatten().map(|s| s.id).collect();
            self.textures.retain(|id, _| live.contains(id));
            self.slots = r.slots;
            self.shown_rev = r.rev;
        }
        if ed.revision != self.seen_rev {
            self.seen_rev = ed.revision;
            self.seen_at = now;
        }
        // Debounce: wait until edits pause (or immediately for the first set).
        let quiet = now - self.seen_at > 0.6 || self.sent_rev == u64::MAX;
        if self.sent_rev != ed.revision && quiet && !ed.human_gesture_active() {
            self.sent_rev = ed.revision;
            if !ed.doc.path.is_empty() {
                let _ = self.tx.send(Msg::Doc { epoch: self.epoch, rev: ed.revision, doc: ed.doc.clone() });
            } else {
                self.epoch += 1;
                let _ = self.tx.send(Msg::Clear { epoch: self.epoch });
                self.textures.clear();
                self.slots.clear();
                self.shown_rev = ed.revision;
            }
        } else if self.sent_rev != ed.revision {
            // Wake once the edit burst has gone quiet, not on a poll.
            self.ctx.request_repaint_after(Duration::from_secs_f64((0.6 - (now - self.seen_at)).max(0.05)));
        }
    }

    /// Step `i`'s picture, and whether it is up to date with the newest
    /// document sent (else it is an older one, shown until redrawn).
    pub fn get(&self, i: usize) -> Option<(&TextureHandle, bool)> {
        let s = self.slots.get(i).copied().flatten()?;
        Some((self.textures.get(&s.id)?, s.fresh))
    }

    /// Pictures held (for the `perf` query).
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    pub fn stale(&self, rev: u64) -> bool {
        self.shown_rev != rev
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zebra_core::{Command, Origin};

    fn wait(t: &mut Thumbs, ed: &Editor) {
        for i in 0..600 {
            t.update(ed, 100.0 + i as f64);
            if !t.stale(ed.revision) && (0..ed.doc.path.len()).all(|s| t.slots.get(s).copied().flatten().is_none_or(|s| s.fresh)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("no thumbnails arrived: the thread is gone");
    }

    fn cmd(v: serde_json::Value) -> Command {
        serde_json::from_value(v).unwrap()
    }

    /// Frames at x = 0, 1000, 2000 ... (300 x 200), each a path step,
    /// with a shape in each.
    fn deck(n: usize) -> Editor {
        let mut ed = Editor::new(Document::default());
        for k in 0..n {
            let x = k as f64 * 1000.0;
            ed.apply(cmd(serde_json::json!({ "op": "add_frame", "x": x, "y": 0.0, "w": 300.0, "h": 200.0, "add_to_path": true })), Origin::Human).unwrap();
            ed.apply(cmd(serde_json::json!({ "op": "add_shape", "shape": "rectangle", "x": x, "y": 0.0, "w": 50.0, "h": 50.0 })), Origin::Human).unwrap();
        }
        ed.view.viewport = Vec2::new(THUMB_W as f64, THUMB_H as f64);
        ed
    }

    /// A shape's id in the frame of step `k` of [`deck`].
    fn shape_in(ed: &Editor, k: usize) -> ElementId {
        let x = k as f64 * 1000.0;
        ed.doc.elements.values().find(|e| !e.is_frame() && e.transform.x == x).unwrap().id.clone()
    }

    /// Every step drawn (picture `100 + i` for step i).
    fn drawn(ed: &Editor) -> Vec<Entry> {
        let mut v = carry_over(None, &[], ed);
        for (i, e) in v.iter_mut().enumerate() {
            e.pic = Some(100 + i as u64);
            e.fresh = true;
        }
        v
    }

    /// The steps `carry_over` keeps from `before` to `after`.
    fn kept(before: &Editor, after: &Editor) -> Vec<bool> {
        carry_over(Some(before), &drawn(before), after).iter().map(|e| e.fresh).collect()
    }

    fn snapshot(ed: &Editor) -> Editor {
        let mut s = Editor::new(ed.doc.clone());
        s.view.viewport = ed.view.viewport;
        s
    }

    #[test]
    fn an_edit_redraws_only_the_steps_it_shows_in() {
        let mut ed = deck(4);
        let before = snapshot(&ed);
        let id = shape_in(&ed, 1);
        ed.apply(cmd(serde_json::json!({ "op": "update", "id": id, "patch": { "fill": "#ff0000" } })), Origin::Human).unwrap();
        assert_eq!(kept(&before, &ed), [true, false, true, true]);
    }

    #[test]
    fn a_move_redraws_where_it_was_and_where_it_went() {
        let mut ed = deck(4);
        let before = snapshot(&ed);
        let id = shape_in(&ed, 1);
        ed.apply(cmd(serde_json::json!({ "op": "move", "ids": [id], "dx": 2000.0, "dy": 0.0 })), Origin::Human).unwrap();
        assert_eq!(kept(&before, &ed), [true, false, true, false]);
    }

    #[test]
    fn undo_and_redo_redraw_the_same_steps() {
        let mut ed = deck(4);
        let id = shape_in(&ed, 2);
        ed.apply(cmd(serde_json::json!({ "op": "move", "ids": [id], "dx": 10.0, "dy": 0.0 })), Origin::Human).unwrap();
        let moved = snapshot(&ed);
        ed.undo(Origin::Human).unwrap();
        assert_eq!(kept(&moved, &ed), [true, true, false, true]);
        let undone = snapshot(&ed);
        ed.redo(Origin::Human).unwrap();
        assert_eq!(kept(&undone, &ed), [true, true, false, true]);
    }

    #[test]
    fn a_theme_change_redraws_every_step() {
        let mut ed = deck(3);
        let before = snapshot(&ed);
        ed.apply(cmd(serde_json::json!({ "op": "set_theme", "theme": "chalkboard" })), Origin::Human).unwrap();
        assert_eq!(kept(&before, &ed), [false, false, false]);
    }

    #[test]
    fn a_change_that_draws_nothing_keeps_every_step() {
        let mut ed = deck(3);
        let before = snapshot(&ed);
        ed.apply(Command::SetTitle { title: "again".into() }, Origin::Human).unwrap();
        assert_eq!(kept(&before, &ed), [true, true, true]);
    }

    #[test]
    fn restacking_redraws_only_the_step_it_shows_in() {
        let mut ed = deck(3);
        let before = snapshot(&ed);
        let id = shape_in(&ed, 0);
        ed.apply(cmd(serde_json::json!({ "op": "z_order", "ids": [id], "order": "back" })), Origin::Human).unwrap();
        assert_eq!(kept(&before, &ed), [false, true, true]);
    }

    #[test]
    fn reordered_steps_keep_their_pictures() {
        let mut ed = deck(3);
        let before = snapshot(&ed);
        let pics = drawn(&before);
        ed.apply(Command::MoveStep { from: 0, to: 2 }, Origin::Human).unwrap();
        let after = carry_over(Some(&before), &pics, &ed);
        let got: Vec<(Option<u64>, bool)> = after.iter().map(|e| (e.pic, e.fresh)).collect();
        assert_eq!(got, [(Some(101), true), (Some(102), true), (Some(100), true)]);
    }

    #[test]
    fn a_moved_frame_redraws_its_step_and_shows_the_old_picture_meanwhile() {
        let mut ed = deck(3);
        let before = snapshot(&ed);
        let pics = drawn(&before);
        let frame = ed.doc.elements.values().find(|e| e.is_frame() && e.transform.x == 2000.0).unwrap().id.clone();
        ed.apply(cmd(serde_json::json!({ "op": "move", "ids": [frame], "dx": 0.0, "dy": 5000.0 })), Origin::Human).unwrap();
        let after = carry_over(Some(&before), &pics, &ed);
        let got: Vec<(Option<u64>, bool)> = after.iter().map(|e| (e.pic, e.fresh)).collect();
        assert_eq!(got, [(Some(100), true), (Some(101), true), (Some(102), false)]);
    }

    #[test]
    fn a_build_redraws_the_steps_before_it_that_show_it() {
        let mut ed = deck(3);
        let before = snapshot(&ed);
        // Step 1's shape fades in at step 2: hidden at steps 0 and 1, but
        // only step 1 shows it.
        let id = shape_in(&ed, 1);
        ed.apply(cmd(serde_json::json!({ "op": "update_step", "index": 2, "builds": [id] })), Origin::Human).unwrap();
        assert_eq!(kept(&before, &ed), [true, false, true]);
    }

    #[test]
    fn only_steps_on_screen_are_drawn_and_far_ones_let_go() {
        let renderer = Arc::new(Renderer::new());
        let ed = deck(3);
        let mut w = Worker { renderer, ed: None, epoch: 0, rev: 0, entries: vec![], show: 0..0, next_id: 0 };
        w.take(Msg::Doc { epoch: 0, rev: 1, doc: ed.doc.clone() });
        assert_eq!(w.next(), None, "nothing on screen, nothing drawn");
        w.take(Msg::Show(1..2));
        assert_eq!(w.next(), Some(1), "the step on screen first");
        let mut n = 0;
        while let Some(i) = w.next() {
            assert!(w.draw(i).is_some());
            n += 1;
        }
        assert_eq!(n, 3, "then the margin");
        w.show = 1000..1001;
        assert!(w.evict());
        assert!(w.entries.iter().all(|e| e.pic.is_none() && !e.fresh), "far from view: let go");
    }

    #[test]
    fn a_step_that_cannot_be_drawn_does_not_end_the_thumbnails() {
        let renderer = std::sync::Arc::new(zebra_render::Renderer::new());
        let mut ed = Editor::new(Document::default());
        ed.measure = renderer.measure();
        for x in [0.0, 500.0] {
            let add: Command = serde_json::from_value(serde_json::json!({ "op": "add_frame", "x": x, "y": 0.0, "w": 300.0, "h": 200.0, "add_to_path": true })).unwrap();
            ed.apply(add, Origin::Human).unwrap();
        }
        assert_eq!(ed.doc.path.len(), 2);
        ed.apply(Command::SetTitle { title: FAIL_TITLE.into() }, Origin::Human).unwrap();
        let mut t = Thumbs::new(egui::Context::default(), renderer);
        t.show(0..2);
        FAIL_STEP.store(1, std::sync::atomic::Ordering::Relaxed);
        wait(&mut t, &ed);
        assert!(t.get(0).is_some() && t.get(1).is_none(), "the step that failed is empty, the other is drawn");
        FAIL_STEP.store(usize::MAX, std::sync::atomic::Ordering::Relaxed);
        ed.apply(Command::SetTitle { title: format!("{FAIL_TITLE} again") }, Origin::Human).unwrap();
        wait(&mut t, &ed);
        assert!(t.get(0).is_some() && t.get(1).is_some(), "the next round draws both");
    }
}
