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
    ShapeKind, StepTarget, TextData, TextStyle, Tool, Vec2, ViewCommand,
};

use crate::app::{ScrollMode, ZebraApp};
use crate::style::{self, ACCENT, AGENT, GUIDE, PATH};

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
    },
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
}

pub struct CanvasState {
    pub rect: ERect,
    pub gesture: Gesture,
    pub hover: Option<ElementId>,
    pub text: Option<TextEditState>,
    pub text_toolbar: Option<ERect>,
    pub guides: Vec<[Vec2; 2]>,
    pub frame_chip: Option<(ERect, ElementId)>,
    pub step_hover: Option<usize>,
    pub rect_valid: bool,
    pub rect_presenting: bool,
    /// Window content size when `rect` was last synced.
    pub window_size: egui::Vec2,
}

impl Default for CanvasState {
    fn default() -> Self {
        CanvasState {
            rect: ERect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0)),
            gesture: Gesture::None,
            hover: None,
            text: None,
            text_toolbar: None,
            guides: vec![],
            frame_chip: None,
            step_hover: None,
            rect_valid: false,
            rect_presenting: false,
            window_size: egui::Vec2::ZERO,
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
    Some(SelBox { world, screen: world.map(|p| app.canvas.w2s(ed, p)), center, rotation })
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
    Rotate,
    Body,
}

