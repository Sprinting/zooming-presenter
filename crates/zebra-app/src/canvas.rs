//! The canvas: content drawing (vello via zebra-render), direct manipulation
//! (select, zebra transform tool, marquee, create, draw, pan/zoom, snapping),
//! in-place text editing, path-edit badges and agent presence overlays.
//!
//! Every gesture becomes zebra-core commands. Drags are one transaction
//! (`begin` … live `apply`s … `commit`), so a drag is one undo step and agents
//! get `busy` while the human holds the mouse.
use std::collections::HashSet;

use egui::{Align2, Color32, CursorIcon, FontId, Key, Painter, PointerButton, Pos2, Rect as ERect, Sense, Shape, Stroke, Ui};
use zebra_core::{
    Command, ElementId, ElementKind, ElementPatch, FrameData, FrameStyle, Mode, NewElement, Origin, Rect, ShapeData,
    ListKind, ShapeKind, SpanFormat, StepTarget, TextData, TextStyle, Tool, Vec2, ViewCommand,
};

use crate::app::{ScrollMode, ZebraApp};
use crate::style::{self, AGENT, GUIDE, PATH};

const H: Origin = Origin::Human;

#[derive(Debug, Clone, Default)]
pub enum Gesture {
    #[default]
    None,
    Pan,
    Marquee {
        start: Pos2,
        additive: bool,
    },
    Move {
        ids: Vec<ElementId>,
        origin: Vec2,
        applied: Vec2,
        start_bounds: Rect,
        moving: HashSet<ElementId>,
    },
    Scale {
        ids: Vec<ElementId>,
        anchor: Vec2,
        start_dist: f64,
        applied: f64,
    },
    Rotate {
        ids: Vec<ElementId>,
        center: Vec2,
        start_angle: f64,
        applied: f64,
        base_rot: f64,
    },
    Create {
        tool: Tool,
        start: Vec2,
    },
    Ink {
        points: Vec<Vec2>,
    },
    Badge {
        step: usize,
        /// The step as it was when the drag began: if the path changes
        /// under the drag, `step` may be another step by the time it ends.
        was: Box<zebra_core::PathStep>,
    },
    /// Dragging one end of a connector (`to_end`: the arrow end).
    ConnectorEnd {
        id: ElementId,
        to_end: bool,
    },
    /// Dragging a text box's left or right width handle. `anchor`: the
    /// world point of the opposite top corner, which stays put.
    TextWidth {
        id: ElementId,
        right: bool,
        anchor: Vec2,
    },
}

/// World positions of a connector's two ends.
pub fn connector_ends(app: &ZebraApp, id: &str) -> Option<[Vec2; 2]> {
    let e = app.editor.doc.get(id)?;
    let ElementKind::Connector(c) = &e.kind else { return None };
    Some([e.transform.apply(Vec2::new(c.from.x, c.from.y)), e.transform.apply(Vec2::new(c.to.x, c.to.y))])
}

/// The element a connector end dropped at `w` would stick to: the topmost
/// non-connector under the point (frames by their border).
pub fn attach_target(app: &ZebraApp, w: Vec2, exclude: Option<&str>) -> Option<ElementId> {
    let ed = &app.editor;
    let tol = 5.0 / ed.view.camera.zoom;
    ed.elements_at(w, tol)
        .into_iter()
        .rev()
        .filter(|e| Some(e.id.as_str()) != exclude && !matches!(e.kind, ElementKind::Connector(_)))
        .find(|e| e.hit(w, tol, false))
        .map(|e| e.id.clone())
}

/// The selected connector, when exactly one is selected.
fn sole_connector(app: &ZebraApp) -> Option<ElementId> {
    match app.editor.view.selection.as_slice() {
        [one] if app.editor.doc.get(one).is_some_and(|e| matches!(e.kind, ElementKind::Connector(_)) && !e.locked) => Some(one.clone()),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct TextEditState {
    /// None = a new text box (created on commit).
    pub id: Option<ElementId>,
    /// World position of the box's top-left (new text only).
    pub anchor: Vec2,
    pub buf: String,
    pub original: String,
    pub style: TextStyle,
    pub bold: bool,
    pub italic: bool,
    pub bullets: bool,
    pub scale: f64,
    pub focused_once: bool,
    pub size_px: egui::Vec2,
    pub zoom: f64,
    /// Character and paragraph formatting, kept in step with `buf`.
    pub rich: TextData,
    /// Selected characters (start, end) as of the last frame.
    pub sel: (usize, usize),
    /// The link field is open with this URL.
    pub link_draft: Option<String>,
    /// Wrap width in local units (fixed-width boxes and text on shapes).
    pub wrap: Option<f64>,
    /// New text typed on this shape: kept centred on it, and grouped with
    /// it on commit.
    pub host: Option<ElementId>,
    /// Misspelled words, checked when the text changes.
    pub spell: crate::spell::Cache,
    /// The misspelled word the spelling menu is open for (chars).
    pub spell_word: Option<std::ops::Range<usize>>,
    /// (caret, text length) last frame: the canvas follows the caret only
    /// when one of them changed, never against a pan made by hand.
    pub caret_seen: Option<(usize, usize)>,
    /// The caret or text changed last frame: bring the caret into view now.
    /// (Not in the frame of the change itself: there the text is still laid
    /// out at its old width and wraps, so the caret is in the wrong place.)
    pub caret_follow: bool,
    /// A width handle is being dragged: (right handle, world point of the
    /// opposite top corner, which stays put).
    pub width_drag: Option<(bool, Vec2)>,
    /// A press on the selected words, or the drag it became (spec 18c).
    pub drag: Option<crate::text_drag::TextDrag>,
    /// Select these characters when the field next starts (after words
    /// moved inside the box).
    pub pending_sel: Option<(usize, usize)>,
    /// The selection's right-click menu is open.
    pub sel_menu: bool,
    /// The pointer was on the selected words last frame (or their drag or
    /// menu is going on).
    pub on_words: bool,
    /// The text as laid out last frame, and where it was drawn (screen):
    /// where each character is, for the drag and for tests.
    pub laid_out: Option<(std::sync::Arc<egui::Galley>, Pos2)>,
}

pub struct CanvasState {
    pub rect: ERect,
    pub gesture: Gesture,
    pub hover: Option<ElementId>,
    pub text: Option<TextEditState>,
    pub text_toolbar: Option<ERect>,
    pub guides: Vec<[Vec2; 2]>,
    pub frame_chip: Option<(ERect, ElementId)>,
    /// The chip's › part: click zooms into the frame.
    pub chip_chevron: Option<ERect>,
    /// Renaming a frame in place from its chip: (frame, text, focused yet).
    pub renaming: Option<(ElementId, String, bool)>,
    /// The off-screen agent label: click flies there.
    pub agent_edge: Option<ERect>,
    pub step_hover: Option<usize>,
    pub rect_valid: bool,
    pub rect_presenting: bool,
    /// Window content size when `rect` was last synced.
    pub window_size: egui::Vec2,
    /// Bold / Italic / Underline chosen from the menu bar while editing text
    /// (on macOS the menu owns ⌘B/⌘I/⌘U): applied to the text selection.
    pub text_menu: Option<crate::actions::Action>,
    /// After leaving Present: keep re-fitting this camera target while the
    /// window settles (leaving full screen resizes over several frames),
    /// until this time (`view.now`) or the first pointer press.
    pub refit_until: Option<(f64, Option<usize>)>,
    /// Where the canvas context menu was opened (screen point).
    pub menu_at: Option<Pos2>,
    /// Option was down when the primary button went down: a click or drag
    /// on a frame then takes the frame on its own, without what it holds.
    /// Read at the press, so letting Option go mid-drag changes nothing.
    pub alt_press: bool,
    /// Cells selected in the selected table, or one being edited.
    pub table: Option<crate::table_ui::TableSel>,
}

impl Default for CanvasState {
    fn default() -> Self {
        CanvasState {
            rect: ERect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0)),
            gesture: Gesture::None,
            hover: None,
            text: None,
            text_toolbar: None,
            menu_at: None,
            guides: vec![],
            frame_chip: None,
            chip_chevron: None,
            renaming: None,
            agent_edge: None,
            step_hover: None,
            rect_valid: false,
            rect_presenting: false,
            window_size: egui::Vec2::ZERO,
            text_menu: None,
            refit_until: None,
            alt_press: false,
            table: None,
        }
    }
}

impl CanvasState {
    pub fn w2s(&self, ed: &zebra_core::Editor, p: Vec2) -> Pos2 {
        let s = ed.view.camera.world_to_screen(p, ed.view.viewport);
        Pos2::new(self.rect.min.x + s.x as f32, self.rect.min.y + s.y as f32)
    }
    pub fn s2w(&self, ed: &zebra_core::Editor, p: Pos2) -> Vec2 {
        let v = Vec2::new((p.x - self.rect.min.x) as f64, (p.y - self.rect.min.y) as f64);
        ed.view.camera.screen_to_world(v, ed.view.viewport)
    }
    fn local(&self, p: Pos2) -> [f64; 2] {
        [(p.x - self.rect.min.x) as f64, (p.y - self.rect.min.y) as f64]
    }
}

/// Selection box on screen: 4 corners (tl, tr, br, bl), world corners too.
pub struct SelBox {
    pub world: [Vec2; 4],
    pub screen: [Pos2; 4],
    pub center: Vec2,
    pub rotation: f64,
    /// A single unlocked text box: it gets width handles.
    pub text: Option<ElementId>,
}

pub fn selection_box(app: &ZebraApp, ids: &[ElementId]) -> Option<SelBox> {
    let ed = &app.editor;
    let world = match ids {
        [] => return None,
        [one] => ed.doc.get(one)?.corners(),
        many => {
            let b = ed.bounds_of(many);
            [Vec2::new(b.x0, b.y0), Vec2::new(b.x1, b.y0), Vec2::new(b.x1, b.y1), Vec2::new(b.x0, b.y1)]
        }
    };
    let rotation = match ids {
        [one] => ed.doc.get(one).map(|e| e.transform.rotation).unwrap_or(0.0),
        _ => 0.0,
    };
    let center = (world[0] + world[2]) * 0.5;
    let text = match ids {
        [one] => ed.doc.get(one).filter(|e| matches!(e.kind, ElementKind::Text(_)) && !e.locked).map(|e| e.id.clone()),
        _ => None,
    };
    Some(SelBox { world, screen: world.map(|p| app.canvas.w2s(ed, p)), center, rotation, text })
}

/// Screen points of a text box's width handles: the middles of its left
/// and right edges.
fn width_handles(sb: &SelBox) -> [Pos2; 2] {
    [sb.screen[0].lerp(sb.screen[3], 0.5), sb.screen[1].lerp(sb.screen[2], 0.5)]
}

/// The resize cursor for dragging along the screen direction `d`.
fn resize_cursor(d: egui::Vec2) -> CursorIcon {
    let a = d.y.atan2(d.x).to_degrees().rem_euclid(180.0);
    match a {
        a if !(22.5..157.5).contains(&a) => CursorIcon::ResizeHorizontal,
        a if a < 67.5 => CursorIcon::ResizeNwSe,
        a if a < 112.5 => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeNeSw,
    }
}

fn rot_handle_pos(sb: &SelBox) -> Pos2 {
    let tr = sb.screen[1];
    let c = sb.screen[0].lerp(sb.screen[2], 0.5);
    let d = (tr - c).normalized();
    tr + d * 26.0
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Handle {
    Corner(usize),
    /// A text box's width handle (`true`: the right one).
    Width(bool),
    Rotate,
    Body,
}

fn hit_handle(sb: &SelBox, p: Pos2) -> Option<Handle> {
    if rot_handle_pos(sb).distance(p) < 13.0 {
        return Some(Handle::Rotate);
    }
    // The nearest handle wins: on a one-line text the width handles sit
    // close to the corners.
    let corners = sb.screen.iter().enumerate().map(|(i, c)| (Handle::Corner(i), *c));
    let widths = sb.text.as_ref().map(|_| width_handles(sb)).into_iter().flat_map(|[l, r]| [(Handle::Width(false), l), (Handle::Width(true), r)]);
    let near = corners.chain(widths).map(|(h, c)| (h, c.distance(p))).filter(|(_, d)| *d < 9.0).min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((h, _)) = near {
        return Some(h);
    }
    if point_in_quad(p, &sb.screen) {
        return Some(Handle::Body);
    }
    None
}

fn point_in_quad(p: Pos2, q: &[Pos2; 4]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let c = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if c != 0.0 {
            if sign == 0.0 {
                sign = c.signum();
            } else if c.signum() != sign {
                return false;
            }
        }
    }
    true
}

/// Topmost element at a screen point. Frames by their border; with
/// `frames_inside` the innermost frame containing the point also counts.
pub fn hit(app: &ZebraApp, p: Pos2, frames_inside: bool) -> Option<ElementId> {
    let ed = &app.editor;
    let w = app.canvas.s2w(ed, p);
    let tol = 5.0 / ed.view.camera.zoom;
    let hidden = ed.hidden_builds();
    let presenting = ed.view.mode == Mode::Present;
    let under = ed.elements_at(w, tol);
    let found = under
        .iter()
        .rev()
        .filter(|e| !hidden.contains(&e.id))
        .filter(|e| !(presenting && matches!(&e.kind, ElementKind::Frame(f) if f.style == FrameStyle::Invisible)))
        .find(|e| e.hit(w, tol, false))
        .map(|e| e.id.clone());
    if found.is_some() || !frames_inside {
        return found;
    }
    under
        .into_iter()
        .filter(|e| e.is_frame() && e.hit(w, tol, true))
        .filter(|e| !(presenting && matches!(&e.kind, ElementKind::Frame(f) if f.style == FrameStyle::Invisible)))
        .min_by(|a, b| a.bounds().area().total_cmp(&b.bounds().area()))
        .map(|e| e.id.clone())
}

// ----------------------------------------------------------------------
// Content

/// Draw the document into `rect`. Uses the GPU renderer when available.
pub fn paint_content(app: &mut ZebraApp, ui: &mut Ui, rect: ERect, presenting: bool) {
    let ed = &app.editor;
    let mut opts = zebra_render::SceneOptions { presenting, ..Default::default() };
    if presenting {
        // Fade-ins still to come: hidden, except in an overview, where they
        // show dimmed (critique 14).
        let dimmed = ed.dimmed_builds();
        opts.hidden = ed.hidden_builds().into_iter().filter(|id| !dimmed.contains(id)).collect();
        let now = ed.view.now;
        opts.opacity = app
            .revealed
            .iter()
            .filter_map(|(id, t)| {
                let a = ((now - t) / 0.3).clamp(0.0, 1.0) as f32;
                (a < 1.0).then(|| (id.clone(), a))
            })
            .collect();
        opts.opacity.extend(dimmed.into_iter().map(|id| (id, 0.25)));
    }
    opts.skip = app.canvas.text.as_ref().and_then(|t| t.id.clone());
    opts.time = Some(ed.view.now);
    opts.async_images = true;
    app.media.scene(&mut opts);
    opts.card_progress = crate::card_window::progress(app);
    let ppp = ui.ctx().pixels_per_point() as f64;
    let vp = [rect.width() as f64, rect.height() as f64];
    if let (Some(canvas), Some(rs), true) = (&mut app.gpu_canvas, &app.wgpu, app.render_ok) {
        let cam = ed.view.camera;
        let r = &app.renderer;
        let t = std::time::Instant::now();
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r.build_display_list(ed, &cam, vp, ppp, &opts)));
        let t_list = t.elapsed().as_secs_f32() * 1000.0;
        match built {
            Ok(list) => {
                let t = std::time::Instant::now();
                canvas.paint(ui, rect, &list, rs);
                let t_paint = t.elapsed().as_secs_f32() * 1000.0;
                app.editor.perf.record_part("canvas_scene", t_list);
                app.editor.perf.record_part("canvas_paint", t_paint);
                let [enc, submit, wait] = canvas.last_ms();
                app.editor.perf.record_part("vello_encode", enc);
                app.editor.perf.record_part("vello_submit", submit);
                app.editor.perf.record_part("vello_scope_wait", wait);
                // Animated GIFs on screen: repaint for their next frame only.
                if let Some(t) = list.next_frame_in {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(t.clamp(0.01, 10.0)));
                }
                return;
            }
            Err(_) => {
                eprintln!("zebra: renderer not available; using the simple fallback painter");
                app.render_ok = false;
            }
        }
    }
    let painter = ui.painter_at(rect);
    crate::fallback::paint(
        &painter,
        &app.editor,
        &app.editor.view.camera,
        rect.min,
        app.editor.view.viewport,
        presenting,
        &opts.hidden,
        opts.skip.as_deref(),
        &opts.opacity,
    );
}

