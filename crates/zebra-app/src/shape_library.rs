//! The Shapes & Icons library: a floating panel with a search field and a
//! grid of tiles. Clicking a tile inserts it in the middle of the view (or,
//! when opened from an icon's "Replace…", swaps that icon).
use egui::{Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2 as EVec2};
use kurbo::{Affine, BezPath, PathEl};
use zebra_core::{Command, ElementId, ElementPatch, ShapeKind, ViewCommand};

use crate::app::ZebraApp;
use crate::style::Palette;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Shapes,
    Icons,
}

#[derive(Default)]
pub struct Library {
    pub open: bool,
    pub tab: Tab,
    pub query: String,
    /// Swap this icon element instead of inserting.
    pub replace: Option<ElementId>,
    /// Inserts since opening, to fan new items out instead of stacking them.
    inserted: u32,
    focus_search: bool,
}

impl Library {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.replace = None;
        self.focus_search = self.open;
        self.inserted = 0;
    }
    pub fn open_replace(&mut self, id: ElementId) {
        self.open = true;
        self.tab = Tab::Icons;
        self.replace = Some(id);
        self.focus_search = true;
    }
}

const TILE: f32 = 52.0;
const COLS: usize = 6;

/// Default box for a new shape, in screen points.
fn shape_size(k: ShapeKind) -> (f64, f64) {
    use ShapeKind as K;
    match k {
        k if k.is_line() => (200.0, 24.0),
        K::Rectangle | K::RoundedRect | K::Pill | K::Callout | K::SpeechBubble | K::ThoughtBubble | K::Cloud | K::Document | K::Parallelogram
        | K::Trapezoid | K::Chevron | K::Tag | K::BlockArrowRight | K::BlockArrowLeft | K::BlockArrowBoth | K::Delay | K::Cylinder => {
            if matches!(k, K::Cylinder) {
                (130.0, 170.0)
            } else {
                (220.0, 140.0)
            }
        }
        K::BlockArrowUp | K::BlockArrowDown => (120.0, 190.0),
        K::StickyNote => (200.0, 200.0),
        _ => (160.0, 160.0),
    }
}

pub fn ui(app: &mut ZebraApp, ctx: &egui::Context, p: &Palette) {
    if !app.library.open {
        return;
    }
    let mut lib = std::mem::take(&mut app.library);
    let mut open = true;
    let canvas = app.canvas.rect;
    egui::Window::new(if lib.replace.is_some() { crate::t!("library-window-replace-icon") } else { crate::t!("library-window-title") })
        .id(egui::Id::unique("library"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos(Pos2::new(canvas.center().x - 190.0, canvas.top() + 24.0))
        .frame(crate::style::float_frame(p).inner_margin(10))
        .show(ctx, |ui| {
            ui.set_width(COLS as f32 * (TILE + 4.0));
            if lib.replace.is_none() {
                let before = lib.tab;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut lib.tab, Tab::Shapes, crate::t!("library-tab-shapes"));
                    ui.selectable_value(&mut lib.tab, Tab::Icons, crate::t!("library-tab-icons"));
                });
                lib.focus_search |= lib.tab != before;
            }
            let hint = if lib.tab == Tab::Shapes { crate::t!("library-search-shapes-hint") } else { crate::t!("library-search-icons-hint") };
            let r = ui.add(egui::TextEdit::singleline(&mut lib.query).hint_text(hint).desired_width(f32::INFINITY));
            if lib.focus_search {
                r.request_focus();
                lib.focus_search = false;
            }
            ui.add_space(4.0);
            match lib.tab {
                Tab::Shapes => shapes_grid(app, &mut lib, ui, p),
                Tab::Icons => icons_grid(app, &mut lib, ui, p),
            }
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    if !open {
        lib.open = false;
        lib.replace = None;
    }
    app.library = lib;
}

fn tile(ui: &mut egui::Ui, tip: &str, p: &Palette) -> (Rect, egui::Response) {
    let (r, resp) = ui.allocate_exact_size(EVec2::splat(TILE), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 6.0, p.hover);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    (r, resp.on_hover_text(tip))
}

fn shapes_grid(app: &mut ZebraApp, lib: &mut Library, ui: &mut egui::Ui, p: &Palette) {
    let q = lib.query.to_lowercase();
    let kinds: Vec<ShapeKind> = ShapeKind::ALL.iter().copied().filter(|k| q.is_empty() || k.label().to_lowercase().contains(q.trim())).collect();
    if kinds.is_empty() {
        ui.label(RichText::new(crate::t!("library-no-shapes")).color(p.weak));
        return;
    }
    let line = ui.visuals().text_color();
    egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
        egui::Grid::new("shape-grid").spacing([4.0, 4.0]).show(ui, |ui| {
            for (i, &k) in kinds.iter().enumerate() {
                let (r, resp) = tile(ui, k.label(), p);
                paint_shape(ui.painter(), r.shrink(9.0), k, line);
                if resp.clicked() {
                    insert_shape(app, lib, k);
                }
                if i % COLS == COLS - 1 {
                    ui.end_row();
                }
            }
        });
    });
}