fn hit_handle(sb: &SelBox, p: Pos2) -> Option<Handle> {
    if rot_handle_pos(sb).distance(p) < 13.0 {
        return Some(Handle::Rotate);
    }
    for (i, c) in sb.screen.iter().enumerate() {
        if c.distance(p) < 9.0 {
            return Some(Handle::Corner(i));
        }
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
    let found = ed
        .doc
        .ordered()
        .rev()
        .filter(|e| !hidden.contains(&e.id))
        .filter(|e| !(presenting && matches!(&e.kind, ElementKind::Frame(f) if f.style == FrameStyle::Invisible)))
        .find(|e| e.hit(w, tol, false))
        .map(|e| e.id.clone());
    if found.is_some() || !frames_inside {
        return found;
    }
    ed.doc
        .ordered()
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
        opts.hidden = ed.hidden_builds();
        let now = ed.view.now;
        opts.opacity = app
            .revealed
            .iter()
            .filter_map(|(id, t)| {
                let a = ((now - t) / 0.3).clamp(0.0, 1.0) as f32;
                (a < 1.0).then(|| (id.clone(), a))
            })
            .collect();
    }
    opts.skip = app.canvas.text.as_ref().and_then(|t| t.id.clone());
    let ppp = ui.ctx().pixels_per_point() as f64;
    let vp = [rect.width() as f64, rect.height() as f64];
    if let (Some(canvas), Some(rs), true) = (&mut app.gpu_canvas, &app.wgpu, app.render_ok) {
        let cam = ed.view.camera;
        let r = &app.renderer;
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r.build_display_list(ed, &cam, vp, ppp, &opts)));
        match built {
            Ok(list) => {
                canvas.paint(ui, rect, &list, rs);
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
    let resp = ui.interact(rect, egui::Id::unique("zebra-canvas"), Sense::click_and_drag());
    paint_content(app, ui, rect, false);

    let ctx = ui.ctx().clone();
    let text_focus = ctx.egui_wants_keyboard_input();
    let space = !text_focus && ctx.input(|i| i.key_down(Key::Space));
    let pointer = resp.hover_pos();
    let mods = ctx.input(|i| i.modifiers);
    let path_mode = app.editor.view.mode == Mode::PathEdit;
    let tool = app.editor.view.tool;
    let zoom = app.editor.view.camera.zoom;

    handle_scroll(app, ui, resp.contains_pointer());

    // Hover
    app.canvas.hover = match (&app.canvas.gesture, pointer) {
        (Gesture::None, Some(p)) => hit(app, p, path_mode),
        _ => None,
    };
    let sel = app.selection();
    let sb = if path_mode { None } else { selection_box(app, &sel) };
    let handle_under = pointer.and_then(|p| sb.as_ref().and_then(|b| hit_handle(b, p)));
    let badge_under = if path_mode { pointer.and_then(|p| badge_at(app, p)) } else { None };
    app.canvas.step_hover = badge_under;

    // Enter edits the selected text.
    if !text_focus && app.canvas.text.is_none() && ctx.input(|i| i.key_pressed(Key::Enter) && i.modifiers.is_none()) {
        if let [id] = sel.as_slice() {
            if matches!(app.editor.doc.get(id).map(|e| &e.kind), Some(ElementKind::Text(_))) {
                begin_edit_existing(app, id.clone());
            }
        }
    }

    // ---- press / drag
    let middle = resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary);
    if middle {
        let d = resp.drag_delta();
        let _ = app.editor.view(ViewCommand::Pan { dx: d.x as f64, dy: d.y as f64 }, H);
    }
    if resp.drag_started_by(PointerButton::Primary) {
        let start = ctx.input(|i| i.pointer.press_origin()).unwrap_or_default();
        commit_text_edit(app);
        start_gesture(app, start, space, mods.shift);
    }
    if resp.dragged_by(PointerButton::Primary) {
        if let Some(p) = resp.interact_pointer_pos() {
            update_gesture(app, p, resp.drag_delta(), mods.shift, mods.alt);
        }
    }
    if resp.drag_stopped_by(PointerButton::Primary) {
        let p = resp.interact_pointer_pos().or(pointer).unwrap_or_default();
        end_gesture(app, p, mods.shift);
    }

    // ---- clicks
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if app.canvas.text.is_some() {
                commit_text_edit(app);
            } else if let Some((r, id)) = app.canvas.frame_chip.clone().filter(|(r, _)| r.contains(p)) {
                let _ = r;
                let _ = app.editor.view(ViewCommand::ZoomTo { id, animate: true }, H);
            } else {
                click(app, p, mods.shift);
            }
        }
    }
    if resp.double_clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            double_click(app, p);
        }
    }

    // ---- cursor
    let cursor = match (&app.canvas.gesture, handle_under) {
        (Gesture::Pan, _) => CursorIcon::Grabbing,
        (Gesture::Rotate { .. }, _) | (_, Some(Handle::Rotate)) => CursorIcon::Alias,
        (Gesture::Scale { .. }, _) => CursorIcon::ResizeNwSe,
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
    if resp.contains_pointer() {
        ctx.set_cursor_icon(cursor);
    }

    // ---- overlays
    let painter = ui.painter_at(rect);
    draw_overlays(app, &painter, path_mode, zoom);
    text_overlay(app, &ctx);
}