/// Keep the core's viewport equal to the canvas size.
pub fn sync_viewport(app: &mut ZebraApp, rect: ERect, window: egui::Vec2) {
    let vp = app.editor.view.viewport;
    let old = app.canvas.rect;
    if (vp.x - rect.width() as f64).abs() > 0.5 || (vp.y - rect.height() as f64).abs() > 0.5 {
        let _ = app.editor.view(ViewCommand::SetViewport { width: rect.width().max(1.0) as f64, height: rect.height().max(1.0) as f64 }, H);
    }
    // Presenting, and the show's area changed size: a step is fitted to the
    // size the canvas had at that moment, and Play from a small window fits
    // step 1 to that window while it is still growing into full screen (on
    // the Mac the step then sat small in a corner of the screen). Fit the
    // step again; a flight still under way keeps flying, to the new fit.
    if app.editor.view.mode == Mode::Present && app.canvas.rect_valid && old.size() != rect.size() && !app.editor.view.present.off_path {
        let ed = &app.editor;
        let cam = match ed.view.present.step {
            Some(i) if i < ed.doc.path.len() => ed.camera_for_step(i),
            _ => ed.fit_all_camera(),
        };
        match &mut app.editor.view.flight {
            Some(f) => f.to = cam,
            None => app.editor.view.camera = cam,
        }
    }
    // When a side panel opens or closes inside an unchanged window, keep the
    // content still on screen (nothing jumps under the pointer). When the
    // window itself changes (resize, leaving full screen after presenting),
    // keep the view centred instead, so nothing ends up under a panel.
    let same_mode = app.canvas.rect_presenting == (app.editor.view.mode == Mode::Present);
    let same_window = (app.canvas.window_size - window).length() < 0.5;
    let changed = old != rect && app.canvas.rect_valid;
    if same_mode && same_window && changed && app.editor.view.flight.is_none() {
        let d = old.center() - rect.center();
        if d.length() > 0.1 {
            let _ = app.editor.view(ViewCommand::Pan { dx: d.x as f64, dy: d.y as f64 }, H);
        }
    }
    // Back from presenting: the camera still has the show's zoom, fitted to
    // the whole screen. Re-fit the step you were on into the editor canvas
    // (critique 13), with no animation, and keep doing so while the window
    // leaves full screen.
    let now = app.editor.view.now;
    if app.canvas.rect_valid && app.canvas.rect_presenting && app.editor.view.mode != Mode::Present && !app.editor.view.present.off_path {
        app.canvas.refit_until = Some((now + 1.0, app.editor.view.present.step));
    }
    if let Some((until, step)) = app.canvas.refit_until {
        if now > until || app.editor.view.mode == Mode::Present {
            app.canvas.refit_until = None;
        } else if old != rect || app.canvas.rect_presenting {
            let ed = &app.editor;
            let cam = match step {
                Some(i) if i < ed.doc.path.len() => ed.camera_for_step(i),
                _ => ed.fit_all_camera(),
            };
            app.editor.view.flight = None;
            app.editor.view.fly_to(cam, false);
        }
    }
    if !app.canvas.rect_valid && app.editor.view.mode != Mode::Present && app.editor.view.flight.is_none() {
        // First real canvas size (startup / after opening a file): fit everything.
        let cam = app.editor.fit_all_camera();
        app.editor.view.fly_to(cam, false);
    }
    app.canvas.rect = rect;
    app.canvas.rect_valid = true;
    app.canvas.rect_presenting = app.editor.view.mode == Mode::Present;
    app.canvas.window_size = window;
    // A panel that just opened (e.g. properties on selecting) must not hide
    // the selection: pan it back into view.
    if changed && app.editor.view.mode != Mode::Present && !app.editor.view.selection.is_empty() {
        reveal_selection(app);
    }
}

/// Pan (animated) so the human selection is inside the canvas, if it isn't.
pub fn reveal_selection(app: &mut ZebraApp) {
    let sel = app.editor.view.selection.clone();
    let b = app.editor.bounds_of(&sel);
    let ed = &app.editor;
    let corners = [Vec2::new(b.x0, b.y0), Vec2::new(b.x1, b.y0), Vec2::new(b.x1, b.y1), Vec2::new(b.x0, b.y1)]
        .map(|p| ed.view.camera.world_to_screen(p, ed.view.viewport));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for c in corners {
        x0 = x0.min(c.x);
        y0 = y0.min(c.y);
        x1 = x1.max(c.x);
        y1 = y1.max(c.y);
    }
    let (w, h) = (ed.view.viewport.x, ed.view.viewport.y);
    let m = 24.0;
    let shift = |lo: f64, hi: f64, size: f64| -> f64 {
        if hi - lo > size - 2.0 * m {
            size / 2.0 - (lo + hi) / 2.0 // too big: centre it
        } else if lo < m {
            m - lo
        } else if hi > size - m {
            size - m - hi
        } else {
            0.0
        }
    };
    let (dx, dy) = (shift(x0, x1, w), shift(y0, y1, h));
    if dx.abs() > 0.5 || dy.abs() > 0.5 {
        let mut cam = ed.view.target_camera();
        cam.pan_screen(Vec2::new(dx, dy));
        let _ = app.editor.view(ViewCommand::SetCamera { camera: cam, animate: true }, H);
    }
}

/// Wheel / trackpad / pinch handling shared by edit and present mode.
pub fn handle_scroll(app: &mut ZebraApp, ui: &Ui, hovered: bool) -> bool {
    if !hovered {
        return false;
    }
    let events = ui.input(|i| i.events.clone());
    let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(app.canvas.rect.center());
    let anchor = app.canvas.local(pointer);
    let mut did = false;
    for ev in events {
        match ev {
            egui::Event::MouseWheel { unit, delta, source, modifiers, .. } => {
                let (dx, dy) = match unit {
                    egui::MouseWheelUnit::Point => (delta.x as f64, delta.y as f64),
                    egui::MouseWheelUnit::Line => (delta.x as f64 * 40.0, delta.y as f64 * 40.0),
                    egui::MouseWheelUnit::Page => (delta.x as f64 * 400.0, delta.y as f64 * 400.0),
                };
                let trackpad = match source {
                    egui::MouseWheelSource::Trackpad | egui::MouseWheelSource::Momentum => true,
                    egui::MouseWheelSource::Wheel => false,
                    // Unknown: a pixel-precise delta is (almost always) a trackpad.
                    _ => unit == egui::MouseWheelUnit::Point,
                };
                let zoom = modifiers.command
                    || modifiers.ctrl
                    || match app.scroll_mode {
                        ScrollMode::Zoom => true,
                        ScrollMode::Pan => false,
                        ScrollMode::Auto => !trackpad,
                    };
                if zoom {
                    let f = (dy * 0.0035).exp().clamp(0.5, 2.0);
                    let _ = app.editor.view(ViewCommand::Zoom { factor: f, anchor: Some(anchor) }, H);
                } else {
                    let _ = app.editor.view(ViewCommand::Pan { dx, dy }, H);
                }
                did = true;
            }
            egui::Event::Zoom(f) => {
                let _ = app.editor.view(ViewCommand::Zoom { factor: f as f64, anchor: Some(anchor) }, H);
                did = true;
            }
            _ => {}
        }
    }
    if did && app.editor.view.follow_agent {
        let _ = app.editor.view(ViewCommand::FollowAgent { on: false }, H);
    }
    did
}

// ----------------------------------------------------------------------
// Edit-mode canvas

pub fn edit_canvas(app: &mut ZebraApp, ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    sync_viewport(app, rect, ui.ctx().content_rect().size());
    if ui.input(|i| i.pointer.any_pressed() || i.smooth_scroll_delta != egui::Vec2::ZERO || i.zoom_delta() != 1.0) {
        app.canvas.refit_until = None;
    }
    let resp = ui.interact(rect, app.uid("zebra-canvas"), Sense::click_and_drag());
    // A press on the canvas gives it the keys back from the path list.
    if resp.hovered() && ui.input(|i| i.pointer.any_pressed()) {
        app.path_focus = false;
    }
    paint_content(app, ui, rect, false);
    if crate::media_ui::crop_active(app) {
        crate::media_ui::crop_interact(app, ui, &resp);
        return;
    }

    let ctx = ui.ctx().clone();
    let text_focus = ctx.egui_wants_keyboard_input();
    let space = !text_focus && ctx.input(|i| i.key_down(Key::Space));
    let pointer = resp.hover_pos();
    let mods = ctx.input(|i| i.modifiers);
    let path_mode = app.editor.view.mode == Mode::PathEdit;
    let tool = app.editor.view.tool;
    let zoom = app.editor.view.camera.zoom;

    handle_scroll(app, ui, resp.contains_pointer());
    if !path_mode && app.editor.view.mode != Mode::Present {
        context_menu(app, &resp);
    }

    // Hover
    app.canvas.hover = match (&app.canvas.gesture, pointer) {
        (Gesture::None, Some(p)) => hit(app, p, path_mode),
        _ => None,
    };
    let sel = app.selection();
    // Cells selected in a table: no transform handles (they show at table level).
    let sb = if path_mode || crate::table_ui::active(app) { None } else { selection_box(app, &sel) };
    let handle_under = pointer.and_then(|p| sb.as_ref().and_then(|b| hit_handle(b, p)));
    let badge_under = if path_mode { pointer.and_then(|p| badge_at(app, p)) } else { None };
    app.canvas.step_hover = badge_under;
    // A step waits for a frame: the frame a click here would give it.
    let pick_hover = match (&app.canvas.gesture, pointer) {
        (Gesture::None, Some(p)) if app.step_pick.is_some() && matches!(tool, Tool::Select | Tool::Hand) => pick_target(app, p),
        _ => None,
    };
    if app.step_pick.is_some() {
        // Only the frame a click would pick shows as hovered (with its chip).
        app.canvas.hover = pick_hover.clone();
    }

    // Enter edits the selected text.
    if !text_focus && app.canvas.text.is_none() && ctx.input(|i| i.key_pressed(Key::Enter) && i.modifiers.is_none()) {
        if let [id] = sel.as_slice() {
            if matches!(app.editor.doc.get(id).map(|e| &e.kind), Some(ElementKind::Text(_))) {
                begin_edit_existing(app, id.clone());
            }
        }
    }

    // ---- press / drag
    if ctx.input(|i| i.pointer.primary_pressed()) {
        app.canvas.alt_press = mods.alt;
    }
    let middle = resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary);
    if middle {
        let d = resp.drag_delta();
        let _ = app.editor.view(ViewCommand::Pan { dx: d.x as f64, dy: d.y as f64 }, H);
    }
    if resp.drag_started_by(PointerButton::Primary) {
        let start = ctx.input(|i| i.pointer.press_origin()).unwrap_or_default();
        commit_text_edit(app);
        if !(tool == Tool::Select && !space && crate::table_ui::drag_start(app, start)) {
            start_gesture(app, start, space, mods.shift);
        }
    }
    if resp.dragged_by(PointerButton::Primary) {
        if let Some(p) = resp.interact_pointer_pos() {
            if crate::table_ui::dragging(app) {
                crate::table_ui::drag_update(app, p);
            } else {
                update_gesture(app, p, resp.drag_delta(), mods.shift, mods.alt);
            }
        }
    }
    if resp.drag_stopped_by(PointerButton::Primary) {
        let p = resp.interact_pointer_pos().or(pointer).unwrap_or_default();
        if crate::table_ui::dragging(app) {
            crate::table_ui::drag_end(app);
        } else {
            end_gesture(app, p, mods.shift);
        }
    }
    if crate::table_ui::dragging(app) && !ctx.input(|i| i.pointer.any_down()) {
        crate::table_ui::drag_end(app);
    }
    // A drag that ended where the canvas didn't see it (a sheet or menu
    // took the pointer, or the button came up outside the window): end it
    // now so no snapping guides or previews stay behind.
    if !matches!(app.canvas.gesture, Gesture::None) && !resp.dragged() && !ctx.input(|i| i.pointer.any_down()) {
        let p = pointer.unwrap_or_default();
        end_gesture(app, p, mods.shift);
    }

    // ---- clicks
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if app.step_pick.is_some() && pick_click(app, p) {
                // The waiting step took the frame, or the insert ended.
            } else if app.canvas.text.is_some() {
                commit_text_edit(app);
            } else if app.canvas.agent_edge.is_some_and(|r| r.contains(p)) && app.editor.agent_cursor.is_some() {
                let c = app.editor.agent_cursor.unwrap_or_default();
                let camera = zebra_core::Camera { cx: c.x, cy: c.y, ..app.editor.view.camera };
                let _ = app.editor.view(ViewCommand::SetCamera { camera, animate: true }, H);
            } else if let Some((_, id)) = app.canvas.frame_chip.clone().filter(|(r, _)| r.contains(p)) {
                // Chip: › zooms in; the name selects (double-click renames).
                if app.canvas.chip_chevron.is_some_and(|c| c.contains(p)) {
                    let _ = app.editor.view(ViewCommand::ZoomTo { id, animate: true }, H);
                } else if app.canvas.alt_press {
                    select_frame_only(app, id);
                } else {
                    let _ = app.editor.view(ViewCommand::Select { ids: vec![id], frame_only: false }, H);
                }
            } else if !(tool == Tool::Select && crate::table_ui::click(app, p, mods.shift)) {
                click(app, p, mods.shift, app.canvas.alt_press);
            }
        }
    }
    if resp.double_clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let chip = app.canvas.frame_chip.clone().filter(|(r, _)| r.contains(p));
            match chip {
                Some((_, id)) if !app.canvas.chip_chevron.is_some_and(|c| c.contains(p)) => {
                    let name = app.editor.doc.get(&id).map(|e| crate::words::element_label(e, &app.editor.doc)).unwrap_or_default();
                    app.canvas.renaming = Some((id, name, false));
                }
                Some(_) => {}
                None => {
                    if !(tool == Tool::Select && (crate::table_ui::double_click(app, p) || crate::chart_ui::double_click(app, p))) {
                        double_click(app, p)
                    }
                }
            }
        }
    }
    // Chip tooltips (critique 27).
    if let (Some(p), Some((r, _)), Gesture::None) = (pointer, &app.canvas.frame_chip, &app.canvas.gesture) {
        if r.contains(p) && app.canvas.renaming.is_none() {
            let tip = if app.canvas.chip_chevron.is_some_and(|c| c.contains(p)) { crate::t!("canvas-chip-zoom-into-frame") } else { crate::t!("canvas-chip-rename-hint") };
            resp.clone().on_hover_text(tip);
        }
    }

    // ---- cursor
    let cursor = match (&app.canvas.gesture, handle_under) {
        (Gesture::Pan, _) => CursorIcon::Grabbing,
        (Gesture::Rotate { .. }, _) | (_, Some(Handle::Rotate)) => CursorIcon::Alias,
        (Gesture::Scale { .. }, _) => CursorIcon::ResizeNwSe,
        (Gesture::TextWidth { .. }, _) => sb.as_ref().map_or(CursorIcon::ResizeHorizontal, |b| resize_cursor(b.screen[1] - b.screen[0])),
        (_, Some(Handle::Width(_))) => sb.as_ref().map_or(CursorIcon::ResizeHorizontal, |b| resize_cursor(b.screen[1] - b.screen[0])),
        (_, Some(Handle::Corner(i))) => {
            if i % 2 == 0 { CursorIcon::ResizeNwSe } else { CursorIcon::ResizeNeSw }
        }
        (Gesture::Move { .. }, _) => CursorIcon::Move,
        _ if space || tool == Tool::Hand => CursorIcon::Grab,
        _ if path_mode && badge_under.is_some() => CursorIcon::Grab,
        _ if path_mode && app.canvas.hover.is_some() => CursorIcon::PointingHand,
        _ if tool == Tool::Text => CursorIcon::Text,
        _ if matches!(tool, Tool::Frame | Tool::Rectangle | Tool::Ellipse | Tool::Line | Tool::Arrow | Tool::Pen | Tool::Triangle | Tool::Highlighter) => {
            CursorIcon::Crosshair
        }
        (_, Some(Handle::Body)) => CursorIcon::Move,
        _ => CursorIcon::Default,
    };
    let cursor = if pick_hover.is_some()
        || pointer.is_some_and(|p| app.canvas.agent_edge.is_some_and(|r| r.contains(p)) || app.canvas.chip_chevron.is_some_and(|r| r.contains(p)))
    {
        CursorIcon::PointingHand
    } else {
        cursor
    };
    if resp.contains_pointer() {
        ctx.set_cursor_icon(cursor);
    }

    // ---- overlays
    let painter = ui.painter_at(rect);
    draw_overlays(app, &painter, path_mode, zoom);
    if !path_mode {
        crate::table_ui::overlay(app, ui, &painter);
    }
    if let Some(e) = pick_hover.as_ref().and_then(|id| app.editor.doc.get(id)) {
        let mut q = outline_screen(app, e);
        q.push(q[0]);
        painter.add(Shape::line(q, Stroke::new(2.0, style::accent())));
    }
    rename_overlay(app, ui);
    text_overlay(app, &ctx);
}

/// In-place rename field over a frame's chip.
fn rename_overlay(app: &mut ZebraApp, ui: &mut Ui) {
    let Some((id, mut text, focused)) = app.canvas.renaming.take() else { return };
    let Some(e) = app.editor.doc.get(&id) else { return };
    let tl = app.canvas.w2s(&app.editor, e.corners()[0]);
    let r = ERect::from_min_size(tl + egui::vec2(0.0, -24.0), egui::vec2(200.0, 22.0));
    let resp = ui.put(r, egui::TextEdit::singleline(&mut text).font(style::ui_font(11.0)).margin(egui::vec2(6.0, 3.0)).id_salt("frame-rename"));
    if !focused {
        resp.request_focus();
        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()))));
            st.store(ui.ctx(), resp.id);
        }
    }
    let esc = ui.input(|i| i.key_pressed(Key::Escape));
    if esc {
        return;
    }
    if resp.lost_focus() && focused {
        let current = crate::words::element_label(e, &app.editor.doc);
        if text.trim() != current {
            let patch = zebra_core::ElementPatch { name: Some(text.trim().to_string()), ..Default::default() };
            if app.editor.apply(Command::Update { id, patch }, H).is_err() {
                app.toast(crate::t!("canvas-toast-rename-failed"));
            }
        }
        return;
    }
    app.canvas.renaming = Some((id, text, true));
}