fn icons_grid(app: &mut ZebraApp, lib: &mut Library, ui: &mut egui::Ui, p: &Palette) {
    let icons = zebra_core::icons::search(&lib.query, usize::MAX);
    if icons.is_empty() {
        ui.label(RichText::new(crate::t!("library-no-icons")).color(p.weak));
        return;
    }
    let color = ui.visuals().text_color();
    let rows = icons.len().div_ceil(COLS);
    egui::ScrollArea::vertical().max_height(330.0).show_rows(ui, TILE + 4.0, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for icon in icons.iter().skip(row * COLS).take(COLS) {
                    let (r, resp) = tile(ui, icon.name, p);
                    paint_icon(ui.painter(), r.shrink(13.0), icon.name, color);
                    if resp.clicked() {
                        insert_icon(app, lib, icon.name);
                    }
                }
            });
        }
    });
    ui.add_space(2.0);
    ui.label(RichText::new(crate::t!("library-icons-count", count = icons.len())).size(11.0).color(p.weak));
}

/// Centre of the view, fanned out a little for each insert.
fn drop_point(app: &ZebraApp, lib: &Library) -> (f64, f64) {
    let cam = &app.editor.view.camera;
    let k = (lib.inserted % 8) as f64 * 24.0 / cam.zoom;
    (cam.cx + k, cam.cy + k)
}

fn insert_shape(app: &mut ZebraApp, lib: &mut Library, k: ShapeKind) {
    let zoom = app.editor.view.camera.zoom;
    let (w, h) = shape_size(k);
    let (x, y) = drop_point(app, lib);
    let out = app.apply(Command::AddShape { shape: k, x, y, w: w / zoom, h: h / zoom, rotation: 0.0, fill: None, stroke: None, id: None });
    if let Some(o) = out {
        lib.inserted += 1;
        app.view_cmd(ViewCommand::Select { ids: o.created });
    }
}

fn insert_icon(app: &mut ZebraApp, lib: &mut Library, name: &str) {
    if let Some(id) = lib.replace.clone() {
        app.apply(Command::Update { id, patch: ElementPatch { icon: Some(name.to_string()), ..Default::default() } });
        lib.open = false;
        lib.replace = None;
        return;
    }
    let zoom = app.editor.view.camera.zoom;
    let (x, y) = drop_point(app, lib);
    let out = app.apply(Command::AddIcon { name: name.to_string(), x, y, size: 96.0 / zoom, color: None, id: None });
    if let Some(o) = out {
        lib.inserted += 1;
        app.view_cmd(ViewCommand::Select { ids: o.created });
    }
}

/// Map a centred local path into a screen rect and split it into polylines.
fn polylines(path: &BezPath, to: Affine) -> Vec<(Vec<Pos2>, bool)> {
    let mut out: Vec<(Vec<Pos2>, bool)> = Vec::new();
    let pt = |q: kurbo::Point| {
        let q = to * q;
        Pos2::new(q.x as f32, q.y as f32)
    };
    kurbo::flatten(path, 0.1, |el| match el {
        PathEl::MoveTo(q) => out.push((vec![pt(q)], false)),
        PathEl::LineTo(q) => {
            if let Some(v) = out.last_mut() {
                v.0.push(pt(q));
            }
        }
        PathEl::ClosePath => {
            if let Some(v) = out.last_mut() {
                v.1 = true;
            }
        }
        _ => {}
    });
    out
}

pub fn paint_shape(painter: &egui::Painter, r: Rect, k: ShapeKind, line: Color32) {
    let c = r.center();
    if k.is_line() {
        let (a, b) = (Pos2::new(r.left(), c.y), Pos2::new(r.right(), c.y));
        let st = Stroke::new(2.0, line);
        painter.line_segment([a, b], st);
        let h = 6.0;
        if k != ShapeKind::Line {
            painter.add(Shape::convex_polygon(vec![b, b + EVec2::new(-h, -h * 0.6), b + EVec2::new(-h, h * 0.6)], line, Stroke::NONE));
        }
        if k == ShapeKind::DoubleArrow {
            painter.add(Shape::convex_polygon(vec![a, a + EVec2::new(h, h * 0.6), a + EVec2::new(h, -h * 0.6)], line, Stroke::NONE));
        }
        return;
    }
    let (w, h) = shape_size(k);
    let s = (r.width() as f64 / w).min(r.height() as f64 / h);
    let g = zebra_render::shapes::geometry(k, w, h, 0.0);
    let to = Affine::translate((c.x as f64, c.y as f64)) * Affine::scale(s);
    // Outline tiles, like a stencil sheet. Separate segments, because
    // egui's mitred joins shoot past sharp tips (stars, sparkles).
    let st = Stroke::new(1.4, line);
    for (mut pts, closed) in polylines(&g.outline, to) {
        if closed {
            if let Some(&f) = pts.first() {
                pts.push(f);
            }
        }
        for w in pts.windows(2) {
            painter.line_segment([w[0], w[1]], st);
        }
    }
    if let Some(d) = &g.detail {
        for (pts, _) in polylines(d, to) {
            painter.add(Shape::line(pts, Stroke::new(1.0, line)));
        }
    }
}

pub fn paint_icon(painter: &egui::Painter, r: Rect, name: &str, color: Color32) {
    let Some(path) = zebra_render::shapes::icon_path(name) else { return };
    let s = r.width().min(r.height()) as f64 / 24.0;
    let to = Affine::translate((r.left() as f64, r.top() as f64)) * Affine::scale(s);
    let st = Stroke::new((2.0 * s as f32).max(1.0), color);
    for (pts, closed) in polylines(&path, to) {
        if closed {
            painter.add(Shape::closed_line(pts, st));
        } else {
            painter.add(Shape::line(pts, st));
        }
    }
}