fn start_gesture(app: &mut ZebraApp, start: Pos2, space: bool, shift: bool) {
    let ed = &app.editor;
    let tool = ed.view.tool;
    let world = app.canvas.s2w(ed, start);
    if space || tool == Tool::Hand {
        app.canvas.gesture = Gesture::Pan;
        return;
    }
    if ed.view.mode == Mode::PathEdit {
        if let Some(step) = badge_at(app, start) {
            app.canvas.gesture = Gesture::Badge { step };
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
    // Select tool: handles first.
    let sel = app.selection();
    if let Some(sb) = selection_box(app, &sel) {
        match hit_handle(&sb, start) {
            Some(Handle::Rotate) => {
                let ids = app.editable_selection();
                if ids.is_empty() {
                    return;
                }
                let d = world - sb.center;
                let _ = app.editor.begin("Rotate", H);
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
                let _ = app.editor.begin("Resize", H);
                app.canvas.gesture = Gesture::Scale {
                    ids: moving_ids(app, &ids),
                    anchor,
                    start_dist: (world - anchor).len().max(1e-9),
                    applied: 1.0,
                };
                return;
            }
            Some(Handle::Body) if hit(app, start, false).is_none_or(|h| sel.contains(&h)) => {
                begin_move(app, world);
                return;
            }
            _ => {}
        }
    }
    match hit(app, start, false) {
        Some(id) => {
            if !sel.contains(&id) {
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

/// The fixed set of elements a drag moves: the selection plus frame contents
/// computed once at drag start (so a frame doesn't scoop things up mid-drag).
fn moving_ids(app: &ZebraApp, ids: &[ElementId]) -> Vec<ElementId> {
    app.editor.expand(ids, true).into_iter().filter(|id| app.editor.doc.get(id).is_some_and(|e| !e.locked)).collect()
}

fn begin_move(app: &mut ZebraApp, origin: Vec2) {
    let sel = app.editable_selection();
    if sel.is_empty() {
        return;
    }
    let ids = moving_ids(app, &sel);
    let start_bounds = app.editor.bounds_of(&sel);
    if app.editor.begin("Move", H).is_err() {
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
            let (d, guides) = if alt { (raw, vec![]) } else { snap_move(app, moving, start_bounds, raw, 6.0 / zoom) };
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
        Gesture::Move { .. } | Gesture::Scale { .. } | Gesture::Rotate { .. } => app.editor.commit(),
        Gesture::Marquee { start, additive } => {
            let a = app.canvas.s2w(&app.editor, start);
            let r = Rect::from_points([a, world]).unwrap();
            let hidden: Vec<ElementId> = app
                .editor
                .doc
                .ordered()
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
                app.apply(Command::AddInk { points: pts, color: "theme:accent".into(), width: w, highlighter: hl });
            }
        }
        Gesture::Badge { step } => {
            if let Some(id) = hit(app, p, true) {
                app.apply(Command::UpdateStep { index: step, target: Some(StepTarget::Element { id }), builds: None, notes: None, duration: None });
                app.toast(format!("Step {} now points here", step + 1));
            }
        }
        Gesture::Pan | Gesture::None => {}
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
            Tool::Frame => match app.editor.view.frame_style {
                FrameStyle::Circle => (360.0, 360.0),
                _ => (480.0, 320.0),
            },
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
            (ElementKind::Frame(FrameData { style, color: None }), rot, w, h)
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
            (ElementKind::Shape(ShapeData { shape, fill: None, stroke: Some("theme:shape1".into()), stroke_width: 3.0 }), ang, len.max(8.0), 24.0)
        }
        _ => {
            let shape = match tool {
                Tool::Ellipse => ShapeKind::Ellipse,
                Tool::Triangle => ShapeKind::Triangle,
                _ => ShapeKind::Rectangle,
            };
            (ElementKind::Shape(ShapeData { shape, fill: Some("theme:shape1".into()), stroke: None, stroke_width: 3.0 }), rot, w, h)
        }
    };
    let is_frame = matches!(kind, ElementKind::Frame(_));
    let el = NewElement { id: None, x: center.x, y: center.y, scale: s, rotation, w: Some(w), h: Some(h), name: String::new(), kind };
    if let Some(out) = app.apply(Command::AddElement { element: el }) {
        app.select(out.created.clone());
        if is_frame {
            let n = out.created.first().map(|id| app.editor.doc.frame_contents(id).len()).unwrap_or(0);
            if n > 0 {
                app.toast(format!("Framed {n} object{}", if n == 1 { "" } else { "s" }));
            }
        }
    }
    let _ = app.editor.view(ViewCommand::SetTool { tool: Tool::Select }, H);
}

fn click(app: &mut ZebraApp, p: Pos2, shift: bool) {
    let mode = app.editor.view.mode;
    let tool = app.editor.view.tool;
    if mode == Mode::PathEdit {
        if let Some(step) = badge_at(app, p) {
            app.selected_step = Some(step);
            let _ = app.editor.view(ViewCommand::GotoStep { step }, H);
            return;
        }
        if let Some(id) = hit(app, p, true) {
            app.apply(Command::AddStep { target: StepTarget::Element { id }, index: None, builds: vec![], notes: String::new() });
            let n = app.editor.doc.path.len();
            app.selected_step = Some(n - 1);
            app.toast(format!("Added step {n}"));
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
        Some(id) if shift => {
            let mut s = app.selection();
            if let Some(i) = s.iter().position(|x| *x == id) {
                s.remove(i);
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
    });
    let _ = app.editor.view(ViewCommand::ClearSelection, H);
}

pub fn begin_edit_existing(app: &mut ZebraApp, id: ElementId) {
    commit_text_edit(app);
    let Some(e) = app.editor.doc.get(&id) else { return };
    let ElementKind::Text(t) = &e.kind else { return };
    if e.locked {
        app.toast("This text is locked — unlock it to edit (⌘L)");
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
    });
    app.select(vec![id]);
}

/// Cancel without applying (used by undo while editing).
pub fn cancel_text_edit(app: &mut ZebraApp) {
    app.canvas.text = None;
}

/// Apply the in-place edit as one command (Add text / Edit text).
pub fn commit_text_edit(app: &mut ZebraApp) {
    let Some(t) = app.canvas.text.take() else { return };
    let text = t.buf.trim_end().to_string();
    match &t.id {
        None => {
            if text.trim().is_empty() {
                return;
            }
            // Centre of the box as laid out on screen.
            let half = Vec2::new(t.size_px.x as f64, t.size_px.y as f64) * (0.5 / t.zoom);
            let c = t.anchor + half.rotate_deg(app.editor.view.camera.rotation);
            let el = NewElement {
                id: None,
                x: c.x,
                y: c.y,
                scale: t.scale,
                rotation: app.editor.view.camera.rotation,
                w: None,
                h: None,
                name: String::new(),
                kind: ElementKind::Text(TextData { text, style: t.style, bold: t.bold, italic: t.italic, bullets: t.bullets, ..Default::default() }),
            };
            let r = app.editor.apply_batch(vec![Command::AddElement { element: el }], Some("Add text".into()), H);
            if let Some(out) = app.report(r) {
                app.select(out.created);
            }
        }
        Some(id) => {
            let Some(ElementKind::Text(old)) = app.editor.doc.get(id).map(|e| e.kind.clone()) else { return };
            if text.trim().is_empty() {
                app.apply(Command::Delete { ids: vec![id.clone()] });
                return;
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
            if patch != ElementPatch::default() {
                let r = app.editor.apply_batch(vec![Command::Update { id: id.clone(), patch }], Some("Edit text".into()), H);
                app.report(r);
            }
        }
    }
}

fn text_overlay(app: &mut ZebraApp, ctx: &egui::Context) {
    let Some(mut t) = app.canvas.text.take() else { return };
    // The element may have been deleted (e.g. by an agent).
    if let Some(id) = &t.id {
        match app.editor.doc.get(id) {
            Some(e) => {
                t.anchor = e.corners()[0];
                t.scale = e.transform.scale;
            }
            None => return,
        }
    }
    let theme = app.editor.theme();
    let ts = theme.text_style(t.style);
    let zoom = app.editor.view.camera.zoom;
    t.zoom = zoom;
    let size = (ts.size * t.scale * zoom) as f32;
    let font = FontId::new(size.clamp(4.0, 600.0), style::doc_family(&ts.font, t.bold, t.italic));
    let color = crate::fallback::theme_color(&theme, match t.style {
        TextStyle::Title => "theme:title",
        TextStyle::Subtitle => "theme:subtitle",
        TextStyle::Body => "theme:body",
    });
    let pos = app.canvas.w2s(&app.editor, t.anchor);
    let measure = if t.buf.is_empty() { "Type something".to_string() } else { t.buf.clone() + " " };
    let gw = ctx.fonts_mut(|f| f.layout_no_wrap(measure, font.clone(), color).size().x);
    let width = gw.max(size * 2.0);
    let mut committed = false;
    let mut refocus = false;

    let area = egui::Area::new(egui::Id::unique("zebra-text-edit")).fixed_pos(pos).order(egui::Order::Foreground).constrain(false);
    let r = area.show(ctx, |ui| {
        // Areas otherwise remember last frame's (narrow) width, which made the
        // text wrap a few letters per line while typing.
        ui.set_min_width(width);
        let edit = egui::TextEdit::multiline(&mut t.buf)
            .font(font.clone())
            .text_color(color)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .desired_width(width)
            .desired_rows(1)
            .hint_text(egui::RichText::new("Type something").color(color.gamma_multiply(0.35)).font(font.clone()))
            .id(egui::Id::unique("zebra-text-edit-field"));
        let resp = ui.add(edit);
        if !t.focused_once {
            resp.request_focus();
            t.focused_once = true;
        }
        let r = resp.rect.expand(4.0);
        ui.painter().add(Shape::dashed_line(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()], Stroke::new(1.0, ACCENT), 4.0, 3.0));
        resp
    });
    let resp = r.inner;
    t.size_px = resp.rect.size();

    // Mini text toolbar (style, bold, italic, bullets), like Prezi's.
    let tb_pos = resp.rect.left_top() - egui::vec2(4.0, 44.0);
    let p = style::Palette::of(ctx);
    let tb = egui::Area::new(egui::Id::unique("zebra-text-toolbar")).fixed_pos(tb_pos).order(egui::Order::Foreground).show(ctx, |ui| {
        style::float_frame(&p).inner_margin(egui::Margin::same(3)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (s, label) in [(TextStyle::Title, "Title"), (TextStyle::Subtitle, "Subtitle"), (TextStyle::Body, "Body")] {
                    if ui.selectable_label(t.style == s, label).clicked() {
                        t.style = s;
                        refocus = true;
                    }
                }
                ui.separator();
                if ui.selectable_label(t.bold, egui::RichText::new("B").strong()).on_hover_text("Bold").clicked() {
                    t.bold = !t.bold;
                    refocus = true;
                }
                if ui.selectable_label(t.italic, egui::RichText::new("I").italics()).on_hover_text("Italic").clicked() {
                    t.italic = !t.italic;
                    refocus = true;
                }
                if ui.selectable_label(t.bullets, "• List").on_hover_text("Bullets").clicked() {
                    t.bullets = !t.bullets;
                    refocus = true;
                }
                ui.separator();
                if ui.button("Done").clicked() {
                    committed = true;
                }
            });
        });
    });
    app.canvas.text_toolbar = Some(tb.response.rect);
    let over_toolbar = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| tb.response.rect.contains(p));
    if refocus || (resp.lost_focus() && over_toolbar && !committed) {
        resp.request_focus();
    } else if resp.lost_focus() {
        committed = true;
    }
    app.canvas.text = Some(t);
    if committed {
        commit_text_edit(app);
        app.canvas.text_toolbar = None;
    }
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
pub fn badge_positions(app: &ZebraApp) -> Vec<Pos2> {
    let ed = &app.editor;
    let raw: Vec<Pos2> = ed.doc
        .path
        .iter()
        .map(|s| {
            let w = match &s.target {
                StepTarget::Element { id } => ed.doc.get(id).map(|e| e.transform.center()).unwrap_or(Vec2::ZERO),
                StepTarget::View { view } => Vec2::new(view.cx, view.cy),
                StepTarget::Overview => ed.doc.bounds().map(|b| b.center()).unwrap_or(Vec2::ZERO),
            };
            app.canvas.w2s(ed, w)
        })
        .collect();
    // Steps sharing a target (e.g. overview at start and end) sit side by side.
    let mut out: Vec<Pos2> = Vec::with_capacity(raw.len());
    for p in raw {
        let mut q = p;
        while out.iter().any(|o| o.distance(q) < 20.0) {
            q.x += 30.0;
        }
        out.push(q);
    }
    out
}

fn badge_at(app: &ZebraApp, p: Pos2) -> Option<usize> {
    // Later steps sit on top.
    badge_positions(app).iter().enumerate().rev().find(|(_, b)| b.distance(p) < 15.0).map(|(i, _)| i)
}

fn quad_screen(app: &ZebraApp, corners: [Vec2; 4]) -> Vec<Pos2> {
    corners.iter().map(|c| app.canvas.w2s(&app.editor, *c)).collect()
}

fn dashed_quad(painter: &Painter, q: &[Pos2], stroke: Stroke, dash: f32, gap: f32) {
    let mut pts = q.to_vec();
    pts.push(q[0]);
    painter.extend(Shape::dashed_line(&pts, stroke, dash, gap));
}

pub fn draw_agent_presence(app: &ZebraApp, painter: &Painter) {
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
            let a = (1.0 - age / 1.2) as f32;
            let mut q = quad_screen(app, e.corners());
            q.push(q[0]);
            painter.add(Shape::line(q, Stroke::new(2.5, AGENT.gamma_multiply(a))));
        }
    }
    // Agent cursor with name label.
    if let Some(c) = ed.agent_cursor {
        if app.agent_connected() || ed.activity.back().is_some_and(|a| a.origin.is_agent() && now - a.time < 10.0) {
            let p = app.canvas.w2s(ed, c);
            if app.canvas.rect.expand(-2.0).contains(p) {
                let pts = vec![p, p + egui::vec2(0.0, 18.0), p + egui::vec2(4.5, 13.5), p + egui::vec2(12.0, 13.0)];
                painter.add(Shape::convex_polygon(pts, AGENT, Stroke::new(1.5, Color32::WHITE)));
                let name = app.agent_name();
                let galley = painter.layout_no_wrap(name, FontId::proportional(12.0), Color32::WHITE);
                let r = ERect::from_min_size(p + egui::vec2(12.0, 16.0), galley.size() + egui::vec2(12.0, 6.0));
                painter.rect_filled(r, 6.0, AGENT);
                painter.galley(r.min + egui::vec2(6.0, 3.0), galley, Color32::WHITE);
            } else {
                // Edge indicator pointing to off-screen agent activity.
                let rect = app.canvas.rect.shrink(18.0);
                let c0 = rect.center();
                let d = (p - c0).normalized();
                let tx = if d.x.abs() > 1e-6 { (rect.width() / 2.0) / d.x.abs() } else { f32::INFINITY };
                let ty = if d.y.abs() > 1e-6 { (rect.height() / 2.0) / d.y.abs() } else { f32::INFINITY };
                let e = c0 + d * tx.min(ty);
                let perp = egui::vec2(-d.y, d.x);
                painter.add(Shape::convex_polygon(vec![e + d * 9.0, e - d * 5.0 + perp * 7.0, e - d * 5.0 - perp * 7.0], AGENT, Stroke::NONE));
            }
        }
    }
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
    // Hover outline + frame chip.
    app.canvas.frame_chip = None;
    let sel = &ed.view.selection;
    let hover_frame = app.canvas.hover.clone().filter(|h| ed.doc.get(h).is_some_and(|e| e.is_frame()));
    if let Some(h) = &app.canvas.hover {
        if !sel.contains(h) && !path_mode {
            if let Some(e) = ed.doc.get(h) {
                let mut q = quad_screen(app, e.corners());
                q.push(q[0]);
                painter.add(Shape::line(q, Stroke::new(1.0, ACCENT.gamma_multiply(0.7))));
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
            let label = format!("{}  ›", e.label(&ed.doc));
            let galley = painter.layout_no_wrap(label, FontId::proportional(11.5), p.text);
            let r = ERect::from_min_size(tl + egui::vec2(0.0, -24.0), galley.size() + egui::vec2(14.0, 8.0));
            painter.rect(r, 6.0, p.float, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
            painter.galley(r.min + egui::vec2(7.0, 4.0), galley, p.text);
            app.canvas.frame_chip = Some((r, fid));
        }
    }

    if path_mode {
        draw_path_overlay(app, painter);
    }

    draw_agent_presence(app, painter);

    // Selection + zebra transform tool.
    if !path_mode && app.canvas.text.is_none() {
        if let Some(sb) = selection_box(app, &ed.view.selection.clone()) {
            draw_transform_tool(app, painter, &sb);
        }
    }

    // Snapping guides.
    for g in &app.canvas.guides {
        let a = app.canvas.w2s(ed, g[0]);
        let b = app.canvas.w2s(ed, g[1]);
        painter.line_segment([a, b], Stroke::new(1.0, GUIDE));
    }

    // Gesture previews.
    match &app.canvas.gesture {
        Gesture::Marquee { start, .. } => {
            if let Some(cur) = painter.ctx().input(|i| i.pointer.hover_pos()) {
                let r = ERect::from_two_pos(*start, cur);
                painter.rect(r, 2.0, ACCENT.gamma_multiply(0.08), Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
            }
        }
        Gesture::Create { tool, start } => {
            if let Some(cur) = painter.ctx().input(|i| i.pointer.hover_pos()) {
                let a = app.canvas.w2s(ed, *start);
                let st = Stroke::new(1.5, ACCENT);
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
        Gesture::Badge { step } => {
            if let Some(cur) = painter.ctx().input(|i| i.pointer.hover_pos()) {
                draw_badge(painter, cur, *step + 1, true);
            }
        }
        _ => {}
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
    let col = if locked { Color32::from_gray(150) } else { ACCENT };
    let mut q = sb.screen.to_vec();
    q.push(q[0]);
    painter.add(Shape::line(q, Stroke::new(1.5, col)));
    if locked {
        let c = sb.screen[0];
        painter.text(c + egui::vec2(2.0, -4.0), Align2::LEFT_BOTTOM, "locked", FontId::proportional(11.0), col);
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
    let fill = if active { ACCENT } else { PATH };
    painter.circle(p, 13.0, fill, Stroke::new(2.0, Color32::WHITE));
    painter.text(p, Align2::CENTER_CENTER, n.to_string(), style::bold(12.0), Color32::WHITE);
}

fn draw_path_overlay(app: &ZebraApp, painter: &Painter) {
    // Dim content slightly so the path stands out.
    painter.rect_filled(app.canvas.rect, 0.0, Color32::from_white_alpha(70));
    let ed = &app.editor;
    // Outline every step target.
    for s in &ed.doc.path {
        if let StepTarget::Element { id } = &s.target {
            if let Some(e) = ed.doc.get(id) {
                let q = quad_screen(app, e.corners());
                dashed_quad(painter, &q, Stroke::new(1.0, PATH.gamma_multiply(0.6)), 4.0, 4.0);
            }
        }
    }
    let pos = badge_positions(app);
    if pos.len() >= 2 {
        // Gentle curve between consecutive badges.
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
            painter.add(Shape::line(pts, Stroke::new(2.5, PATH.gamma_multiply(0.75))));
        }
    }
    let selected = app.selected_step;
    for (i, p) in pos.iter().enumerate() {
        draw_badge(painter, *p, i + 1, selected == Some(i) || app.canvas.step_hover == Some(i));
        let builds = ed.doc.path[i].builds.len();
        if builds > 0 {
            painter.text(*p + egui::vec2(12.0, -12.0), Align2::LEFT_BOTTOM, format!("+{builds}"), FontId::proportional(10.0), PATH);
        }
    }
}