fn start_gesture(app: &mut ZebraApp, start: Pos2, space: bool, shift: bool) {
    let ed = &app.editor;
    let tool = ed.view.tool;
    let world = app.canvas.s2w(ed, start);
    // While a step waits for a frame, dragging looks around for one.
    if space || tool == Tool::Hand || (app.step_pick.is_some() && tool == Tool::Select) {
        app.canvas.gesture = Gesture::Pan;
        return;
    }
    if ed.view.mode == Mode::PathEdit {
        if let Some((step, was)) = badge_at(app, start).and_then(|s| Some((s, app.editor.doc.path.get(s)?.clone()))) {
            app.canvas.gesture = Gesture::Badge { step, was: Box::new(was) };
            return;
        }
        app.canvas.gesture = Gesture::Pan;
        return;
    }
    match tool {
        Tool::Frame | Tool::Rectangle | Tool::Ellipse | Tool::Triangle | Tool::Line | Tool::Arrow => {
            app.canvas.gesture = Gesture::Create { tool, start: world };
            return;
        }
        Tool::Pen | Tool::Highlighter => {
            app.canvas.gesture = Gesture::Ink { points: vec![world] };
            return;
        }
        Tool::Text => {
            app.canvas.gesture = Gesture::Marquee { start, additive: false };
            return;
        }
        _ => {}
    }
    // Select tool: connector end handles first.
    if let Some(id) = sole_connector(app) {
        if let Some(ends) = connector_ends(app, &id) {
            for (i, w) in ends.iter().enumerate() {
                if app.canvas.w2s(&app.editor, *w).distance(start) < 9.0 && app.editor.begin(crate::t!("canvas-undo-move-connector-end"), H).is_ok() {
                    app.canvas.gesture = Gesture::ConnectorEnd { id, to_end: i == 1 };
                    return;
                }
            }
        }
    }
    // Then the transform handles.
    let sel = app.selection();
    if let Some(sb) = selection_box(app, &sel).filter(|_| sole_connector(app).is_none() && !crate::table_ui::active(app)) {
        match hit_handle(&sb, start) {
            Some(Handle::Rotate) => {
                let ids = app.editable_selection();
                if ids.is_empty() {
                    return;
                }
                let d = world - sb.center;
                let _ = app.editor.begin(crate::t!("canvas-undo-rotate"), H);
                app.canvas.gesture = Gesture::Rotate {
                    ids: moving_ids(app, &ids),
                    center: sb.center,
                    start_angle: d.y.atan2(d.x).to_degrees(),
                    applied: 0.0,
                    base_rot: sb.rotation,
                };
                return;
            }
            Some(Handle::Corner(i)) => {
                let ids = app.editable_selection();
                if ids.is_empty() {
                    return;
                }
                let anchor = sb.world[(i + 2) % 4];
                let _ = app.editor.begin(crate::t!("canvas-undo-resize"), H);
                app.canvas.gesture = Gesture::Scale {
                    ids: moving_ids(app, &ids),
                    anchor,
                    start_dist: (world - anchor).len().max(1e-9),
                    applied: 1.0,
                };
                return;
            }
            Some(Handle::Width(right)) => {
                let Some(id) = sb.text.clone() else { return };
                let Some(e) = app.editor.doc.get(&id) else { return };
                let anchor = e.corners()[if right { 0 } else { 1 }];
                if app.editor.begin(crate::t!("canvas-undo-text-width"), H).is_ok() {
                    app.canvas.gesture = Gesture::TextWidth { id, right, anchor };
                }
                return;
            }
            Some(Handle::Body) if hit(app, start, false).is_none_or(|h| sel.contains(&h)) => {
                if let Some(id) = hit(app, start, false).filter(|_| app.canvas.alt_press) {
                    option_frame(app, &id);
                }
                begin_move(app, world);
                return;
            }
            _ => {}
        }
    }
    match hit(app, start, false) {
        Some(id) => {
            if app.canvas.alt_press && option_frame(app, &id) {
                // Option-drag on a frame: the frame on its own.
            } else if !sel.contains(&id) {
                if shift {
                    let _ = app.editor.view(ViewCommand::AddToSelection { ids: vec![id] }, H);
                } else {
                    app.select(vec![id]);
                }
            }
            begin_move(app, world);
        }
        None => app.canvas.gesture = Gesture::Marquee { start, additive: shift },
    }
}

/// The fixed set of elements a drag moves, worked out once when it starts
/// (so a frame doesn't pick things up or drop them on the way): the
/// selection and what its frames hold, not anything locked; a frame
/// selected on its own moves alone.
fn moving_ids(app: &ZebraApp, ids: &[ElementId]) -> Vec<ElementId> {
    if app.editor.frame_only().is_some_and(|f| ids == std::slice::from_ref(f)) {
        return ids.to_vec();
    }
    app.editor.carried(ids)
}

/// Select frame `id` on its own (Option-click, Select Frame Only): moving,
/// resizing or turning it leaves what it holds where it is.
pub fn select_frame_only(app: &mut ZebraApp, id: ElementId) {
    app.view_cmd(ViewCommand::Select { ids: vec![id], frame_only: true });
}

/// An Option-click or Option-drag on `id`: a frame is taken on its own.
/// False (nothing done) for anything else.
fn option_frame(app: &mut ZebraApp, id: &ElementId) -> bool {
    if !app.editor.doc.get(id).is_some_and(|e| e.is_frame()) {
        return false;
    }
    if app.editor.frame_only() != Some(id) {
        select_frame_only(app, id.clone());
    }
    true
}

fn begin_move(app: &mut ZebraApp, origin: Vec2) {
    let sel = app.editable_selection();
    if sel.is_empty() {
        return;
    }
    let ids = moving_ids(app, &sel);
    let start_bounds = app.editor.bounds_of(&sel);
    if app.editor.begin(crate::t!("canvas-undo-move"), H).is_err() {
        return;
    }
    let moving = ids.iter().cloned().collect();
    app.canvas.gesture = Gesture::Move { ids, origin, applied: Vec2::ZERO, start_bounds, moving };
}

fn update_gesture(app: &mut ZebraApp, p: Pos2, delta: egui::Vec2, shift: bool, alt: bool) {
    let world = app.canvas.s2w(&app.editor, p);
    let zoom = app.editor.view.camera.zoom;
    let mut g = std::mem::take(&mut app.canvas.gesture);
    match &mut g {
        Gesture::Pan => {
            let _ = app.editor.view(ViewCommand::Pan { dx: delta.x as f64, dy: delta.y as f64 }, H);
            if app.editor.view.follow_agent {
                let _ = app.editor.view(ViewCommand::FollowAgent { on: false }, H);
            }
        }
        Gesture::Move { ids, origin, applied, start_bounds, moving } => {
            let raw = world - *origin;
            let (d, guides) = if alt || !app.snapping { (raw, vec![]) } else { snap_move(app, moving, start_bounds, raw, 6.0 / zoom) };
            app.canvas.guides = guides;
            let inc = d - *applied;
            if inc.x != 0.0 || inc.y != 0.0 {
                let _ = app.editor.apply(Command::Move { ids: ids.clone(), dx: inc.x, dy: inc.y, with_contents: false }, H);
                *applied = d;
            }
        }
        Gesture::Scale { ids, anchor, start_dist, applied } => {
            let total = ((world - *anchor).len() / *start_dist).max(0.02);
            let f = total / *applied;
            if (f - 1.0).abs() > 1e-9 {
                let r = app.editor.apply(Command::Scale { ids: ids.clone(), factor: f, about: Some([anchor.x, anchor.y]), with_contents: false }, H);
                if r.is_ok() {
                    *applied = total;
                }
            }
        }
        Gesture::Rotate { ids, center, start_angle, applied, base_rot } => {
            let d = world - *center;
            let mut total = d.y.atan2(d.x).to_degrees() - *start_angle;
            if shift {
                let target = ((*base_rot + total) / 15.0).round() * 15.0;
                total = target - *base_rot;
            }
            let inc = total - *applied;
            if inc.abs() > 1e-9 {
                let _ = app.editor.apply(Command::Rotate { ids: ids.clone(), degrees: inc, about: Some([center.x, center.y]), with_contents: false }, H);
                *applied = total;
            }
        }
        Gesture::Ink { points } => {
            if points.last().is_none_or(|l| (*l - world).len() * zoom > 1.5) {
                points.push(world);
            }
        }
        Gesture::TextWidth { id, right, anchor } => {
            drag_text_width(app, id, *right, *anchor, world);
        }
        Gesture::ConnectorEnd { id, to_end } => {
            let spec = zebra_core::command::EndSpec { element: None, x: world.x, y: world.y };
            let patch = if *to_end { ElementPatch { to: Some(spec), ..Default::default() } } else { ElementPatch { from: Some(spec), ..Default::default() } };
            let _ = app.editor.apply(Command::Update { id: id.clone(), patch }, H);
        }
        _ => {}
    }
    app.canvas.gesture = g;
}

fn end_gesture(app: &mut ZebraApp, p: Pos2, shift: bool) {
    let g = std::mem::take(&mut app.canvas.gesture);
    app.canvas.guides.clear();
    let world = app.canvas.s2w(&app.editor, p);
    let zoom = app.editor.view.camera.zoom;
    match g {
        Gesture::Move { .. } | Gesture::Scale { .. } | Gesture::Rotate { .. } | Gesture::TextWidth { .. } => {
            app.editor.commit_as(&crate::app::HUMAN);
        }
        Gesture::Marquee { start, additive } => {
            let a = app.canvas.s2w(&app.editor, start);
            let r = Rect::from_points([a, world]).unwrap();
            let hidden: Vec<ElementId> = app
                .editor
                .elements_in(r)
                .into_iter()
                .filter(|e| r.contains_rect(&e.bounds()))
                .map(|e| e.id.clone())
                .collect();
            let ids = app.top_level(&hidden);
            if additive {
                let _ = app.editor.view(ViewCommand::AddToSelection { ids }, H);
            } else {
                app.select(ids);
            }
        }
        Gesture::Create { tool, start } => create_shape(app, tool, start, world, shift),
        Gesture::Ink { points } => {
            if points.len() >= 2 {
                let tool = app.editor.view.tool;
                let hl = tool == Tool::Highlighter;
                let pts = points.iter().map(|p| [p.x, p.y]).collect();
                let w = if hl { 18.0 } else { 4.0 } / zoom;
                app.apply(Command::AddInk { points: pts, color: "theme:accent".into(), width: w, highlighter: hl, id: None });
            }
        }
        Gesture::Badge { step, was } => {
            // The path changed under the drag: this number is another step now.
            if app.editor.doc.path.get(step) != Some(&*was) {
                return;
            }
            if let Some(id) = hit(app, p, true) {
                app.apply(Command::UpdateStep { index: step, target: Some(StepTarget::Element { id }), builds: None, notes: None, duration: None, dwell: None, hide_builds: None });
                app.toast(crate::t!("canvas-toast-step-retargeted", step = ((step + 1) as u64)));
            }
        }
        Gesture::ConnectorEnd { id, to_end } => {
            if let Some(t) = attach_target(app, world, Some(&id)) {
                let spec = zebra_core::command::EndSpec { element: Some(t), x: world.x, y: world.y };
                let patch = if to_end { ElementPatch { to: Some(spec), ..Default::default() } } else { ElementPatch { from: Some(spec), ..Default::default() } };
                let _ = app.editor.apply(Command::Update { id, patch }, H);
            }
            app.editor.commit_as(&crate::app::HUMAN);
        }
        Gesture::Pan | Gesture::None => {}
    }
}

/// A text box's width handle dragged to `world`: the box wraps at the new
/// width (in its own units, so rotation and scale count) and keeps the
/// opposite top corner `anchor` where it was; its height follows the
/// words. Call inside a transaction (the drag is one undo step).
pub fn drag_text_width(app: &mut ZebraApp, id: &str, right: bool, anchor: Vec2, world: Vec2) {
    let Some(e) = app.editor.doc.get(id) else { return };
    let ElementKind::Text(t) = &e.kind else { return };
    let tr = e.transform;
    let (a, p) = (tr.inverse(anchor), tr.inverse(world));
    // About one glyph at the narrowest.
    let min = app.editor.theme().text_style(t.style).size * 0.6;
    let w = if right { p.x - a.x } else { a.x - p.x }.max(min);
    if t.fixed_width && (w - e.size.w).abs() < 1e-9 {
        return;
    }
    if app.editor.apply(Command::Resize { id: id.to_string(), width: Some(w * tr.scale), height: None }, H).is_err() {
        return;
    }
    let Some(e) = app.editor.doc.get(id) else { return };
    let (hw, hh) = (e.size.w / 2.0, e.size.h / 2.0);
    let corner = Vec2::new(if right { -hw } else { hw }, -hh);
    let c = anchor - (corner * tr.scale).rotate_deg(tr.rotation);
    let _ = app.editor.apply(Command::Update { id: id.to_string(), patch: ElementPatch { x: Some(c.x), y: Some(c.y), ..Default::default() } }, H);
}

/// Size on screen of a frame placed with a click.
fn frame_size(style: FrameStyle) -> (f64, f64) {
    match style {
        FrameStyle::Circle => (360.0, 360.0),
        _ => (480.0, 320.0),
    }
}

/// Insert › New Frame: a frame of the current style in the nearest empty
/// space, beside the selection or else in the middle of the view.
pub fn insert_frame(app: &mut ZebraApp) {
    let cam = app.editor.view.target_camera();
    let style = app.editor.view.frame_style;
    let (w, h) = frame_size(style);
    let s = 1.0 / cam.zoom;
    let (bw, bh) = zebra_core::geom::rotated_extent(w * s, h * s, cam.rotation);
    let sel = app.selection();
    let at = if sel.is_empty() {
        app.canvas_center_world()
    } else {
        let b = app.editor.bounds_of(&sel);
        Vec2::new(b.x1 + bw / 2.0 + bw.min(bh) * 0.15, b.center().y)
    };
    let at = app.editor.doc.free_spot(at, bw, bh);
    let el = NewElement { id: None, x: at.x, y: at.y, scale: s, rotation: cam.rotation, w: Some(w), h: Some(h), name: String::new(), kind: ElementKind::Frame(FrameData { style, color: None, slide: false }) };
    // A step waiting for a frame takes this one (one undo step for both).
    if let Some(out) = crate::sidebar::add_frame_for_pick(app, Command::AddElement { element: el }) {
        app.select(out.created);
        reveal_selection(app);
    }
}

fn create_shape(app: &mut ZebraApp, tool: Tool, a: Vec2, b: Vec2, shift: bool) {
    let zoom = app.editor.view.camera.zoom;
    let rot = app.editor.view.camera.rotation;
    // Work in screen-aligned space so shapes drawn on a rotated view stay upright on screen.
    let la = a.rotate_deg(-rot);
    let lb = b.rotate_deg(-rot);
    let mut w = (lb.x - la.x).abs() * zoom;
    let mut h = (lb.y - la.y).abs() * zoom;
    let dragged = w > 4.0 || h > 4.0;
    let mut center = (a + b) * 0.5;
    let line = matches!(tool, Tool::Line | Tool::Arrow);
    if !dragged {
        let (dw, dh) = match tool {
            Tool::Frame => frame_size(app.editor.view.frame_style),
            Tool::Ellipse => (140.0, 140.0),
            Tool::Line | Tool::Arrow => (160.0, 0.0),
            _ => (160.0, 110.0),
        };
        w = dw;
        h = dh;
        center = a;
    } else if shift && !line {
        let m = w.max(h);
        w = m;
        h = m;
    }
    let s = 1.0 / zoom;
    let (kind, rotation, w, h) = match tool {
        Tool::Frame => {
            let style = app.editor.view.frame_style;
            let (w, h) = if style == FrameStyle::Circle { (w.max(h), w.max(h)) } else { (w, h) };
            (ElementKind::Frame(FrameData { style, color: None, slide: false }), rot, w, h)
        }
        Tool::Line | Tool::Arrow if dragged && (attach_target(app, a, None).is_some() || attach_target(app, b, None).is_some()) => {
            // A line drawn from or to an object sticks to it.
            let end = |w: Vec2| zebra_core::command::EndSpec { element: attach_target(app, w, None), x: w.x, y: w.y };
            let (from, to) = (end(a), end(b));
            let head = if tool == Tool::Arrow { zebra_core::Arrowhead::Arrow } else { zebra_core::Arrowhead::None };
            let cmd = Command::AddConnector {
                from,
                to,
                route: zebra_core::Route::Straight,
                start: zebra_core::Arrowhead::None,
                end: head,
                color: Some("theme:shape1".into()),
                width: 3.0 / zoom,
                label: String::new(),
                id: None,
            };
            if let Some(out) = app.apply(cmd) {
                app.select(out.created);
            }
            let _ = app.editor.view(ViewCommand::SetTool { tool: Tool::Select }, H);
            return;
        }
        Tool::Line | Tool::Arrow => {
            let (len, ang) = if dragged {
                let d = b - a;
                (d.len() * zoom, d.y.atan2(d.x).to_degrees())
            } else {
                (w, rot)
            };
            let shape = if tool == Tool::Arrow { ShapeKind::Arrow } else { ShapeKind::Line };
            let ang = if shift { (ang / 15.0).round() * 15.0 } else { ang };
            (ElementKind::Shape(ShapeData { shape, fill: None, stroke: Some("theme:shape1".into()), stroke_width: 3.0, ..Default::default() }), ang, len.max(8.0), 24.0)
        }
        _ => {
            let shape = match tool {
                Tool::Ellipse => ShapeKind::Ellipse,
                Tool::Triangle => ShapeKind::Triangle,
                _ => ShapeKind::Rectangle,
            };
            (ElementKind::Shape(ShapeData { shape, fill: Some("theme:shape1".into()), stroke: None, stroke_width: 3.0, ..Default::default() }), rot, w, h)
        }
    };
    let is_frame = matches!(kind, ElementKind::Frame(_));
    // A clicked frame that would cover other objects moves to the nearest
    // empty space; a drawn one stays where it was drawn.
    if is_frame && !dragged {
        let (bw, bh) = zebra_core::geom::rotated_extent(w * s, h * s, rotation);
        center = app.editor.doc.free_spot(center, bw, bh);
    }
    let el = NewElement { id: None, x: center.x, y: center.y, scale: s, rotation, w: Some(w), h: Some(h), name: String::new(), kind };
    let cmd = Command::AddElement { element: el };
    // A drawn frame fills a step waiting for one (one undo step for both).
    let out = if is_frame { crate::sidebar::add_frame_for_pick(app, cmd) } else { app.apply(cmd) };
    if let Some(out) = out {
        app.select(out.created.clone());
        if is_frame {
            let n = out.created.first().map(|id| app.editor.frame_contents(id).len()).unwrap_or(0);
            if n > 0 {
                app.toast(crate::t!("canvas-toast-framed-objects", count = n));
            }
        }
    }
    let _ = app.editor.view(ViewCommand::SetTool { tool: Tool::Select }, H);
}

/// The frame a click at `p` gives a step waiting for one: the frame
/// clicked, or the one around the object clicked.
fn pick_target(app: &ZebraApp, p: Pos2) -> Option<ElementId> {
    if let Some((r, id)) = &app.canvas.frame_chip {
        if r.contains(p) {
            return Some(id.clone());
        }
    }
    let id = hit(app, p, true)?;
    if app.editor.doc.get(&id)?.is_frame() {
        return Some(id);
    }
    app.editor.parent_frame(&id)
}

/// A click while a step waits for a frame. True when it was used: a frame
/// fills the step; anything else (empty canvas) ends the insert. With the
/// frame tool the click draws a frame, which fills the step as it is made.
fn pick_click(app: &mut ZebraApp, p: Pos2) -> bool {
    match app.editor.view.tool {
        Tool::Frame => false,
        Tool::Select | Tool::Hand => {
            match pick_target(app, p) {
                Some(id) => {
                    crate::sidebar::fill_pick(app, id);
                }
                None => crate::sidebar::cancel_pick(app),
            }
            true
        }
        _ => {
            crate::sidebar::cancel_pick(app);
            false
        }
    }
}

fn click(app: &mut ZebraApp, p: Pos2, shift: bool, alt: bool) {
    let mode = app.editor.view.mode;
    let tool = app.editor.view.tool;
    if mode == Mode::PathEdit {
        if let Some(step) = badge_at(app, p) {
            app.selected_step = Some(step);
            let _ = app.editor.view(ViewCommand::GotoStep { step }, H);
            return;
        }
        if let Some(id) = hit(app, p, true) {
            // Only when the step was added: a refused one used to select
            // the last step and say "added" all the same.
            if app.apply(Command::AddStep { target: StepTarget::Element { id }, index: None, builds: vec![], notes: String::new() }).is_some() {
                let n = app.editor.doc.path.len();
                app.selected_step = n.checked_sub(1);
                app.toast(crate::t!("canvas-toast-added-step", step = n));
            }
        }
        return;
    }
    match tool {
        Tool::Text => {
            begin_new_text(app, p);
            let _ = app.editor.view(ViewCommand::SetTool { tool: Tool::Select }, H);
            return;
        }
        Tool::Frame | Tool::Rectangle | Tool::Ellipse | Tool::Triangle | Tool::Line | Tool::Arrow => {
            let w = app.canvas.s2w(&app.editor, p);
            create_shape(app, tool, w, w, false);
            return;
        }
        Tool::Hand | Tool::Pen | Tool::Highlighter => return,
        _ => {}
    }
    match hit(app, p, false) {
        Some(id) if alt && option_frame(app, &id) => {}
        Some(id) if shift => {
            let mut s = app.selection();
            if s.contains(&id) {
                // A grouped element leaves the selection with its group.
                let group = app.editor.doc.expand_groups(std::slice::from_ref(&id));
                s.retain(|x| !group.contains(x));
            } else {
                s.push(id);
            }
            app.select(s);
        }
        Some(id) => app.select(vec![id]),
        None if !shift => {
            let _ = app.editor.view(ViewCommand::ClearSelection, H);
        }
        None => {}
    }
}

fn double_click(app: &mut ZebraApp, p: Pos2) {
    if app.editor.view.mode == Mode::PathEdit || app.canvas.text.is_some() {
        return;
    }
    match hit(app, p, false) {
        Some(id) => match app.editor.doc.get(&id).map(|e| &e.kind) {
            Some(ElementKind::Text(_)) => begin_edit_existing(app, id),
            Some(ElementKind::Image(_)) => crate::media_ui::begin_crop(app, &id),
            Some(ElementKind::Video(_)) => crate::media_ui::toggle(app, &id),
            Some(ElementKind::Shape(s)) if !s.shape.is_line() => begin_text_on_shape(app, id),
            Some(_) => {
                let _ = app.editor.view(ViewCommand::ZoomTo { id, animate: true }, H);
            }
            None => {}
        },
        None => begin_new_text(app, p),
    }
}

// ----------------------------------------------------------------------
// Text editing (in place, over the canvas)

pub fn begin_new_text(app: &mut ZebraApp, p: Pos2) {
    commit_text_edit(app);
    let zoom = app.editor.view.camera.zoom;
    let theme = app.editor.theme();
    let size = theme.text_style(TextStyle::Body).size;
    // Box top-left so the first line is vertically centred on the click.
    let tl = app.canvas.s2w(&app.editor, p - egui::vec2(2.0, (size * 0.65) as f32));
    app.canvas.text = Some(TextEditState {
        id: None,
        anchor: tl,
        buf: String::new(),
        original: String::new(),
        style: TextStyle::Body,
        bold: false,
        italic: false,
        bullets: false,
        scale: 1.0 / zoom,
        focused_once: false,
        size_px: egui::Vec2::ZERO,
        zoom,
        rich: TextData::default(),
        sel: (0, 0),
        link_draft: None,
        wrap: None,
        host: None,
        spell: Default::default(),
        spell_word: None,
        caret_seen: None,
        caret_follow: false,
        width_drag: None,
        drag: None,
        pending_sel: None,
        sel_menu: false,
        on_words: false,
        laid_out: None,
    });
    let _ = app.editor.view(ViewCommand::ClearSelection, H);
}

/// Double-click on a shape: type on it. If the shape already carries a
/// label (a text grouped with it, centred inside), edit that instead.
pub fn begin_text_on_shape(app: &mut ZebraApp, shape: ElementId) {
    let Some(e) = app.editor.doc.get(&shape).cloned() else { return };
    if let Some(label) = shape_label(app, &e) {
        begin_edit_existing(app, label);
        return;
    }
    if e.locked {
        app.toast(crate::i18n::with_shortcut(crate::t!("canvas-toast-shape-locked"), &crate::actions::Action::ToggleLock));
        return;
    }
    commit_text_edit(app);
    let zoom = app.editor.view.camera.zoom;
    let body = app.editor.theme().text_style(TextStyle::Body).size;
    let (w, h) = (e.size.w * e.transform.scale, e.size.h * e.transform.scale);
    // Text that reads well at this zoom, but never taller than about a
    // third of the shape.
    let scale = (1.0 / zoom).min(h / (body * 3.0)).max(1e-12);
    let wrap = w * 0.8 / scale;
    let c = e.transform.center();
    let fill = match &e.kind {
        ElementKind::Shape(sd) => sd.fill.clone(),
        _ => None,
    };
    let mut rich = TextData { align: zebra_core::Align::Center, color: app.editor.theme().text_on(fill.as_ref()), ..Default::default() };
    rich.fixed_width = true;
    app.canvas.text = Some(TextEditState {
        id: None,
        anchor: Vec2::new(c.x - wrap * scale / 2.0, c.y - body * scale * 0.65),
        buf: String::new(),
        original: String::new(),
        style: TextStyle::Body,
        bold: false,
        italic: false,
        bullets: false,
        scale,
        focused_once: false,
        size_px: egui::Vec2::ZERO,
        zoom,
        rich,
        sel: (0, 0),
        link_draft: None,
        wrap: Some(wrap),
        host: Some(shape.clone()),
        spell: Default::default(),
        spell_word: None,
        caret_seen: None,
        caret_follow: false,
        width_drag: None,
        drag: None,
        pending_sel: None,
        sel_menu: false,
        on_words: false,
        laid_out: None,
    });
    app.select(vec![shape]);
}

/// The text grouped with `shape` whose centre lies inside it, if any.
fn shape_label(app: &ZebraApp, shape: &zebra_core::Element) -> Option<ElementId> {
    let g = shape.group.as_deref()?;
    let b = shape.bounds();
    app.editor
        .doc
        .ordered()
        .filter(|o| o.group.as_deref() == Some(g) && matches!(o.kind, ElementKind::Text(_)))
        .find(|o| b.contains(o.transform.center()))
        .map(|o| o.id.clone())
}

pub fn begin_edit_existing(app: &mut ZebraApp, id: ElementId) {
    commit_text_edit(app);
    let Some(e) = app.editor.doc.get(&id) else { return };
    let ElementKind::Text(t) = &e.kind else { return };
    if e.locked {
        app.toast(crate::i18n::with_shortcut(crate::t!("canvas-toast-text-locked"), &crate::actions::Action::ToggleLock));
        return;
    }
    app.canvas.text = Some(TextEditState {
        id: Some(id.clone()),
        anchor: e.corners()[0],
        buf: t.text.clone(),
        original: t.text.clone(),
        style: t.style,
        bold: t.bold,
        italic: t.italic,
        bullets: t.bullets,
        scale: e.transform.scale,
        focused_once: false,
        size_px: egui::Vec2::ZERO,
        zoom: app.editor.view.camera.zoom,
        rich: t.clone(),
        sel: (0, 0),
        link_draft: None,
        wrap: t.fixed_width.then_some(e.size.w),
        host: None,
        spell: Default::default(),
        spell_word: None,
        caret_seen: None,
        caret_follow: false,
        width_drag: None,
        drag: None,
        pending_sel: None,
        sel_menu: false,
        on_words: false,
        laid_out: None,
    });
    app.select(vec![id]);
}

/// Cancel without applying (used by undo while editing).
pub fn cancel_text_edit(app: &mut ZebraApp) {
    app.canvas.text = None;
}

/// The text box being edited may be gone (an agent deleted it, or undid
/// adding it). What was typed is not thrown away with it: the edit goes on
/// as a new text box in the same place. Returns false when there is
/// nothing to keep (the text was not changed), which ends the edit.
fn keep_orphan(app: &mut ZebraApp, t: &mut TextEditState) -> bool {
    let Some(id) = &t.id else { return true };
    if matches!(app.editor.doc.get(id).map(|e| &e.kind), Some(ElementKind::Text(_))) {
        return true;
    }
    if t.buf.trim().is_empty() || t.buf.trim_end() == t.original.trim_end() {
        return false;
    }
    t.id = None;
    t.host = None;
    app.toast(crate::t!("canvas-toast-text-kept"));
    true
}

/// Apply the in-place edit as one command (Add text / Edit text). Returns
/// the text box the edit left on the canvas, if any.
pub fn commit_text_edit(app: &mut ZebraApp) -> Option<ElementId> {
    let mut t = app.canvas.text.take()?;
    // A width drag still open ends with the edit, as its own undo step.
    if t.width_drag.take().is_some() && t.id.is_some() {
        app.editor.commit_as(&crate::app::HUMAN);
    }
    if !keep_orphan(app, &mut t) {
        return None;
    }
    let text = t.buf.trim_end().to_string();
    zebra_core::rich::edit_text(&mut t.rich, &text);
    match &t.id {
        None => {
            if text.trim().is_empty() {
                return None;
            }
            // Centre of the box as laid out on screen.
            let half = Vec2::new(t.size_px.x as f64, t.size_px.y as f64) * (0.5 / t.zoom);
            let mut c = t.anchor + half.rotate_deg(app.editor.view.camera.rotation);
            let mut rotation = app.editor.view.camera.rotation;
            let host = t.host.clone().and_then(|h| app.editor.doc.get(&h).cloned());
            let mut id = None;
            if let Some(h) = &host {
                c = h.transform.center();
                rotation = h.transform.rotation;
                id = (1..).map(|n| format!("t{n}")).find(|i| !app.editor.doc.elements.contains_key(i));
            }
            let el = NewElement {
                id: id.clone(),
                x: c.x,
                y: c.y,
                scale: t.scale,
                rotation,
                w: t.wrap,
                h: None,
                name: String::new(),
                kind: ElementKind::Text(TextData {
                    text,
                    style: t.style,
                    bold: t.bold,
                    italic: t.italic,
                    bullets: t.bullets,
                    spans: t.rich.spans.clone(),
                    paras: t.rich.paras.clone(),
                    align: t.rich.align,
                    color: t.rich.color.clone(),
                    ..Default::default()
                }),
            };
            let mut cmds = vec![Command::AddElement { element: el }];
            if let (Some(h), Some(id)) = (&host, id) {
                cmds.push(Command::Group { ids: vec![h.id.clone(), id], id: None });
            }
            let r = app.editor.apply_batch(cmds, Some(crate::t!("canvas-undo-add-text")), H);
            let out = app.report(r)?;
            let made = out.created.iter().find(|c| matches!(app.editor.doc.get(c).map(|e| &e.kind), Some(ElementKind::Text(_)))).cloned();
            app.select(out.created);
            made
        }
        Some(id) => {
            let Some(ElementKind::Text(old)) = app.editor.doc.get(id).map(|e| e.kind.clone()) else { return None };
            if text.trim().is_empty() {
                app.apply(Command::Delete { ids: vec![id.clone()] });
                return None;
            }
            let mut patch = ElementPatch::default();
            if text != old.text {
                patch.text = Some(text);
            }
            if t.style != old.style {
                patch.style = Some(t.style);
            }
            if t.bold != old.bold {
                patch.bold = Some(t.bold);
            }
            if t.italic != old.italic {
                patch.italic = Some(t.italic);
            }
            if t.bullets != old.bullets {
                patch.bullets = Some(t.bullets);
            }
            if t.rich.spans != old.spans || patch.text.is_some() {
                patch.spans = Some(t.rich.spans.clone());
            }
            if t.rich.paras != old.paras || patch.text.is_some() {
                patch.paras = Some(t.rich.paras.clone());
            }
            if patch != ElementPatch::default() {
                let r = app.editor.apply_batch(vec![Command::Update { id: id.clone(), patch }], Some(crate::t!("canvas-undo-edit-text")), H);
                app.report(r);
            }
            Some(id.clone())
        }
    }
}

/// Build the editing view of a rich text: one egui section per run of
/// equal formatting, paragraph indents as leading space.
fn rich_job(rich: &TextData, text: &str, theme: &zebra_core::Theme, base: &FontId, font_name: &str, weight: u16, color: Color32, wrap: f32) -> egui::text::LayoutJob {
    use zebra_core::rich;
    let mut r = rich.clone();
    rich::edit_text(&mut r, text);
    let mut job = egui::text::LayoutJob { wrap: egui::text::TextWrapping { max_width: wrap, ..Default::default() }, ..Default::default() };
    if wrap.is_finite() {
        job.halign = match rich.align {
            zebra_core::Align::Left => egui::Align::LEFT,
            zebra_core::Align::Center => egui::Align::Center,
            zebra_core::Align::Right => egui::Align::RIGHT,
        };
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    let byte = |i: usize| chars.get(i).map(|c| c.0).unwrap_or(text.len());
    let mut cuts: Vec<usize> = vec![0, n];
    for sp in &r.spans {
        cuts.push(sp.start.min(n));
        cuts.push(sp.end.min(n));
    }
    let mut para_starts = vec![0usize];
    for (i, (_, c)) in chars.iter().enumerate() {
        if *c == '\n' {
            para_starts.push(i + 1);
            cuts.push(i + 1);
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let size = base.size;
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        let f = r.spans.iter().find(|sp| sp.start <= a && b <= sp.end).map(|sp| sp.format.clone()).unwrap_or_default();
        let w = zebra_core::theme::bold_weight(weight, f.bold.or(rich.bold.then_some(true)));
        let italic = f.italic.unwrap_or(rich.italic);
        let k = f.size.unwrap_or(1.0) as f32;
        let c = match (&f.color, &f.link) {
            (Some(c), _) => crate::fallback::theme_color(theme, c),
            (None, Some(_)) => crate::fallback::theme_color(theme, "theme:accent"),
            _ => color,
        };
        let line = Stroke::new((size * k * 0.06).max(1.0), c);
        let mut fmt = egui::TextFormat {
            // Same cap as the box itself: egui rasterises glyphs at this size.
            font_id: FontId::new((size * k).clamp(1.0, 600.0), style::doc_family(font_name, w, italic)),
            color: c,
            ..Default::default()
        };
        if f.underline == Some(true) || (f.link.is_some() && f.underline != Some(false)) {
            fmt.underline = line;
        }
        if f.strike == Some(true) {
            fmt.strikethrough = line;
        }
        let lead = match para_starts.iter().position(|p| *p == a) {
            Some(pi) => {
                let (list, indent, _) = rich::para(&r, pi);
                indent as f32 * size * 1.25 + if list == ListKind::None { 0.0 } else { size * 1.2 }
            }
            None => 0.0,
        };
        job.append(&text[byte(a)..byte(b)], lead, fmt);
    }
    if job.sections.is_empty() {
        job.append("", 0.0, egui::TextFormat { font_id: base.clone(), color, ..Default::default() });
    }
    job
}

/// egui lays out an empty paragraph at x = 0, ignoring the indent the job
/// gave it, so a new empty list item put its caret (and so its bullet) in
/// the margin until something was typed. Move those rows to where the
/// paragraph's text will start.
fn indent_empty_list_rows(g: std::sync::Arc<egui::Galley>, rich: &TextData, text: &str, size: f32) -> std::sync::Arc<egui::Galley> {
    use zebra_core::rich;
    let mut r = rich.clone();
    rich::edit_text(&mut r, text);
    let mut shifts = vec![];
    let mut para = 0usize;
    for (i, row) in g.rows.iter().enumerate() {
        if row.row.glyphs.is_empty() {
            let (list, indent, align) = rich::para(&r, para);
            let lead = indent as f32 * size * 1.25 + if list == ListKind::None { 0.0 } else { size * 1.2 };
            if lead > 0.0 && align == zebra_core::Align::Left {
                shifts.push((i, lead));
            }
        }
        if row.ends_with_newline {
            para += 1;
        }
    }
    if shifts.is_empty() {
        return g;
    }
    let mut g2 = (*g).clone();
    for (i, lead) in shifts {
        g2.rows[i].pos.x += lead;
    }
    std::sync::Arc::new(g2)
}

/// Whether every selected character is `on` for a property.
fn all_on(r: &TextData, a: usize, b: usize, get: impl Fn(&SpanFormat) -> Option<bool>, default: bool) -> bool {
    (a..b).all(|i| r.spans.iter().find(|s| s.start <= i && i < s.end).and_then(|s| get(&s.format)).unwrap_or(default))
}

fn text_overlay(app: &mut ZebraApp, ctx: &egui::Context) {
    use zebra_core::rich;
    let Some(mut t) = app.canvas.text.take() else { return };
    // The element may have been deleted (e.g. by an agent).
    if !keep_orphan(app, &mut t) {
        return;
    }
    if let Some(e) = t.id.as_ref().and_then(|id| app.editor.doc.get(id)) {
        t.anchor = e.corners()[0];
        t.scale = e.transform.scale;
    }
    let theme = app.editor.theme();
    let ts = theme.text_style(t.style);
    let zoom = app.editor.view.camera.zoom;
    t.zoom = zoom;
    let size = (ts.size * t.scale * zoom) as f32;
    let font = FontId::new(size.clamp(4.0, 600.0), style::doc_family(&ts.font, zebra_core::theme::bold_weight(ts.weight, t.bold.then_some(true)), t.italic));
    let color = crate::fallback::theme_color(&theme, t.rich.color.as_deref().filter(|c| !c.is_empty()).unwrap_or(match t.style {
        TextStyle::Title => "theme:title",
        TextStyle::Subtitle => "theme:subtitle",
        TextStyle::Body => "theme:body",
    }));
    rich::edit_text(&mut t.rich, &t.buf);
    t.rich.bold = t.bold;
    t.rich.italic = t.italic;
    t.rich.bullets = t.bullets;
    let pos = app.canvas.w2s(&app.editor, t.anchor);
    let measure_text = if t.buf.is_empty() { crate::t!("canvas-text-placeholder") } else { t.buf.clone() + " " };
    let gw = {
        let job = rich_job(&t.rich, &measure_text, &theme, &font, &ts.font, ts.weight, color, f32::INFINITY);
        ctx.fonts_mut(|f| f.layout_job(job).size().x)
    };
    let width = match t.wrap {
        Some(w) => (w * t.scale * zoom) as f32,
        None => gw.max(size * 2.0),
    };
    let pos = match t.host.as_ref().and_then(|h| app.editor.doc.get(h)) {
        // Keep new text on a shape centred on it as lines are added.
        Some(h) if t.id.is_none() => {
            let c = h.transform.center();
            t.anchor = Vec2::new(c.x - width as f64 / zoom / 2.0, c.y - (t.size_px.y.max(size * 1.3)) as f64 / zoom / 2.0);
            app.canvas.w2s(&app.editor, t.anchor)
        }
        _ => pos,
    };
    let mut committed = false;
    let mut refocus = false;
    let edit_id = app.uid("zebra-text-edit-field");
    let focused = ctx.memory(|m| m.has_focus(edit_id));

    // Formatting actions, from the keyboard or the toolbar.
    #[derive(PartialEq)]
    enum Fx {
        Bold,
        Italic,
        Underline,
        Strike,
        Bigger,
        Smaller,
        Color(String),
        List(ListKind),
        Indent(i32),
        /// Backspace at the start of paragraph `n`, a list item or
        /// indented: drop its marker, else outdent it one level.
        Outdent(usize),
        Link,
    }
    let mut fx: Vec<Fx> = vec![];
    /// What Tab types inside a line: the canvas text has no tab stops.
    const TAB_SPACES: &str = "    ";
    // The caret, with nothing selected, at the very start of a list item
    // or an indented paragraph: that paragraph's number.
    let caret = egui::TextEdit::load_state(ctx, edit_id).and_then(|s| s.cursor.char_range()).filter(|c| c.primary.index == c.secondary.index).map(|c| usize::from(c.primary.index));
    let outdent_at = caret
        .filter(|&c| c == 0 || t.buf.chars().nth(c - 1) == Some('\n'))
        .map(|c| rich::paragraph_of(&t.rich, c))
        .filter(|&n| matches!(rich::para(&t.rich, n), (l, i, _) if l != ListKind::None || i > 0));
    // Tab with the caret inside a line of a plain paragraph types spaces
    // (Kartik, 20 Nov); at a paragraph's start, in a list or over a
    // selection it indents.
    let mid_line = caret.is_some_and(|c| c > 0 && t.buf.chars().nth(c - 1) != Some('\n') && rich::para(&t.rich, rich::paragraph_of(&t.rich, c)).0 == ListKind::None);
    if focused {
        ctx.input_mut(|i| {
            // Backspace there outdents, as in TextEdit and Pages, instead of
            // joining the paragraph to the one before.
            if let Some(n) = outdent_at {
                if i.consume_key(egui::Modifiers::NONE, Key::Backspace) {
                    fx.push(Fx::Outdent(n));
                }
            }
            if i.consume_key(egui::Modifiers::COMMAND, Key::B) {
                fx.push(Fx::Bold);
            }
            if i.consume_key(egui::Modifiers::COMMAND, Key::I) {
                fx.push(Fx::Italic);
            }
            if i.consume_key(egui::Modifiers::COMMAND, Key::U) {
                fx.push(Fx::Underline);
            }
            if i.consume_key(egui::Modifiers::COMMAND, Key::K) {
                fx.push(Fx::Link);
            }
            if i.consume_key(egui::Modifiers::SHIFT, Key::Tab) {
                fx.push(Fx::Indent(-1));
            } else if i.consume_key(egui::Modifiers::NONE, Key::Tab) {
                if mid_line {
                    // Typed like any other text, so the editor's own undo
                    // and span bookkeeping see it.
                    i.events.push(egui::Event::Text(TAB_SPACES.into()));
                } else {
                    fx.push(Fx::Indent(1));
                }
            }
        });
    }
    match app.canvas.text_menu.take() {
        Some(crate::actions::Action::Bold) => fx.push(Fx::Bold),
        Some(crate::actions::Action::Italic) => fx.push(Fx::Italic),
        Some(crate::actions::Action::Underline) => fx.push(Fx::Underline),
        _ => {}
    }

    let area = egui::Area::new(app.uid("zebra-text-edit")).fixed_pos(pos).order(egui::Order::Foreground).constrain(false);
    let app_rect_valid = app.canvas.rect_valid;
    let rich_now = t.rich.clone();
    let font_name = ts.font.clone();
    let style_weight = ts.weight;
    let theme2 = theme.clone();
    let font2 = font.clone();
    // The spelling menu belongs to this edit: a click in it must not end it.
    let spell_open = t.spell_word.is_some();
    let canvas_rect = app.canvas.rect;
    // Words being dragged stay in the box, dimmed, until the drop.
    let dim = t.drag.as_ref().filter(|d| d.moving && !d.cancelled).map(|d| d.sel);
    let wrap_px = t.wrap.map(|w| (w * t.scale * zoom) as f32);
    let r = area.show(ctx, |ui| {
        // The text stays inside the canvas: typed past its edge, it ran
        // over the inspector.
        if app_rect_valid {
            ui.set_clip_rect(canvas_rect);
        }
        // Areas otherwise remember last frame's (narrow) width, which made the
        // text wrap a few letters per line while typing.
        ui.set_min_width(width);
        let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = rich_job(&rich_now, buf.as_str(), &theme2, &font2, &font_name, style_weight, color, wrap);
            if dim.is_some() {
                crate::text_drag::fade(&mut job, dim, crate::text_drag::DIM_ALPHA);
            }
            let g = ui.fonts_mut(|f| f.layout_job(job));
            indent_empty_list_rows(g, &rich_now, buf.as_str(), font2.size)
        };
        let edit = egui::TextEdit::multiline(&mut t.buf)
            .font(font.clone())
            .text_color(color)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .desired_width(width)
            .desired_rows(1)
            .lock_focus(true)
            .layouter(&mut layouter)
            .hint_text(egui::RichText::new(crate::t!("canvas-text-placeholder")).color(color.gamma_multiply(0.35)).font(font.clone()))
            .id(edit_id);
        if !t.focused_once {
            // A new edit starts with no history: egui keeps a field's undo
            // steps under its id, and this one field edits every text, so
            // ⌘Z here brought back what was typed into the last one.
            ui.ctx().data_mut(|d| d.remove::<egui::text_edit::TextEditState>(edit_id));
            // Words just moved inside the box stay selected where they landed.
            if let Some((a, b)) = t.pending_sel.take() {
                let mut st = egui::text_edit::TextEditState::default();
                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(a), egui::text::CCursor::new(b))));
                st.store(ui.ctx(), edit_id);
                t.sel = (a, b);
            }
        }
        // egui keeps a field's selection only while the field has the keys:
        // shown without them it drops to a caret. A press on the selected
        // words, Esc during their drag and their menu all take the keys
        // away for a moment, so they are given back first.
        if !t.focused_once || t.drag.is_some() || t.sel_menu {
            ui.memory_mut(|m| m.request_focus(edit_id));
        }
        // The selected words' own widgets and their menu are part of the
        // field: a click on them is not a click elsewhere, which would take
        // the field's keys (and so its selection) away, as egui's own
        // completion popup arranges for itself.
        // Dragged words stay in the box, dimmed: not drawn as a selection.
        if dim.is_some() {
            let v = ui.visuals_mut();
            v.selection.bg_fill = Color32::TRANSPARENT;
            v.selection.stroke.color = color.gamma_multiply(crate::text_drag::DIM_ALPHA);
            // The only caret while dragging is the one where the words go.
            v.text_cursor.stroke.color = Color32::TRANSPARENT;
        }
        let surrender = ui.ctx().options(|o| o.input_options.surrender_focus_on);
        if t.on_words {
            ui.ctx().options_mut(|o| o.input_options.surrender_focus_on = egui::SurrenderFocusOn::Never);
        }
        let out = edit.show(ui);
        ui.ctx().options_mut(|o| o.input_options.surrender_focus_on = surrender);
        let resp = out.response.clone();
        if !t.focused_once {
            resp.request_focus();
            t.focused_once = true;
        }
        // List markers, drawn in the margin the indent left free.
        let mut r2 = t.rich.clone();
        rich::edit_text(&mut r2, &t.buf);
        let mut numbers = [0usize; 9];
        let mut at = 0usize;
        for (pi, p) in t.buf.split('\n').enumerate() {
            let (list, indent, _) = rich::para(&r2, pi);
            let lvl = indent.min(8) as usize;
            for n in numbers.iter_mut().skip(lvl + 1) {
                *n = 0;
            }
            let marker = match list {
                ListKind::Bullet => {
                    numbers[lvl] = 0;
                    ["\u{2022}", "\u{25E6}", "\u{25AA}"][lvl % 3].to_string()
                }
                ListKind::Number => {
                    numbers[lvl] += 1;
                    format!("{}.", numbers[lvl])
                }
                ListKind::None => {
                    numbers[lvl] = 0;
                    String::new()
                }
            };
            if !marker.is_empty() {
                let c = out.galley.pos_from_cursor(egui::text::CCursor::new(at));
                let p0 = out.galley_pos + egui::vec2(c.min.x - size * 1.2, c.center().y - out.galley_pos.y + 0.0);
                ui.painter().text(egui::pos2(p0.x, out.galley_pos.y + c.center().y), Align2::LEFT_CENTER, marker, font.clone(), color);
            }
            at += p.chars().count() + 1;
        }
        // Spelling: a red dotted line under each misspelled word, except
        // the one being typed (the cursor is at its end).
        let caret = t.sel.0 == t.sel.1;
        let bad: Vec<std::ops::Range<usize>> = crate::spell::misspelled(&t.buf, &mut t.spell).iter().filter(|r| !(caret && r.end == t.sel.1)).cloned().collect();
        let red = Color32::from_rgb(0xff, 0x3b, 0x30);
        let dot = (size * 0.07).clamp(1.0, 3.0);
        for r in &bad {
            // One run per row the word sits on.
            let mut i = r.start;
            while i < r.end {
                let a = out.galley.pos_from_cursor(egui::text::CCursor::new(i));
                let mut j = i + 1;
                while j < r.end && (out.galley.pos_from_cursor(egui::text::CCursor::new(j)).min.y - a.min.y).abs() < 0.5 {
                    j += 1;
                }
                let b = out.galley.pos_from_cursor(egui::text::CCursor::new(j));
                let y = out.galley_pos.y + a.max.y + dot;
                let x1 = if (b.min.y - a.min.y).abs() < 0.5 { b.min.x } else { a.min.x + (j - i) as f32 * size * 0.5 };
                let mut x = out.galley_pos.x + a.min.x;
                while x < out.galley_pos.x + x1 {
                    ui.painter().circle_filled(egui::pos2(x + dot * 0.5, y), dot * 0.5, red);
                    x += dot * 2.0;
                }
                i = j;
            }
        }
        // Right-click a marked word: its suggestions, Learn and Ignore.
        if resp.secondary_clicked() {
            t.spell_word = resp.interact_pointer_pos().and_then(|p| {
                let c = usize::from(out.galley.cursor_from_pos(p - out.galley_pos).index);
                bad.iter().find(|r| r.start <= c && c <= r.end).cloned()
            });
        }
        if let Some(word) = t.spell_word.clone() {
            let mut keep = false;
            egui::Popup::context_menu(&resp).style(crate::widgets::menu_style).width(200.0).show(|ui| {
                keep = true;
                let text: String = t.buf.chars().skip(word.start).take(word.end - word.start).collect();
                let guesses = crate::spell::guesses(&t.buf, word.clone());
                if guesses.is_empty() {
                    crate::widgets::menu_row(ui, None, &crate::t!("spell-no-guesses"), None, false);
                }
                for g in guesses {
                    if crate::widgets::menu_item(ui, &g, None).clicked() {
                        let b0 = t.buf.char_indices().nth(word.start).map_or(t.buf.len(), |(b, _)| b);
                        let b1 = t.buf.char_indices().nth(word.end).map_or(t.buf.len(), |(b, _)| b);
                        t.buf.replace_range(b0..b1, &g);
                        t.spell_word = None;
                        ui.close();
                    }
                }
                crate::widgets::menu_separator(ui);
                if crate::widgets::menu_item(ui, &crate::t!("spell-learn"), None).clicked() {
                    crate::spell::learn(&text);
                    crate::spell::invalidate(&mut t.spell);
                    t.spell_word = None;
                    ui.close();
                }
                if crate::widgets::menu_item(ui, &crate::t!("spell-ignore"), None).clicked() {
                    crate::spell::ignore(&text);
                    crate::spell::invalidate(&mut t.spell);
                    t.spell_word = None;
                    ui.close();
                }
                // The edit-mode menu always offers it; it works on a selection.
                crate::widgets::menu_separator(ui);
                crate::widgets::menu_row(ui, None, &crate::actions::Action::MoveToNewTextBox.label(), None, false);
            });
            if !keep {
                t.spell_word = None;
            }
        }
        let r = resp.rect.expand(4.0);
        crate::overlay::dashed(ui.painter(), &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], true, Stroke::new(1.0, style::accent()), 4.0, 3.0);
        // Width handles on the box's sides: drag to set where it wraps.
        let mut width_hit = None;
        let mut on_handle = false;
        for (right, c) in [(false, r.left_center()), (true, r.right_center())] {
            let h = ui.interact(ERect::from_center_size(c, egui::vec2(14.0, 18.0)), edit_id.with(("width-handle", right)), Sense::drag());
            draw_width_handle(ui.painter(), c, egui::Vec2::X, style::accent());
            if h.hovered() || h.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            on_handle |= h.hovered() || h.dragged() || h.drag_stopped();
            if h.dragged() || h.drag_stopped() {
                width_hit = Some((right, h.drag_started(), h.drag_stopped(), h.interact_pointer_pos()));
            }
        }
        let caret = out.cursor_range.map(|c| out.galley.pos_from_cursor(c.primary).translate(out.galley_pos.to_vec2()));
        // Selected words: press and drag them out, or to another place.
        let sel_now = out.cursor_range.map_or(t.sel, |c| {
            let (a, b) = (usize::from(c.primary.index), usize::from(c.secondary.index));
            (a.min(b), a.max(b))
        });
        let field = crate::text_drag::Field {
            galley: &out.galley,
            origin: out.galley_pos - egui::vec2(out.galley.rect.left(), 0.0),
            rect: r,
            canvas: if app_rect_valid { canvas_rect } else { ui.ctx().content_rect() },
            id: edit_id,
            sel: sel_now,
            wrap: wrap_px,
        };
        let piece_job = |p: &TextData| rich_job(p, &p.text, &theme2, &font2, &font_name, style_weight, color, wrap_px.unwrap_or(f32::INFINITY));
        let words = crate::text_drag::interact(ui, &mut t, &field, &piece_job);
        t.laid_out = Some((out.galley.clone(), field.origin));
        (resp, caret, width_hit, on_handle, words)
    });
    let (resp, caret, width_hit, on_handle, words) = r.inner;
    if let Some((right, started, stopped, at)) = width_hit {
        text_width_handle(app, &mut t, resp.rect, right, started, at, ts.size * 0.6);
        if stopped && t.width_drag.take().is_some() && t.id.is_some() {
            app.editor.commit_as(&crate::app::HUMAN);
            if matches!(app.canvas.gesture, Gesture::TextWidth { .. }) {
                app.canvas.gesture = Gesture::None;
            }
        }
    }
    t.size_px = resp.rect.size();
    if let Some(cr) = egui::TextEdit::load_state(ctx, edit_id).and_then(|s| s.cursor.char_range()) {
        let (a, b) = (cr.primary.index, cr.secondary.index);
        t.sel = (usize::from(a.min(b)), usize::from(a.max(b)));
        // Typing past the canvas edge: the canvas follows the caret (the
        // text is clipped there, so the caret went out of view).
        let now = (usize::from(cr.primary.index), t.buf.len());
        let changed = t.caret_seen.is_some_and(|was| was != now);
        if changed {
            ctx.request_repaint();
        } else if std::mem::take(&mut t.caret_follow) && app.canvas.rect_valid {
            if let Some(c) = caret {
                let d = follow_caret(c, app.canvas.rect);
                if d != egui::Vec2::ZERO {
                    app.editor.view.camera.pan_screen(Vec2::new(d.x as f64, d.y as f64));
                    ctx.request_repaint();
                }
            }
        }
        t.caret_follow |= changed;
        t.caret_seen = Some(now);
    }

    // Mini text toolbar: box style, then character and
    // paragraph formatting for the selection.
    let mut tb_pos = resp.rect.left_top() - egui::vec2(4.0, 44.0);
    // A text that starts left of the canvas (the view followed the caret):
    // the toolbar stays on the canvas, not over the sidebar.
    // Its right edge stays on the canvas too, not over the inspector
    // (last frame's width; the bar's width hardly changes).
    if app.canvas.rect_valid {
        if let Some(r) = ctx.memory(|m| m.area_rect(app.uid("zebra-text-toolbar"))) {
            tb_pos.x = tb_pos.x.min(app.canvas.rect.right() - 4.0 - r.width());
        }
        tb_pos.x = tb_pos.x.max(app.canvas.rect.left() + 4.0);
    }
    let p = style::Palette::of(ctx);
    let (sa, sb) = t.sel;
    let has_sel = sa < sb;
    let n_chars = t.buf.chars().count();
    let (ra, rb) = if has_sel { (sa, sb) } else { (0, n_chars) };
    // Bold or heavier is on (a Semibold style is already bold).
    let style_weight = theme.text_style(t.style).weight;
    let is_bold = rich::all_bold(&t.rich, style_weight, ra, rb);
    let is_italic = all_on(&t.rich, ra, rb, |f| f.italic, t.italic);
    let is_under = all_on(&t.rich, ra, rb, |f| f.underline, false);
    let is_strike = all_on(&t.rich, ra, rb, |f| f.strike, false);
    let pa = rich::paragraph_of(&t.rich, sa);
    let pb = rich::paragraph_of(&t.rich, sb);
    let cur_list = rich::para(&t.rich, pa).0;
    let tb = egui::Area::new(app.uid("zebra-text-toolbar")).fixed_pos(tb_pos).order(egui::Order::Foreground).show(ctx, |ui| {
        style::float_frame(&p).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                use crate::widgets::{bar_button, bar_divider, bar_toggle};
                for (s, label) in [(TextStyle::Title, crate::t!("canvas-text-style-title")), (TextStyle::Subtitle, crate::t!("canvas-text-style-subtitle")), (TextStyle::Body, crate::t!("canvas-text-style-body"))] {
                    if bar_toggle(ui, t.style == s, label).clicked() {
                        t.style = s;
                        refocus = true;
                    }
                }
                bar_divider(ui);
                if bar_toggle(ui, is_bold, egui::RichText::new(crate::t!("canvas-text-bold-glyph")).strong()).on_hover_text(crate::i18n::with_shortcut(crate::t!("canvas-text-bold"), &crate::actions::Action::Bold)).clicked() {
                    fx.push(Fx::Bold);
                }
                if bar_toggle(ui, is_italic, egui::RichText::new(crate::t!("canvas-text-italic-glyph")).italics()).on_hover_text(crate::i18n::with_shortcut(crate::t!("canvas-text-italic"), &crate::actions::Action::Italic)).clicked() {
                    fx.push(Fx::Italic);
                }
                if bar_toggle(ui, is_under, egui::RichText::new(crate::t!("canvas-text-underline-glyph")).underline()).on_hover_text(crate::i18n::with_shortcut(crate::t!("canvas-text-underline"), &crate::actions::Action::Underline)).clicked() {
                    fx.push(Fx::Underline);
                }
                if bar_toggle(ui, is_strike, egui::RichText::new(crate::t!("canvas-text-strikethrough-glyph")).strikethrough()).on_hover_text(crate::t!("canvas-text-strikethrough")).clicked() {
                    fx.push(Fx::Strike);
                }
                if bar_button(ui, crate::t!("canvas-text-bigger-glyph")).on_hover_text(crate::t!("canvas-text-bigger")).clicked() {
                    fx.push(Fx::Bigger);
                }
                if bar_button(ui, crate::t!("canvas-text-smaller-glyph")).on_hover_text(crate::t!("canvas-text-smaller")).clicked() {
                    fx.push(Fx::Smaller);
                }
                let colour_btn = bar_button(ui, crate::t!("canvas-text-colour"));
                egui::Popup::menu(&colour_btn).style(crate::widgets::menu_style).width(160.0).show(|ui| {
                    for (slot, name) in [
                        ("", crate::t!("canvas-colour-default")),
                        ("theme:title", crate::t!("canvas-colour-title")),
                        ("theme:accent", crate::t!("canvas-colour-accent")),
                        ("theme:shape1", crate::t!("canvas-colour-n", n = 1)),
                        ("theme:shape2", crate::t!("canvas-colour-n", n = 2)),
                        ("theme:shape3", crate::t!("canvas-colour-n", n = 3)),
                        ("theme:shape4", crate::t!("canvas-colour-n", n = 4)),
                    ] {
                        let c = if slot.is_empty() { color } else { crate::fallback::theme_color(&theme, slot) };
                        if crate::widgets::menu_swatch(ui, c, &name).clicked() {
                            fx.push(Fx::Color(slot.to_string()));
                        }
                    }
                });
                if bar_toggle(ui, t.link_draft.is_some(), crate::t!("canvas-text-link")).on_hover_text(crate::i18n::with_key(crate::t!("canvas-text-link"), egui::Modifiers::COMMAND, egui::Key::K)).clicked() {
                    fx.push(Fx::Link);
                }
                bar_divider(ui);
                if bar_toggle(ui, cur_list == ListKind::Bullet, "•").on_hover_text(crate::t!("canvas-text-bulleted-list")).clicked() {
                    fx.push(Fx::List(if cur_list == ListKind::Bullet { ListKind::None } else { ListKind::Bullet }));
                }
                if bar_toggle(ui, cur_list == ListKind::Number, "1.").on_hover_text(crate::t!("canvas-text-numbered-list")).clicked() {
                    fx.push(Fx::List(if cur_list == ListKind::Number { ListKind::None } else { ListKind::Number }));
                }
                // Drawn icons, not ⇤/⇥ characters: the UI fonts may not
                // have those glyphs (egui would draw an empty box).
                if crate::widgets::icon_button(ui, crate::icons::Icon::Outdent, 24.0, true, false, &crate::t!("canvas-text-decrease-indent")).on_hover_text(crate::i18n::with_key(crate::t!("canvas-text-decrease-indent"), egui::Modifiers::SHIFT, egui::Key::Tab)).clicked() {
                    fx.push(Fx::Indent(-1));
                }
                if crate::widgets::icon_button(ui, crate::icons::Icon::Indent, 24.0, true, false, &crate::t!("canvas-text-increase-indent")).on_hover_text(crate::i18n::with_key(crate::t!("canvas-text-increase-indent"), egui::Modifiers::NONE, egui::Key::Tab)).clicked() {
                    fx.push(Fx::Indent(1));
                }
                bar_divider(ui);
                if bar_button(ui, crate::t!("canvas-text-done")).clicked() {
                    committed = true;
                }
            });
            if let Some(mut url) = t.link_draft.take() {
                let mut keep = true;
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.label(crate::t!("canvas-link-to"));
                    let hint = crate::t!("canvas-link-hint");
                    let r = crate::widgets::text_field(ui, &mut url, 240.0, &hint);
                    if !r.has_focus() && !r.lost_focus() && url.is_empty() {
                        r.request_focus();
                    }
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if crate::widgets::push_button(ui, &crate::t!("canvas-link-apply")).clicked() || enter {
                        let fmt = SpanFormat { link: Some(url.trim().to_string()), ..Default::default() };
                        rich::apply_format(&mut t.rich, ra, rb, &fmt);
                        keep = false;
                        refocus = true;
                    }
                    if crate::widgets::push_button(ui, &crate::t!("canvas-link-remove")).clicked() {
                        rich::apply_format(&mut t.rich, ra, rb, &SpanFormat { link: Some(String::new()), ..Default::default() });
                        keep = false;
                        refocus = true;
                    }
                });
                if keep {
                    t.link_draft = Some(url);
                }
            }
        });
    });

    for f in fx {
        refocus = true;
        let fmt = match f {
            Fx::Bold if !has_sel => {
                // The whole box. Against a heavy style (or words with their
                // own bold) the box setting alone isn't enough.
                t.bold = !is_bold;
                if style_weight >= zebra_core::theme::BOLD_FROM || t.rich.spans.iter().any(|s| s.format.bold.is_some()) {
                    rich::apply_format(&mut t.rich, 0, n_chars, &SpanFormat { bold: Some(t.bold), ..Default::default() });
                }
                None
            }
            Fx::Italic if !has_sel => {
                t.italic = !t.italic;
                None
            }
            Fx::Bold => Some(SpanFormat { bold: Some(!is_bold), ..Default::default() }),
            Fx::Italic => Some(SpanFormat { italic: Some(!is_italic), ..Default::default() }),
            Fx::Underline => Some(SpanFormat { underline: Some(!is_under), ..Default::default() }),
            Fx::Strike => Some(SpanFormat { strike: Some(!is_strike), ..Default::default() }),
            Fx::Bigger | Fx::Smaller => {
                let k = if f == Fx::Bigger { 1.25 } else { 0.8 };
                // Scale each run's own size.
                let cur: Vec<(usize, f64)> = (ra..rb).map(|i| (i, t.rich.spans.iter().find(|s| s.start <= i && i < s.end).and_then(|s| s.format.size).unwrap_or(1.0))).collect();
                for (i, s) in cur {
                    rich::apply_format(&mut t.rich, i, i + 1, &SpanFormat { size: Some((s * k).clamp(0.25, 8.0)), ..Default::default() });
                }
                None
            }
            Fx::Color(c) => Some(SpanFormat { color: Some(c), ..Default::default() }),
            Fx::Link => {
                if t.link_draft.is_some() {
                    t.link_draft = None;
                } else {
                    let cur = rich::common_format(&t.rich, ra, rb).link.unwrap_or_default();
                    t.link_draft = Some(cur);
                    refocus = false;
                }
                None
            }
            Fx::List(l) => {
                rich::format_paragraphs(&mut t.rich, pa, pb, Some(l), None, 0, None);
                None
            }
            Fx::Indent(d) => {
                rich::format_paragraphs(&mut t.rich, pa, pb, None, None, d, None);
                None
            }
            Fx::Outdent(n) => {
                if rich::para(&t.rich, n).0 != ListKind::None {
                    rich::format_paragraphs(&mut t.rich, n, n, Some(ListKind::None), None, 0, None);
                } else {
                    rich::format_paragraphs(&mut t.rich, n, n, None, None, -1, None);
                }
                None
            }
        };
        if let Some(fmt) = fmt {
            rich::apply_format(&mut t.rich, ra, rb, &fmt);
        }
    }

    app.canvas.text_toolbar = Some(tb.response.rect);
    let over_toolbar = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| tb.response.rect.contains(p));
    let link_open = t.link_draft.is_some();
    if (refocus && !link_open) || (resp.lost_focus() && (over_toolbar || on_handle || link_open || spell_open || words.keep_focus) && !committed) {
        if !link_open {
            resp.request_focus();
        }
    } else if resp.lost_focus() {
        committed = true;
    }
    app.canvas.text = Some(t);
    if committed {
        commit_text_edit(app);
        app.canvas.text_toolbar = None;
    } else if let Some(d) = words.drop {
        crate::text_drag::apply(app, d);
    } else if words.move_out {
        app.queued.push(crate::actions::Action::MoveToNewTextBox);
    }
}

/// A width handle of the text being typed was dragged to `at` (screen).
/// A text already on the canvas changes as a drag on its handles would
/// (one undo step, in its own transaction); a new one only takes the wrap
/// width it will be made with. `field`: the text field on screen.
fn text_width_handle(app: &mut ZebraApp, t: &mut TextEditState, field: ERect, right: bool, started: bool, at: Option<Pos2>, min: f64) {
    let Some(at) = at else { return };
    // The handles sit 4 points out from the text: the side follows the
    // pointer without first jumping out to it.
    let at = at + egui::vec2(if right { -4.0 } else { 4.0 }, 0.0);
    let world = app.canvas.s2w(&app.editor, at);
    if started || t.width_drag.is_none() {
        let anchor = match t.id.as_ref().and_then(|id| app.editor.doc.get(id)) {
            Some(e) => {
                let a = e.corners()[if right { 0 } else { 1 }];
                if app.editor.begin(crate::t!("canvas-undo-text-width"), H).is_err() {
                    return;
                }
                // The canvas sees a drag under way: nothing else ends the
                // transaction early.
                app.canvas.gesture = Gesture::TextWidth { id: t.id.clone().unwrap_or_default(), right, anchor: a };
                a
            }
            None => app.canvas.s2w(&app.editor, if right { field.left_top() } else { field.right_top() }),
        };
        t.width_drag = Some((right, anchor));
    }
    let Some((right, anchor)) = t.width_drag else { return };
    match t.id.clone() {
        Some(id) => {
            drag_text_width(app, &id, right, anchor, world);
            if let Some(e) = app.editor.doc.get(&id) {
                t.wrap = Some(e.size.w);
                t.anchor = e.corners()[0];
            }
        }
        None => {
            let px = (t.scale * app.editor.view.camera.zoom) as f32;
            let a = app.canvas.w2s(&app.editor, anchor);
            let host = t.host.as_ref().and_then(|h| app.editor.doc.get(h)).map(|h| app.canvas.w2s(&app.editor, h.transform.center()));
            let w = match host {
                // Text on a shape stays centred on it: both sides move.
                Some(c) => 2.0 * (at.x - c.x).abs(),
                None if right => at.x - a.x,
                None => a.x - at.x,
            };
            let w = (w / px) as f64;
            let w = w.max(min);
            t.wrap = Some(w);
            if host.is_none() && !right {
                t.anchor = app.canvas.s2w(&app.editor, Pos2::new(a.x - w as f32 * px, a.y));
            }
        }
    }
}

/// How far to slide the canvas (screen points) so the caret is inside it,
/// with a little room; zero when it already is.
pub fn follow_caret(caret: ERect, canvas: ERect) -> egui::Vec2 {
    let room = 24.0_f32.min(canvas.width() / 4.0).min(canvas.height() / 4.0);
    let inner = canvas.shrink(room);
    let axis = |lo: f32, hi: f32, min: f32, max: f32| {
        if hi > max {
            max - hi
        } else if lo < min {
            min - lo
        } else {
            0.0
        }
    };
    egui::vec2(axis(caret.left(), caret.right(), inner.left(), inner.right()), axis(caret.top(), caret.bottom(), inner.top(), inner.bottom()))
}

// ----------------------------------------------------------------------
// Snapping

fn snap_move(app: &ZebraApp, moving: &HashSet<ElementId>, b: &Rect, d: Vec2, thr: f64) -> (Vec2, Vec<[Vec2; 2]>) {
    let ed = &app.editor;
    let vis = ed.view.camera.visible_bounds(ed.view.viewport);
    let others: Vec<Rect> = ed.doc.ordered().filter(|e| !moving.contains(&e.id)).map(|e| e.bounds()).filter(|r| r.intersects(&vis)).collect();
    let mb = Rect { x0: b.x0 + d.x, y0: b.y0 + d.y, x1: b.x1 + d.x, y1: b.y1 + d.y };
    let xs = [mb.x0, (mb.x0 + mb.x1) / 2.0, mb.x1];
    let ys = [mb.y0, (mb.y0 + mb.y1) / 2.0, mb.y1];
    let mut best_x: Option<(f64, f64, Rect)> = None; // (adjust, line x, other)
    let mut best_y: Option<(f64, f64, Rect)> = None;
    for o in &others {
        for tx in [o.x0, (o.x0 + o.x1) / 2.0, o.x1] {
            for sx in xs {
                let diff = tx - sx;
                if diff.abs() < thr && best_x.is_none_or(|b| diff.abs() < b.0.abs()) {
                    best_x = Some((diff, tx, *o));
                }
            }
        }
        for ty in [o.y0, (o.y0 + o.y1) / 2.0, o.y1] {
            for sy in ys {
                let diff = ty - sy;
                if diff.abs() < thr && best_y.is_none_or(|b| diff.abs() < b.0.abs()) {
                    best_y = Some((diff, ty, *o));
                }
            }
        }
    }
    let mut out = d;
    let mut guides = vec![];
    if let Some((adj, _, _)) = best_x {
        out.x += adj;
    }
    if let Some((adj, _, _)) = best_y {
        out.y += adj;
    }
    let fb = Rect { x0: b.x0 + out.x, y0: b.y0 + out.y, x1: b.x1 + out.x, y1: b.y1 + out.y };
    if let Some((_, x, o)) = best_x {
        guides.push([Vec2::new(x, fb.y0.min(o.y0)), Vec2::new(x, fb.y1.max(o.y1))]);
    }
    if let Some((_, y, o)) = best_y {
        guides.push([Vec2::new(fb.x0.min(o.x0), y), Vec2::new(fb.x1.max(o.x1), y)]);
    }
    (out, guides)
}

// ----------------------------------------------------------------------
// Overlays

/// Screen positions of path badges.
/// World corners of a step's camera box for view and overview steps
/// (drawn dashed in path editing); `None` for element steps.
fn step_view_quad(app: &ZebraApp, t: &StepTarget) -> Option<Vec<Pos2>> {
    let ed = &app.editor;
    let v = match t {
        StepTarget::View { view } => *view,
        StepTarget::Overview => {
            let b = ed.doc.bounds()?;
            let c = b.center();
            zebra_core::ViewRect { cx: c.x, cy: c.y, width: b.width(), height: b.height(), rotation: 0.0 }
        }
        StepTarget::Element { .. } => return None,
    };
    let (hw, hh) = (v.width / 2.0, v.height / 2.0);
    Some(
        [Vec2::new(-hw, -hh), Vec2::new(hw, -hh), Vec2::new(hw, hh), Vec2::new(-hw, hh)]
            .iter()
            .map(|p| app.canvas.w2s(ed, p.rotate_deg(v.rotation) + Vec2::new(v.cx, v.cy)))
            .collect(),
    )
}

pub fn badge_positions(app: &ZebraApp) -> Vec<Pos2> {
    let ed = &app.editor;
    // Element steps: the element's centre. View and overview steps: the
    // top-left corner of their dashed camera box, so they don't land on
    // whatever content happens to be in the middle (critique 11).
    let raw: Vec<Pos2> = ed.doc
        .path
        .iter()
        .map(|s| match &s.target {
            StepTarget::Element { id } => app.canvas.w2s(ed, ed.doc.get(id).map(|e| e.transform.center()).unwrap_or(Vec2::ZERO)),
            t => step_view_quad(app, t).map(|q| q[0] + (q[2] - q[0]).normalized() * 22.0).unwrap_or(app.canvas.rect.center()),
        })
        .collect();
    // Steps sharing a spot (e.g. overview at start and end) fan out around it.
    let mut out: Vec<Pos2> = Vec::with_capacity(raw.len());
    for p in raw {
        let mut q = p;
        let mut k = 0;
        while out.iter().any(|o| o.distance(q) < 24.0) && k < 24 {
            k += 1;
            let ring = 1 + (k - 1) / 6;
            let a = (k as f32) * std::f32::consts::TAU / 6.0 + ring as f32 * 0.5;
            q = p + egui::vec2(a.cos(), a.sin()) * 30.0 * ring as f32;
        }
        out.push(q);
    }
    out
}

fn badge_at(app: &ZebraApp, p: Pos2) -> Option<usize> {
    // Later steps sit on top.
    badge_positions(app).iter().enumerate().rev().find(|(_, b)| b.distance(p) < 15.0).map(|(i, _)| i)
}

/// Screen outline of what an element draws: its box, or the inscribed
/// circle for circle frames.
fn outline_screen(app: &ZebraApp, e: &zebra_core::Element) -> Vec<Pos2> {
    if e.is_circle_frame() {
        let (w, _) = e.visual_size();
        let r = w / 2.0;
        return (0..64)
            .map(|i| {
                let a = i as f64 / 64.0 * std::f64::consts::TAU;
                app.canvas.w2s(&app.editor, e.transform.apply(Vec2::new(r * a.cos(), r * a.sin())))
            })
            .collect();
    }
    quad_screen(app, e.corners())
}

pub fn quad_screen(app: &ZebraApp, corners: [Vec2; 4]) -> Vec<Pos2> {
    corners.iter().map(|c| app.canvas.w2s(&app.editor, *c)).collect()
}

/// A dashed closed outline in screen points, drawn only near the screen
/// (see [`crate::overlay`]): outlines of things far away or huge on screen
/// would otherwise cost one shape per dash.
fn dashed_quad(painter: &Painter, q: &[Pos2], stroke: Stroke, dash: f32, gap: f32) {
    crate::overlay::dashed(painter, q, true, stroke, dash, gap);
}

pub fn draw_agent_presence(app: &ZebraApp, painter: &Painter) -> Option<ERect> {
    let ed = &app.editor;
    let now = ed.view.now;
    // Agent selection: violet, dashed.
    for id in &ed.view.agent_selection {
        if let Some(e) = ed.doc.get(id) {
            let q = quad_screen(app, e.corners());
            dashed_quad(painter, &q, Stroke::new(1.5, AGENT), 6.0, 4.0);
        }
    }
    // ~1 s flash on agent-changed elements.
    for (id, t) in &ed.flashes {
        let age = now - t;
        if !(0.0..1.2).contains(&age) {
            continue;
        }
        if let Some(e) = ed.doc.get(id) {
            painter.ctx().request_repaint();
            let a = (1.0 - age / 1.2) as f32;
            let mut q = quad_screen(app, e.corners());
            q.push(q[0]);
            painter.add(Shape::line(q, Stroke::new(2.5, AGENT.gamma_multiply(a))));
        }
    }
    // Agent cursor with name label. Fades out after 5 s without agent
    // edits, whether or not the agent is still connected (critique 18).
    let mut edge = None;
    if let Some(c) = ed.agent_cursor {
        const HOLD: f64 = 5.0;
        const FADE: f64 = 0.6;
        let age = now - ed.agent_cursor_time;
        let a = if age < HOLD { 1.0 } else { (1.0 - (age - HOLD) / FADE).clamp(0.0, 1.0) as f32 };
        if a > 0.0 {
            if age < HOLD {
                painter.ctx().request_repaint_after(std::time::Duration::from_secs_f64(HOLD - age));
            } else {
                painter.ctx().request_repaint();
            }
            let agent = AGENT.gamma_multiply(a);
            let white = Color32::WHITE.gamma_multiply(a);
            let p = app.canvas.w2s(ed, c);
            let name = app.agent_name();
            if app.canvas.rect.expand(-2.0).contains(p) {
                let pts = vec![p, p + egui::vec2(0.0, 18.0), p + egui::vec2(4.5, 13.5), p + egui::vec2(12.0, 13.0)];
                painter.add(Shape::convex_polygon(pts, agent, Stroke::new(1.5, white)));
                let galley = painter.layout_no_wrap(name, style::ui_font(12.0), white);
                let r = ERect::from_min_size(p + egui::vec2(12.0, 16.0), galley.size() + egui::vec2(12.0, 6.0));
                painter.rect_filled(r, 3.0, agent);
                painter.galley(r.min + egui::vec2(6.0, 3.0), galley, white);
            } else {
                // Edge arrow + label pointing to off-screen agent edits;
                // clicking the label flies there.
                let rect = app.canvas.rect.shrink(18.0);
                let c0 = rect.center();
                let d = (p - c0).normalized();
                let tx = if d.x.abs() > 1e-6 { (rect.width() / 2.0) / d.x.abs() } else { f32::INFINITY };
                let ty = if d.y.abs() > 1e-6 { (rect.height() / 2.0) / d.y.abs() } else { f32::INFINITY };
                let e = c0 + d * tx.min(ty);
                let perp = egui::vec2(-d.y, d.x);
                painter.add(Shape::convex_polygon(vec![e + d * 9.0, e - d * 5.0 + perp * 7.0, e - d * 5.0 - perp * 7.0], agent, Stroke::NONE));
                let galley = painter.layout_no_wrap(crate::t!("canvas-agent-editing-here", name = name.to_string()), style::ui_font(11.0), white);
                let size = galley.size() + egui::vec2(14.0, 6.0);
                // Place the label inward from the arrow, kept inside the canvas.
                let centre = e - d * (14.0 + size.length() / 2.0);
                let r = ERect::from_center_size(centre, size);
                let inner = app.canvas.rect.shrink(4.0);
                let r = r.translate(egui::vec2(
                    (inner.min.x - r.min.x).max(0.0) + (inner.max.x - r.max.x).min(0.0),
                    (inner.min.y - r.min.y).max(0.0) + (inner.max.y - r.max.y).min(0.0),
                ));
                painter.rect_filled(r, 3.0, agent);
                painter.galley(r.min + egui::vec2(7.0, 3.0), galley, white);
                edge = Some(r);
            }
        }
    }
    edge
}

fn draw_overlays(app: &mut ZebraApp, painter: &Painter, path_mode: bool, zoom: f64) {
    let ed = &app.editor;
    let p = style::Palette::of(painter.ctx());
    // Invisible frames: dashed in edit mode.
    for e in ed.doc.ordered() {
        if let ElementKind::Frame(f) = &e.kind {
            if f.style == FrameStyle::Invisible {
                let q = quad_screen(app, e.corners());
                dashed_quad(painter, &q, Stroke::new(1.0, Color32::from_gray(150)), 5.0, 4.0);
            }
        }
    }
    // Web videos: a chain-link mark in the corner (edit mode only).
    crate::web_video_ui::paint_link_marks(app, painter);
    // Hover outline + frame chip.
    app.canvas.frame_chip = None;
    app.canvas.chip_chevron = None;
    let sel = &ed.view.selection;
    let hover_frame = app.canvas.hover.clone().filter(|h| ed.doc.get(h).is_some_and(|e| e.is_frame()));
    if let Some(h) = &app.canvas.hover {
        if !sel.contains(h) && !path_mode {
            if let Some(e) = ed.doc.get(h) {
                let mut q = outline_screen(app, e);
                q.push(q[0]);
                painter.add(Shape::line(q, Stroke::new(1.0, style::accent().gamma_multiply(0.7))));
            }
        }
    }
    let chip_frame = hover_frame.or_else(|| match sel.as_slice() {
        [one] if ed.doc.get(one).is_some_and(|e| e.is_frame()) => Some(one.clone()),
        _ => None,
    });
    if let (Some(fid), false) = (chip_frame, path_mode) {
        if let Some(e) = ed.doc.get(&fid) {
            let tl = app.canvas.w2s(ed, e.corners()[0]);
            let galley = painter.layout_no_wrap(crate::words::element_label(e, &ed.doc), style::ui_font(11.0), p.text);
            let chev_w = 20.0;
            let r = ERect::from_min_size(tl + egui::vec2(0.0, -24.0), egui::vec2(galley.size().x + 14.0 + chev_w, galley.size().y.max(14.0) + 8.0));
            let chev = ERect::from_min_max(egui::pos2(r.max.x - chev_w, r.min.y), r.max);
            if app.canvas.renaming.as_ref().is_none_or(|(rid, _, _)| *rid != fid) {
                painter.rect(r, 6.0, p.float, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                painter.galley(r.min + egui::vec2(7.0, 4.0), galley, p.text);
                painter.line_segment([egui::pos2(chev.min.x, r.min.y + 4.0), egui::pos2(chev.min.x, r.max.y - 4.0)], Stroke::new(1.0, p.border));
                crate::icons::paint_icon(painter, ERect::from_center_size(chev.center(), egui::vec2(10.0, 10.0)), crate::icons::Icon::ChevronRight, p.weak);
            }
            app.canvas.frame_chip = Some((r, fid));
            app.canvas.chip_chevron = Some(chev);
        }
    }

    if path_mode {
        draw_path_overlay(app, painter);
    }

    app.canvas.agent_edge = draw_agent_presence(app, painter);

    // Selection + zebra transform tool.
    if !path_mode && app.canvas.text.is_none() {
        if let Some(id) = sole_connector(app) {
            draw_connector_handles(app, painter, &id);
        } else if crate::table_ui::active(app) {
            // Cells selected: the table's outline only (table_ui draws it).
        } else if let Some(sb) = selection_box(app, &ed.view.selection.clone()) {
            draw_transform_tool(app, painter, &sb);
        }
    }

    // Snapping guides, only while something is being moved.
    let moving = matches!(app.canvas.gesture, Gesture::Move { .. });
    for g in app.canvas.guides.iter().filter(|_| moving) {
        let a = app.canvas.w2s(ed, g[0]);
        let b = app.canvas.w2s(ed, g[1]);
        painter.line_segment([a, b], Stroke::new(1.0, GUIDE));
    }

    // Gesture previews.
    match &app.canvas.gesture {
        Gesture::Marquee { start, .. } => {
            if let Some(cur) = painter.ctx().input(|i| i.pointer.hover_pos()) {
                let r = ERect::from_two_pos(*start, cur);
                painter.rect(r, 2.0, style::accent().gamma_multiply(0.08), Stroke::new(1.0, style::accent()), egui::StrokeKind::Inside);
            }
        }
        Gesture::Create { tool, start } => {
            if let Some(cur) = painter.ctx().input(|i| i.pointer.hover_pos()) {
                let a = app.canvas.w2s(ed, *start);
                let st = Stroke::new(1.5, style::accent());
                match tool {
                    Tool::Line | Tool::Arrow => {
                        painter.line_segment([a, cur], st);
                    }
                    Tool::Ellipse => {
                        let r = ERect::from_two_pos(a, cur);
                        painter.add(egui::epaint::EllipseShape::stroke(r.center(), r.size() / 2.0, st));
                    }
                    Tool::Frame if ed.view.frame_style == FrameStyle::Circle => {
                        let r = ERect::from_two_pos(a, cur);
                        painter.circle_stroke(r.center(), r.width().max(r.height()) / 2.0, st);
                    }
                    Tool::Frame => {
                        let r = ERect::from_two_pos(a, cur);
                        draw_bracket_preview(painter, r, st);
                    }
                    _ => {
                        painter.rect_stroke(ERect::from_two_pos(a, cur), 0.0, st, egui::StrokeKind::Middle);
                    }
                }
            }
        }
        Gesture::Ink { points } => {
            let pts: Vec<Pos2> = points.iter().map(|w| app.canvas.w2s(ed, *w)).collect();
            let theme = ed.theme();
            painter.add(Shape::line(pts, Stroke::new(4.0, crate::fallback::theme_color(&theme, "theme:accent"))));
        }
        Gesture::Badge { step, .. } => {
            if let Some(cur) = painter.ctx().input(|i| i.pointer.hover_pos()) {
                draw_badge(painter, cur, *step + 1, true);
            }
        }
        _ => {}
    }
    // Where a line end would stick: outline the object under the pointer.
    let sticking = match &app.canvas.gesture {
        Gesture::ConnectorEnd { id, .. } => Some(Some(id.clone())),
        Gesture::Create { tool: Tool::Line | Tool::Arrow, .. } => Some(None),
        _ => None,
    };
    if let (Some(exclude), Some(cur)) = (sticking, painter.ctx().input(|i| i.pointer.hover_pos())) {
        let w = app.canvas.s2w(&app.editor, cur);
        if let Some(t) = attach_target(app, w, exclude.as_deref()).and_then(|t| app.editor.doc.get(&t)) {
            let mut q = quad_screen(app, t.corners());
            q.push(q[0]);
            painter.add(Shape::line(q, Stroke::new(2.0, style::accent())));
        }
    }
    let _ = zoom;
}

fn draw_bracket_preview(painter: &Painter, r: ERect, st: Stroke) {
    let arm = (r.width().min(r.height()) * 0.08).max(6.0);
    painter.add(Shape::line(vec![r.left_top() + egui::vec2(arm, 0.0), r.left_top(), r.left_bottom(), r.left_bottom() + egui::vec2(arm, 0.0)], st));
    painter.add(Shape::line(vec![r.right_top() - egui::vec2(arm, 0.0), r.right_top(), r.right_bottom(), r.right_bottom() - egui::vec2(arm, 0.0)], st));
}

/// Box with corner handles (uniform scale) and a zebra-striped rotation
/// handle — the signature transform tool.
fn draw_transform_tool(app: &ZebraApp, painter: &Painter, sb: &SelBox) {
    let locked = app.editor.view.selection.iter().all(|id| app.editor.doc.get(id).is_some_and(|e| e.locked));
    let col = if locked { Color32::from_gray(150) } else { style::accent() };
    // Frames: draw the outline 4 pt outside the frame so its own style
    // (brackets, circle, rectangle) stays visible (critique 24).
    let all_frames = !app.editor.view.selection.is_empty()
        && app.editor.view.selection.iter().all(|id| app.editor.doc.get(id).is_some_and(|e| e.is_frame()));
    let mut q = sb.screen.to_vec();
    if all_frames {
        let u = (sb.screen[1] - sb.screen[0]).normalized() * 4.0;
        let v = (sb.screen[3] - sb.screen[0]).normalized() * 4.0;
        q = vec![sb.screen[0] - u - v, sb.screen[1] + u - v, sb.screen[2] + u + v, sb.screen[3] - u + v];
    }
    q.push(q[0]);
    painter.add(Shape::line(q, Stroke::new(1.5, col)));
    if locked {
        let c = sb.screen[0];
        painter.text(c + egui::vec2(2.0, -4.0), Align2::LEFT_BOTTOM, crate::t!("canvas-locked"), style::ui_font(11.0), col);
        return;
    }
    // rotation handle: thin stem + zebra-striped disc
    let rp = rot_handle_pos(sb);
    painter.line_segment([sb.screen[1], rp], Stroke::new(1.0, col));
    draw_zebra_disc(painter, rp, 9.0);
    for c in sb.screen {
        let r = ERect::from_center_size(c, egui::vec2(9.0, 9.0));
        painter.rect(r, 2.0, Color32::WHITE, Stroke::new(1.5, col), egui::StrokeKind::Middle);
    }
    if sb.text.is_some() {
        let along = (sb.screen[1] - sb.screen[0]).normalized();
        for c in width_handles(sb) {
            draw_width_handle(painter, c, along, col);
        }
    }
    // While rotating: the angle beside the handle. Hovering the handle: what
    // it does (critique 23).
    let label = match &app.canvas.gesture {
        Gesture::Rotate { base_rot, applied, .. } => {
            let a = (base_rot + applied).rem_euclid(360.0);
            let a = if a > 180.0 { a - 360.0 } else { a };
            Some(crate::t!("canvas-rotate-angle", degrees = format!("{:.0}", a + 0.0)))
        }
        Gesture::None => {
            let hover = painter.ctx().pointer_hover_pos();
            hover.filter(|p| matches!(hit_handle(sb, *p), Some(Handle::Rotate))).map(|_| crate::t!("canvas-rotate-hint", modifier = "⇧"))
        }
        _ => None,
    };
    if let Some(text) = label {
        let font = style::ui_font(11.0);
        let galley = painter.layout_no_wrap(text, font, Color32::WHITE);
        let pad = egui::vec2(7.0, 3.0);
        let pos = rp + egui::vec2(14.0, -galley.size().y / 2.0 - pad.y);
        let r = ERect::from_min_size(pos, galley.size() + pad * 2.0);
        painter.rect_filled(r, 5.0, Color32::from_black_alpha(190));
        painter.galley(r.min + pad, galley, Color32::WHITE);
    }
}

/// A text box's width handle: the corner handles' square, upright along
/// the box, with a double-headed arrow across it.
fn draw_width_handle(painter: &Painter, c: Pos2, along: egui::Vec2, col: Color32) {
    let across = along.rot90();
    let (hw, hh) = (4.5, 7.0);
    let q = vec![c - along * hw - across * hh, c + along * hw - across * hh, c + along * hw + across * hh, c - along * hw + across * hh];
    painter.add(Shape::convex_polygon(q, Color32::WHITE, Stroke::new(1.5, col)));
    let s = Stroke::new(1.0, col);
    let (a, b) = (c - along * 2.6, c + along * 2.6);
    painter.line_segment([a, b], s);
    for (tip, d) in [(a, along), (b, -along)] {
        painter.line_segment([tip, tip + d * 1.6 + across * 1.6], s);
        painter.line_segment([tip, tip + d * 1.6 - across * 1.6], s);
    }
}

/// A selected connector: its route highlighted, with a round handle at each
/// end (filled when the end is stuck to something).
fn draw_connector_handles(app: &ZebraApp, painter: &Painter, id: &str) {
    let Some(e) = app.editor.doc.get(id) else { return };
    let ElementKind::Connector(c) = &e.kind else { return };
    let pts: Vec<Pos2> = zebra_core::connector::local_route(c).sample().iter().map(|p| app.canvas.w2s(&app.editor, e.transform.apply(*p))).collect();
    painter.add(Shape::line(pts, Stroke::new(1.5, style::accent().gamma_multiply(0.6))));
    for (end, attached) in [(&c.from, c.from.element.is_some()), (&c.to, c.to.element.is_some())] {
        let p = app.canvas.w2s(&app.editor, e.transform.apply(Vec2::new(end.x, end.y)));
        painter.circle(p, 5.5, if attached { style::accent() } else { Color32::WHITE }, Stroke::new(1.5, style::accent()));
    }
}

/// A disc with diagonal black/white zebra stripes.
pub fn draw_zebra_disc(painter: &Painter, c: Pos2, r: f32) {
    painter.circle_filled(c, r + 1.5, Color32::WHITE);
    painter.circle_filled(c, r, Color32::from_gray(250));
    let d = egui::vec2(1.0, -1.0).normalized();
    let n = egui::vec2(1.0, 1.0).normalized();
    let stripes = 5;
    for k in 0..stripes {
        // stripe band centred at offset o along n, width w
        let o = -r + (k as f32 + 0.5) * (2.0 * r / stripes as f32);
        let w = r / stripes as f32 * 1.0;
        if k % 2 == 1 {
            continue;
        }
        let mut pts = vec![];
        for (oo, rev) in [(o - w / 2.0, false), (o + w / 2.0, true)] {
            let h = (r * r - oo * oo).max(0.0).sqrt();
            let a = c + n * oo - d * h;
            let b = c + n * oo + d * h;
            if rev {
                pts.push(b);
                pts.push(a);
            } else {
                pts.push(a);
                pts.push(b);
            }
        }
        painter.add(Shape::convex_polygon(pts, Color32::from_gray(20), Stroke::NONE));
    }
    painter.circle_stroke(c, r, Stroke::new(1.2, Color32::from_gray(20)));
}

pub fn draw_badge(painter: &Painter, p: Pos2, n: usize, active: bool) {
    let fill = if active { style::accent() } else { PATH };
    painter.circle(p, 13.0, fill, Stroke::new(2.0, Color32::WHITE));
    painter.text(p, Align2::CENTER_CENTER, n.to_string(), style::bold(12.0), Color32::WHITE);
}

fn draw_path_overlay(app: &ZebraApp, painter: &Painter) {
    // Dim content slightly so the path stands out.
    let dim = Color32::from_white_alpha(70);
    painter.rect_filled(app.canvas.rect, 0.0, dim);
    let ed = &app.editor;
    // Element steps: faint dashed outline. View and overview steps: their
    // camera box, dashed.
    for s in &ed.doc.path {
        match &s.target {
            StepTarget::Element { id } => {
                if let Some(e) = ed.doc.get(id) {
                    let q = outline_screen(app, e);
                    dashed_quad(painter, &q, Stroke::new(1.0, PATH.gamma_multiply(0.5)), 4.0, 4.0);
                }
            }
            t => {
                if let Some(q) = step_view_quad(app, t) {
                    dashed_quad(painter, &q, Stroke::new(1.25, PATH.gamma_multiply(0.8)), 8.0, 5.0);
                }
            }
        }
    }
    let pos = badge_positions(app);
    if pos.len() >= 2 {
        // Gentle curve between consecutive badges, kept light so text under
        // it stays readable.
        for w in pos.windows(2) {
            let (a, b) = (w[0], w[1]);
            let mid = a.lerp(b, 0.5);
            let d = b - a;
            let ctrl = mid + egui::vec2(-d.y, d.x) * 0.12;
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0;
                    let u = 1.0 - t;
                    Pos2::new(u * u * a.x + 2.0 * u * t * ctrl.x + t * t * b.x, u * u * a.y + 2.0 * u * t * ctrl.y + t * t * b.y)
                })
                .collect();
            painter.add(Shape::line(pts, Stroke::new(2.0, PATH.gamma_multiply(0.45))));
        }
    }
    // Hovered object that isn't a step yet: outline + ghost badge with the
    // number it would get.
    if let Some(h) = &app.canvas.hover {
        if app.canvas.step_hover.is_none() {
            if let Some(e) = ed.doc.get(h) {
                let mut q = outline_screen(app, e);
                q.push(q[0]);
                painter.add(Shape::line(q, Stroke::new(1.5, PATH)));
                if ed.doc.steps_using(h).is_empty() {
                    let c = app.canvas.w2s(ed, e.transform.center());
                    painter.circle(c, 13.0, PATH.gamma_multiply(0.35), Stroke::new(1.5, Color32::WHITE.gamma_multiply(0.8)));
                    painter.text(c, Align2::CENTER_CENTER, format!("{}", ed.doc.path.len() + 1), style::bold(12.0), Color32::WHITE);
                }
            }
        }
    }
    let selected = app.selected_step;
    for (i, p) in pos.iter().enumerate() {
        draw_badge(painter, *p, i + 1, selected == Some(i) || app.canvas.step_hover == Some(i));
        let builds = ed.doc.path.get(i).map_or(0, |s| s.builds.len());
        if builds > 0 {
            let g = painter.layout_no_wrap(format!("+{builds}"), style::ui_font(11.0), Color32::WHITE);
            let r = ERect::from_min_size(*p + egui::vec2(10.0, -22.0), g.size() + egui::vec2(8.0, 2.0));
            painter.rect_filled(r, r.height() / 2.0, PATH);
            painter.galley(r.min + egui::vec2(4.0, 1.0), g, Color32::WHITE);
        }
    }
}

/// Right-click on the canvas: like a Mac app, right-clicking an unselected
/// object selects it first; the menu then acts on the selection.
fn context_menu(app: &mut ZebraApp, resp: &egui::Response) {
    use crate::actions::Action;
    use crate::widgets as w;
    if resp.secondary_clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            commit_text_edit(app);
            app.canvas.menu_at = Some(p);
            crate::table_ui::secondary_click(app, p);
            match hit(app, p, false) {
                Some(id) if !app.editor.view.selection.contains(&id) => app.select(vec![id]),
                Some(_) => {}
                None => {
                    let _ = app.editor.view(ViewCommand::ClearSelection, H);
                }
            }
        }
    }
    let item = |ui: &mut Ui, app: &mut ZebraApp, a: Action| {
        let st = crate::menus::state(app, &a);
        if w::menu_row(ui, st.checked, &st.title, a.shortcut_text().as_deref(), st.enabled).clicked() {
            app.queued.push(a);
        }
    };
    egui::Popup::context_menu(resp).style(crate::widgets::menu_style).width(220.0).show(|ui| {
        let sel = app.selection();
        let one = (sel.len() == 1).then(|| app.editor.doc.get(&sel[0]).cloned()).flatten();
        if sel.is_empty() {
            if w::menu_item(ui, &crate::t!("canvas-menu-add-text-here"), None).clicked() {
                if let Some(p) = app.canvas.menu_at {
                    begin_new_text(app, p);
                }
            }
            item(ui, app, Action::ShapeLibrary);
            item(ui, app, Action::InsertImage);
            w::menu_separator(ui);
            item(ui, app, Action::Paste);
            item(ui, app, Action::SelectAll);
            w::menu_separator(ui);
            item(ui, app, Action::AddCurrentView);
            item(ui, app, Action::FitAll);
            return;
        }
        // Object-specific actions first.
        match one.as_ref().map(|e| &e.kind) {
            Some(ElementKind::Frame(_)) => {
                item(ui, app, Action::SelectFrameOnly);
                w::menu_separator(ui);
            }
            Some(ElementKind::Shape(s)) if !s.shape.is_line() => {
                let label = if one.as_ref().and_then(|e| shape_label(app, e)).is_some() { crate::t!("canvas-menu-edit-text") } else { crate::t!("canvas-menu-add-text") };
                if w::menu_item(ui, &label, None).clicked() {
                    begin_text_on_shape(app, sel[0].clone());
                }
                w::menu_separator(ui);
            }
            Some(ElementKind::Text(_)) => {
                if w::menu_item(ui, &crate::t!("canvas-menu-edit-text"), None).clicked() {
                    begin_edit_existing(app, sel[0].clone());
                }
                w::menu_separator(ui);
            }
            Some(ElementKind::Icon(_)) => {
                if w::menu_item(ui, &crate::t!("canvas-menu-replace-icon"), None).clicked() {
                    app.library.open_replace(sel[0].clone());
                }
                w::menu_separator(ui);
            }
            Some(ElementKind::Image(_)) => {
                item(ui, app, Action::CropImage);
                item(ui, app, Action::ReplaceImage);
                w::menu_separator(ui);
            }
            Some(ElementKind::Video(_)) => {
                item(ui, app, Action::PlayPauseVideo);
                w::menu_separator(ui);
            }
            Some(ElementKind::Table(_)) if crate::table_ui::active(app) => crate::table_ui::menu(ui, app),
            _ => {}
        }
        for a in [Action::Cut, Action::Copy, Action::Paste, Action::Duplicate, Action::Delete] {
            item(ui, app, a);
        }
        w::menu_separator(ui);
        let grouped = sel.iter().any(|id| app.editor.doc.get(id).is_some_and(|e| e.group.is_some()));
        if sel.len() > 1 {
            item(ui, app, Action::Group);
        }
        if grouped {
            item(ui, app, Action::Ungroup);
        }
        item(ui, app, Action::BringToFront);
        item(ui, app, Action::SendToBack);
        item(ui, app, Action::ToggleLock);
        w::menu_separator(ui);
        item(ui, app, Action::WrapInFrame);
        item(ui, app, Action::AddToPath);
        item(ui, app, Action::ZoomToSelection);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new, empty list item: its caret row starts where the text of the
    /// items above starts, not in the margin.
    #[test]
    fn empty_list_item_lines_up_with_the_others() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        let mut t = TextData { text: "One\nTwo".into(), ..Default::default() };
        zebra_core::rich::format_paragraphs(&mut t, 0, 1, Some(ListKind::Bullet), None, 0, None);
        let font = FontId::proportional(20.0);
        let theme = zebra_core::Theme::resolve("classic");
        for buf in ["One\nTwo\n", "One\n\nTwo"] {
            let job = rich_job(&t, buf, &theme, &font, "", 400, Color32::WHITE, f32::INFINITY);
            let g = ctx.fonts_mut(|f| f.layout_job(job));
            let g = indent_empty_list_rows(g, &t, buf, font.size);
            let first = g.pos_from_cursor(egui::text::CCursor::new(0)).min.x;
            let empty_at = buf.find("\n\n").map(|i| i + 1).unwrap_or(buf.chars().count());
            let empty = g.pos_from_cursor(egui::text::CCursor::new(empty_at)).min.x;
            assert!((first - empty).abs() < 0.5, "{buf:?}: first item at {first}, empty item at {empty}");
            assert!(first > 0.0);
        }
    }
}
