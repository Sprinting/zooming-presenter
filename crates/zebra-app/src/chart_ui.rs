//! Charts in the app: the toolbar's Chart menu and Insert › Chart, the
//! Chart Data window (double-click a chart, or Edit Chart Data), the Chart,
//! Axis and Series tabs, and making a chart from a table.
//!
//! As in Keynote: a chart holds its own data, typed into the Chart Data
//! window; Link to Table… makes it follow a table on the canvas instead.
//! Every change is a zebra-core chart command, one undo step each.
use egui::{Align2, Color32, Key, Pos2, Rect as ERect, Sense, Stroke, Ui};
use zebra_core::chart::{self, ChartData, ChartLook, ChartType, SeriesIn};
use zebra_core::command::{AxisAuto, AxisValue, ChartFormat, SeriesPatch};
use zebra_core::table::{self, CellInput, CellRange, CellType, DataFormat};
use zebra_core::{Command, Element, ElementId, ElementKind, Origin, Theme};

use crate::app::ZebraApp;
use crate::icons::Icon;
use crate::inspector::{Insp, Tab};
use crate::style::{self, Palette};
use crate::widgets::{self as w, Num, Seg};

const H: Origin = Origin::Human;

/// The Chart Data window: which chart, and the cell being typed into.
#[derive(Debug, Clone, Default)]
pub struct DataWindow {
    pub id: ElementId,
    /// (row, column, text) of the cell being edited.
    pub edit: Option<(usize, usize, String)>,
    /// Focus the edit field on the next frame.
    focus: bool,
}

fn chart<'a>(app: &'a ZebraApp, id: &str) -> Option<(&'a Element, &'a ChartData)> {
    let e = app.editor.doc.get(id)?;
    match &e.kind {
        ElementKind::Chart(c) => Some((e, c)),
        _ => None,
    }
}

pub fn type_name(t: ChartType) -> String {
    crate::i18n::tr(&format!("chart-type-{}", t.key().replace('_', "-")))
}

pub fn look_name(l: ChartLook) -> String {
    crate::i18n::tr(&format!("chart-look-{}", l.key()))
}

// ----------------------------------------------------------------------
// Inserting

/// A new chart of `t` with sample data in the middle of the view, its
/// data open to type over.
pub fn insert_chart(app: &mut ZebraApp, t: ChartType) {
    let at = app.canvas_center_world();
    let zoom = app.editor.view.camera.zoom.max(1e-9);
    let vis_w = app.editor.view.viewport.x / zoom;
    let scale = (1.0 / zoom).min(0.6 * vis_w / chart::DEFAULT_W);
    let cmd = Command::AddChart {
        chart_type: Some(t),
        look: None,
        data: None,
        csv: None,
        from_table: None,
        range: None,
        link: false,
        series_in: None,
        title: None,
        x: at.x,
        y: at.y,
        width: None,
        height: None,
        scale: Some(scale),
        place: None,
        name: None,
        id: None,
    };
    if let Some(out) = app.apply(cmd) {
        app.select(out.created.clone());
        if let Some(id) = out.created.first() {
            open_data(app, id);
        }
    }
}

/// A chart of the selected table (or its selected cells), linked to it,
/// beside it.
pub fn chart_from_table(app: &mut ZebraApp, t: ChartType) -> bool {
    let sel = app.selection();
    let [id] = sel.as_slice() else { return false };
    let Some(e) = app.editor.doc.get(id).filter(|e| matches!(e.kind, ElementKind::Table(_))) else { return false };
    let range = app.canvas.table.as_ref().filter(|s| s.id == *id).map(|s| s.range()).filter(|r| !r.is_single());
    let (w, h) = e.world_size();
    let scale = e.transform.scale * (h / chart::DEFAULT_H).clamp(0.25, 4.0) / e.transform.scale.max(1e-9);
    let cw = chart::DEFAULT_W * scale;
    let cmd = Command::AddChart {
        chart_type: Some(t),
        look: None,
        data: None,
        csv: None,
        from_table: Some(id.clone()),
        range,
        link: true,
        series_in: None,
        title: None,
        x: e.transform.x + w / 2.0 + cw / 2.0 + w * 0.08,
        y: e.transform.y,
        width: None,
        height: None,
        scale: Some(scale),
        place: None,
        name: None,
        id: None,
    };
    if let Some(out) = app.apply(cmd) {
        app.select(out.created);
    }
    true
}

/// Whether a table is selected (a chart can be made from it).
pub fn table_selected(app: &ZebraApp) -> bool {
    let sel = app.selection();
    sel.len() == 1 && app.editor.doc.get(&sel[0]).is_some_and(|e| matches!(e.kind, ElementKind::Table(_)))
}

// ----------------------------------------------------------------------
// Toolbar: the Chart item's menu of types

/// The colours a thumbnail is drawn in: the theme's first three series
/// colours and its text, on its background.
struct Thumb {
    c: [Color32; 3],
    axis: Color32,
    ground: Color32,
}

impl Thumb {
    fn of(th: &Theme) -> Thumb {
        let c = |k: &str| style::c32(th.color(&k.to_string()));
        Thumb { c: [c("theme:shape1"), c("theme:shape2"), c("theme:shape3")], axis: c("theme:body").gamma_multiply(0.45), ground: c("theme:background") }
    }
}

/// A small drawing of a chart type, in `r`.
fn paint_type(painter: &egui::Painter, r: ERect, t: ChartType, k: &Thumb) {
    let [a, b, g] = k.c;
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * r.width(), r.top() + y * r.height());
    let bar = |x0: f32, y0: f32, x1: f32, y1: f32, c: Color32| painter.rect_filled(ERect::from_two_pos(at(x0, y0), at(x1, y1)), 0.0, c);
    let line = |pts: &[(f32, f32)], c: Color32, w: f32| painter.add(egui::Shape::line(pts.iter().map(|(x, y)| at(*x, *y)).collect(), Stroke::new(w, c)));
    let base = || painter.line_segment([at(0.08, 0.9), at(0.92, 0.9)], Stroke::new(1.0, k.axis));
    let vals = [0.62, 0.48, 0.36, 0.55];
    let vals2 = [0.72, 0.52, 0.32, 0.64];
    match t {
        ChartType::Column | ChartType::Combo => {
            base();
            for i in 0..4 {
                let x = 0.12 + i as f32 * 0.2;
                if t == ChartType::Combo {
                    bar(x, 0.9 - vals[i] * 0.95, x + 0.15, 0.9, a);
                } else {
                    bar(x, 0.9 - vals[i], x + 0.075, 0.9, a);
                    bar(x + 0.075, 0.9 - vals2[i], x + 0.15, 0.9, b);
                }
            }
            if t == ChartType::Combo {
                line(&[(0.14, 0.2), (0.34, 0.42), (0.54, 0.62), (0.74, 0.3)], b, 2.0);
            }
        }
        ChartType::StackedColumn | ChartType::PercentColumn => {
            base();
            for i in 0..4 {
                let x = 0.14 + i as f32 * 0.2;
                let (h1, h2) = if t == ChartType::PercentColumn { (0.4, 0.4) } else { (vals[i] * 0.55, vals2[i] * 0.45) };
                bar(x, 0.9 - h1, x + 0.12, 0.9, a);
                bar(x, 0.9 - h1 - h2, x + 0.12, 0.9 - h1, b);
            }
        }
        ChartType::Bar | ChartType::StackedBar => {
            painter.line_segment([at(0.1, 0.08), at(0.1, 0.92)], Stroke::new(1.0, k.axis));
            for i in 0..4 {
                let y = 0.12 + i as f32 * 0.2;
                if t == ChartType::Bar {
                    bar(0.1, y, 0.1 + vals[i] * 1.1, y + 0.075, a);
                    bar(0.1, y + 0.075, 0.1 + vals2[i] * 1.1, y + 0.15, b);
                } else {
                    let w1 = vals[i] * 0.6;
                    bar(0.1, y, 0.1 + w1, y + 0.13, a);
                    bar(0.1 + w1, y, 0.1 + w1 + vals2[i] * 0.5, y + 0.13, b);
                }
            }
        }
        ChartType::Line | ChartType::Step => {
            base();
            let p1 = [(0.14, 0.32), (0.38, 0.45), (0.62, 0.55), (0.86, 0.36)];
            let p2 = [(0.14, 0.22), (0.38, 0.42), (0.62, 0.62), (0.86, 0.28)];
            for (p, c) in [(p1, a), (p2, b)] {
                if t == ChartType::Step {
                    let mut s = vec![];
                    for (i, q) in p.iter().enumerate() {
                        if i > 0 {
                            let mx = (p[i - 1].0 + q.0) / 2.0;
                            s.push((mx, p[i - 1].1));
                            s.push((mx, q.1));
                        }
                        s.push(*q);
                    }
                    line(&s, c, 2.0);
                } else {
                    line(&p, c, 2.0);
                }
            }
        }
        ChartType::Area | ChartType::StackedArea => {
            base();
            let top2 = [(0.12, 0.2), (0.4, 0.38), (0.65, 0.5), (0.88, 0.26)];
            let top1 = if t == ChartType::StackedArea { [(0.12, 0.55), (0.4, 0.62), (0.65, 0.7), (0.88, 0.58)] } else { [(0.12, 0.3), (0.4, 0.45), (0.65, 0.55), (0.88, 0.35)] };
            for (top, c) in [(top2, b), (top1, a)] {
                let mut pts: Vec<Pos2> = top.iter().map(|(x, y)| at(*x, *y)).collect();
                pts.push(at(0.88, 0.9));
                pts.push(at(0.12, 0.9));
                // Not convex in general: fill as a mesh of columns.
                for i in 0..3 {
                    let q = [pts[i], pts[i + 1], Pos2::new(pts[i + 1].x, at(0.0, 0.9).y), Pos2::new(pts[i].x, at(0.0, 0.9).y)];
                    painter.add(egui::Shape::convex_polygon(q.to_vec(), c, Stroke::NONE));
                }
            }
        }
        ChartType::Pie | ChartType::Donut => {
            let c = r.center();
            let rad = r.width().min(r.height()) * 0.4;
            let shares = [0.33, 0.24, 0.16, 0.27];
            let cols = [a, b, g, k.axis];
            let mut start = -std::f32::consts::FRAC_PI_2;
            for (s, col) in shares.iter().zip(cols) {
                let end = start + s * std::f32::consts::TAU;
                let n = 16;
                let mut pts = vec![c];
                for j in 0..=n {
                    let t = start + (end - start) * j as f32 / n as f32;
                    pts.push(c + egui::vec2(t.cos(), t.sin()) * rad);
                }
                painter.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
                start = end;
            }
            if t == ChartType::Donut {
                painter.circle_filled(c, rad * 0.5, k.ground);
            }
        }
        ChartType::Scatter | ChartType::Bubble => {
            base();
            let pts = [(0.2, 0.7, 0.04, a), (0.35, 0.5, 0.07, b), (0.5, 0.55, 0.05, a), (0.65, 0.3, 0.09, b), (0.8, 0.25, 0.06, a)];
            for (x, y, s, c) in pts {
                let rad = if t == ChartType::Bubble { s * r.height() } else { 0.035 * r.height() };
                painter.circle_filled(at(x, y), rad, if t == ChartType::Bubble { c.gamma_multiply(0.7) } else { c });
            }
        }
        ChartType::Radar => {
            let c = r.center();
            let rad = r.width().min(r.height()) * 0.42;
            let ang = |i: usize| -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / 5.0;
            let ring: Vec<Pos2> = (0..=5).map(|i| c + egui::vec2(ang(i).cos(), ang(i).sin()) * rad).collect();
            painter.add(egui::Shape::line(ring, Stroke::new(1.0, k.axis)));
            for (vals, col) in [([0.8, 0.6, 0.7, 0.4, 0.6], a), ([0.5, 0.9, 0.4, 0.7, 0.3], b)] {
                let pts: Vec<Pos2> = (0..=5).map(|i| c + egui::vec2(ang(i).cos(), ang(i).sin()) * rad * vals[i % 5]).collect();
                painter.add(egui::Shape::line(pts, Stroke::new(1.6, col)));
            }
        }
        ChartType::Waterfall => {
            base();
            bar(0.1, 0.4, 0.24, 0.9, a);
            bar(0.28, 0.22, 0.42, 0.4, g);
            bar(0.46, 0.22, 0.6, 0.32, Color32::from_rgb(0xc0, 0x39, 0x2b));
            bar(0.64, 0.14, 0.78, 0.32, g);
            bar(0.82, 0.14, 0.94, 0.9, a);
        }
    }
}

/// The toolbar's Chart menu: the sixteen types, drawn in the theme.
pub fn toolbar_menu(ui: &mut Ui, app: &mut ZebraApp) {
    let th = app.editor.theme();
    let k = Thumb::of(&th);
    let p = Palette::of(ui.ctx());
    let from_table = table_selected(app);
    ui.add_space(4.0);
    if from_table {
        ui.label(egui::RichText::new(crate::t!("chart-menu-from-table")).font(style::ui_font(11.0)).color(p.weak));
        ui.add_space(2.0);
    }
    let cols = 4;
    let size = egui::vec2(70.0, 52.0);
    for row in ChartType::ALL.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for t in row {
                let name = type_name(*t);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                    let painter = ui.painter();
                    painter.rect_filled(r.shrink(1.0), 4.0, k.ground);
                    paint_type(painter, r.shrink(7.0), *t, &k);
                    let border = if resp.hovered() { Stroke::new(2.0, style::accent()) } else { Stroke::new(1.0, p.border) };
                    painter.rect_stroke(r.shrink(1.0), 4.0, border, egui::StrokeKind::Inside);
                    let (nr, _) = ui.allocate_exact_size(egui::vec2(size.x, 26.0), Sense::hover());
                    let g = ui.painter().layout(name.clone(), style::ui_font(11.0), p.weak, size.x);
                    ui.painter().galley(Pos2::new(nr.center().x - g.size().x / 2.0, nr.top()), g, p.weak);
                    if resp.on_hover_text(&name).clicked() {
                        if !(from_table && chart_from_table(app, *t)) {
                            insert_chart(app, *t);
                        }
                        ui.close();
                    }
                });
            }
        });
    }
}

// ----------------------------------------------------------------------
// Canvas

/// A double click on a chart opens its data, as in Keynote.
pub fn double_click(app: &mut ZebraApp, p: Pos2) -> bool {
    let Some(id) = crate::canvas::hit(app, p, false) else { return false };
    if chart(app, &id).is_none() {
        return false;
    }
    if app.selection() != vec![id.clone()] {
        app.select(vec![id.clone()]);
    }
    open_data(app, &id);
    true
}

/// Esc while Chart Data is open: out of the cell without keeping what was
/// typed, else the window closes. Called before the canvas sees the key,
/// so the selection stays.
pub fn escape(app: &mut ZebraApp, ctx: &egui::Context) {
    let Some(win) = app.chart_data.as_mut() else { return };
    if !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        return;
    }
    if win.edit.take().is_none() {
        app.chart_data = None;
    }
}

pub fn open_data(app: &mut ZebraApp, id: &str) {
    app.chart_data = Some(DataWindow { id: id.to_string(), edit: None, focus: false });
}

// ----------------------------------------------------------------------
// The Chart Data window

/// Width of the row-number column and of the cells (pt).
const NUM_W: f32 = 30.0;
const CELL_W: f32 = 104.0;
const FIRST_W: f32 = 120.0;
const ROW_H: f32 = 26.0;

fn commit(app: &mut ZebraApp, id: &str, r: usize, c: usize, text: String) {
    let same = chart(app, id).and_then(|(_, ch)| ch.data.cell(r, c)).is_some_and(|cell| cell.input == text);
    if same {
        return;
    }
    app.apply(Command::SetChartData { id: id.to_string(), at: table::cell_name(r, c), values: Some(vec![vec![CellInput::Text(text)]]), data: None, rows: None, columns: None });
}

/// The grid with row `r` or column `c` taken out (formulas keep their
/// text).
fn without(data: &table::TableData, row: Option<usize>, col: Option<usize>) -> Vec<Vec<CellInput>> {
    data.cells
        .iter()
        .enumerate()
        .filter(|(r, _)| Some(*r) != row)
        .map(|(_, cells)| cells.iter().enumerate().filter(|(c, _)| Some(*c) != col).map(|(_, x)| CellInput::Text(x.input.clone())).collect())
        .collect()
}

pub fn window(app: &mut ZebraApp, ctx: &egui::Context) {
    let Some(win) = app.chart_data.clone() else { return };
    let Some((e, c)) = chart(app, &win.id).map(|(e, c)| (e.clone(), c.clone())) else {
        app.chart_data = None;
        return;
    };
    let mut open = true;
    let p = Palette::of(ctx);
    let mut win = win;
    let doc_tables: Vec<(ElementId, String)> = app
        .editor
        .doc
        .ordered()
        .filter(|t| matches!(t.kind, ElementKind::Table(_)))
        .map(|t| (t.id.clone(), crate::words::element_label(t, &app.editor.doc)))
        .collect();
    let mut actions: Vec<Command> = vec![];
    let mut show_table: Option<ElementId> = None;
    let src = c.source(&app.editor.doc);
    let linked = src.linked;
    let (t, range) = (src.table.clone(), src.range);
    let link_name = c.link.as_ref().and_then(|l| doc_tables.iter().find(|(i, _)| *i == l.table)).map(|(_, n)| n.clone()).unwrap_or_default();
    let plot = c.plot(&app.editor.doc);
    let th = app.editor.theme();
    // Tall enough for every row up to a dozen; over the lower half of the
    // canvas, as Keynote puts it under the chart.
    let rows = c.source(&app.editor.doc).range.rows().min(12) as f32;
    let height = 150.0 + if linked { 34.0 } else { 0.0 } + rows * ROW_H;
    let canvas = app.canvas.rect;
    let at = Pos2::new(canvas.center().x - 280.0, (canvas.bottom() - height - 40.0).max(canvas.top() + 24.0));
    egui::Window::new(crate::t!("chart-data-title"))
        .id(egui::Id::unique("zebra-chart-data"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_pos(at)
        .default_size(egui::vec2(560.0, height))
        .min_width(420.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add_enabled_ui(!linked, |ui| {
                    if w::icon_text_button(ui, Icon::Plus, &crate::t!("chart-data-row"), !linked).clicked() {
                        actions.push(Command::SetChartData { id: win.id.clone(), at: "A1".into(), values: None, data: None, rows: Some(c.data.n_rows() + 1), columns: None });
                    }
                    if w::icon_text_button(ui, Icon::Plus, &crate::t!("chart-data-column"), !linked).clicked() {
                        actions.push(Command::SetChartData { id: win.id.clone(), at: "A1".into(), values: None, data: None, rows: None, columns: Some(c.data.n_cols() + 1) });
                    }
                });
                let rows_l = crate::t!("chart-data-series-rows");
                let cols_l = crate::t!("chart-data-series-columns");
                let mut si = c.series_in;
                if w::segmented(ui, &mut si, &[(SeriesIn::Rows, Seg::Text(&rows_l)), (SeriesIn::Columns, Seg::Text(&cols_l))], None).changed() {
                    actions.push(Command::FormatChart { id: win.id.clone(), format: ChartFormat { series_in: Some(si), ..Default::default() } });
                }
                let label = if linked { link_name.clone() } else { crate::t!("chart-data-link") };
                w::popup_button(ui, "chart-link", &label, 170.0, |ui| {
                    if doc_tables.is_empty() {
                        w::menu_row(ui, None, &crate::t!("chart-data-no-tables"), None, false);
                    }
                    for (tid, name) in &doc_tables {
                        let on = c.linked_table() == Some(tid.as_str());
                        if w::menu_check(ui, on, name, None).clicked() && !on {
                            actions.push(Command::LinkChart { id: win.id.clone(), table: tid.clone(), range: None });
                        }
                    }
                    if linked {
                        w::menu_separator(ui);
                        if w::menu_item(ui, &crate::t!("chart-data-unlink"), None).clicked() {
                            actions.push(Command::UnlinkChart { id: win.id.clone() });
                        }
                    }
                });
            });
            ui.add_space(4.0);
            if linked {
                ui.horizontal(|ui| {
                    w::caption(ui, crate::t!("chart-data-linked-note", table = link_name.as_str()));
                    if w::push_button(ui, &crate::t!("chart-data-show-table")).clicked() {
                        show_table = c.link.as_ref().map(|l| l.table.clone());
                    }
                    if w::push_button(ui, &crate::t!("chart-data-unlink")).clicked() {
                        actions.push(Command::UnlinkChart { id: win.id.clone() });
                    }
                });
                ui.add_space(4.0);
            }
            if plot.cut {
                w::caption(ui, cut_note(&plot));
                ui.add_space(4.0);
            }
            grid(ui, &p, &th, &c, &t, range, linked, &mut win, &mut actions);
        });
    if !open {
        if let Some((r, cc, text)) = win.edit.take() {
            commit(app, &win.id, r, cc, text);
        }
        app.chart_data = None;
    } else {
        app.chart_data = Some(win);
    }
    for a in actions {
        app.apply(a);
    }
    if let Some(tid) = show_table {
        app.select(vec![tid]);
        app.queued.push(crate::actions::Action::ZoomToSelection);
    }
    let _ = e;
}

#[allow(clippy::too_many_arguments)]
fn grid(ui: &mut Ui, p: &Palette, th: &Theme, c: &ChartData, t: &table::TableData, range: CellRange, linked: bool, win: &mut DataWindow, actions: &mut Vec<Command>) {
    let rows = range.rows();
    let cols = range.cols();
    let width = |cc: usize| if cc == 0 { FIRST_W } else { CELL_W };
    let total_w = NUM_W + (0..cols).map(width).sum::<f32>();
    let series_color = |k: usize| {
        let own = c.series.get(k).and_then(|f| f.color.as_ref()).map(|s| th.color(s));
        style::c32(own.unwrap_or_else(|| th.color(&format!("theme:shape{}", k % 6 + 1))))
    };
    let per = c.chart_type.values_per_series();
    // Which data column (or row) is which series' colour.
    let names = c.names(&chart::Source { table: t, range, linked });
    // Lanes are columns (series in columns) or rows; the names lane is
    // first when there is one.
    let first_lane = match c.series_in {
        SeriesIn::Columns => names.1 as usize,
        SeriesIn::Rows => names.0 as usize,
    };
    let swatch_for = |lane: usize| -> Option<Color32> { (lane >= first_lane).then(|| series_color((lane - first_lane) / per)) };
    let mut move_to: Option<(usize, usize)> = None;
    egui::ScrollArea::both().auto_shrink([false, false]).show_rows(ui, ROW_H, rows, |ui, shown| {
        for r in shown {
            let (row_rect, _) = ui.allocate_exact_size(egui::vec2(total_w, ROW_H), Sense::hover());
            let painter = ui.painter().clone();
            let header = r == 0 && names.0;
            if header {
                painter.rect_filled(row_rect, 0.0, p.control);
            }
            // The row number (data rows count from 1).
            let num = ERect::from_min_size(row_rect.min, egui::vec2(NUM_W, ROW_H));
            if !header {
                painter.text(num.center(), Align2::CENTER_CENTER, r.to_string(), style::ui_font(11.0), p.weak);
                if !linked {
                    let resp = ui.interact(num, ui.scope_id().with(("chart-row", r)), Sense::click());
                    resp.context_menu(|ui| {
                        if w::menu_item(ui, &crate::t!("table-delete-row"), None).clicked() && t.n_rows() > 1 {
                            actions.push(Command::SetChartData { id: win.id.clone(), at: "A1".into(), values: None, data: Some(without(t, Some(r), None)), rows: None, columns: None });
                        }
                    });
                }
            }
            let mut x = num.right();
            for cc in 0..cols {
                let (tr, tc) = (range.r0 + r, range.c0 + cc);
                let cell = ERect::from_min_size(Pos2::new(x, row_rect.top()), egui::vec2(width(cc), ROW_H));
                x += width(cc);
                painter.line_segment([cell.left_top(), cell.left_bottom()], Stroke::new(0.5, p.border));
                let editing = win.edit.as_ref().is_some_and(|(er, ec, _)| *er == tr && *ec == tc);
                if editing {
                    let (_, _, text) = win.edit.as_mut().expect("editing");
                    // Read the keys before the field takes them.
                    let (enter, tab, shift) = ui.input(|i| {
                        let held = i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Enter | Key::Tab, pressed: true, modifiers, .. } if modifiers.shift));
                        (i.key_pressed(Key::Enter), i.key_pressed(Key::Tab), i.modifiers.shift || held)
                    });
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cell.shrink(2.0)));
                    let out = egui::TextEdit::singleline(text).font(style::ui_font(13.0)).desired_width(cell.width() - 6.0).show(&mut child);
                    if win.focus {
                        out.response.request_focus();
                        win.focus = false;
                    }
                    if out.response.lost_focus() || enter || tab {
                        let (er, ec, text) = win.edit.take().expect("editing");
                        let same = t.cell(er, ec).is_some_and(|x| x.input == text) || (t.cell(er, ec).is_none() && text.is_empty());
                        if !same {
                            actions.push(Command::SetChartData { id: win.id.clone(), at: table::cell_name(er, ec), values: Some(vec![vec![CellInput::Text(text)]]), data: None, rows: None, columns: None });
                        }
                        // Return goes down, Tab across; with Shift, back.
                        move_to = match (enter, tab, shift) {
                            (true, _, false) => Some((er + 1, ec)),
                            (true, _, true) => er.checked_sub(1).map(|r| (r, ec)),
                            (_, true, false) => Some((er, ec + 1)),
                            (_, true, true) => ec.checked_sub(1).map(|c| (er, c)),
                            _ => None,
                        };
                    }
                    continue;
                }
                let shown = t.shown(tr, tc, None);
                let numeric = shown.numeric;
                let bold = cc == 0 && !header && names.1;
                let mut text_x = cell.left() + 8.0;
                if header {
                    let swatch = if c.series_in == SeriesIn::Columns { swatch_for(cc) } else { None };
                    if let Some(col) = swatch {
                        let sw = ERect::from_center_size(Pos2::new(cell.left() + 14.0, cell.center().y), egui::vec2(10.0, 10.0));
                        painter.rect_filled(sw, 1.5, col);
                        text_x += 14.0;
                    }
                } else if cc == 0 && c.series_in == SeriesIn::Rows && names.1 {
                    if let Some(col) = swatch_for(r) {
                        let sw = ERect::from_center_size(Pos2::new(cell.left() + 14.0, cell.center().y), egui::vec2(10.0, 10.0));
                        painter.rect_filled(sw, 1.5, col);
                        text_x += 14.0;
                    }
                }
                let font = if bold || header { style::semibold(13.0) } else { style::ui_font(13.0) };
                let clip = painter.with_clip_rect(cell.shrink(2.0));
                if numeric {
                    clip.text(Pos2::new(cell.right() - 8.0, cell.center().y), Align2::RIGHT_CENTER, &shown.text, font, p.text);
                } else {
                    clip.text(Pos2::new(text_x, cell.center().y), Align2::LEFT_CENTER, &shown.text, font, p.text);
                }
                let resp = ui.interact(cell, ui.scope_id().with(("chart-cell", tr, tc)), if linked { Sense::hover() } else { Sense::click() });
                // For VoiceOver (and tests): the cell's name and what it shows.
                resp.widget_info(|| {
                    let mut info = egui::WidgetInfo::labeled(egui::Role::Cell, !linked, table::cell_name(tr, tc));
                    info.current_text_value = Some(shown.text.clone());
                    info
                });
                if !linked {
                    if resp.hovered() {
                        painter.rect_stroke(cell.shrink(1.0), 2.0, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                    }
                    if resp.clicked() {
                        let input = t.cell(tr, tc).map(|x| x.input.clone()).unwrap_or_default();
                        win.edit = Some((tr, tc, input));
                        win.focus = true;
                    }
                    if header {
                        resp.context_menu(|ui| {
                            if w::menu_item(ui, &crate::t!("table-delete-column"), None).clicked() && t.n_cols() > 1 {
                                actions.push(Command::SetChartData { id: win.id.clone(), at: "A1".into(), values: None, data: Some(without(t, None, Some(tc))), rows: None, columns: None });
                            }
                        });
                    }
                }
            }
            painter.line_segment([row_rect.left_bottom(), row_rect.right_bottom()], Stroke::new(0.5, p.border));
        }
    });
    // Return and Tab go on to the next cell, inside the grid.
    if let Some((r, cc)) = move_to {
        if r < t.n_rows() && cc < t.n_cols() {
            win.edit = Some((r, cc, t.cell(r, cc).map(|x| x.input.clone()).unwrap_or_default()));
            win.focus = true;
        }
    }
}

// ----------------------------------------------------------------------
// Inspector: Chart, Axis and Series tabs

pub fn tabs() -> Vec<Tab> {
    vec![Tab::Chart, Tab::Axis, Tab::Series, Tab::Arrange, Tab::Path]
}

/// Apply a change from the inspector; live controls join one undo step.
fn format(insp: &mut Insp, f: ChartFormat) {
    if insp.gesture && !insp.app.editor.in_transaction() {
        let _ = insp.app.editor.begin(crate::t!("history-format-chart"), H);
    }
    let id = insp.el.id.clone();
    insp.app.apply(Command::FormatChart { id, format: f });
}

/// A small column chart in look `l`, in the theme.
fn look_thumb(ui: &mut Ui, th: &Theme, l: ChartLook, size: egui::Vec2, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let name = look_name(l);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &name));
    let k = Thumb::of(th);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect.shrink(2.0), 3.0, k.ground);
    if l == ChartLook::Framed {
        painter.rect_filled(rect.shrink(5.0), 2.0, style::c32(th.color(&th.frame_fill)));
    }
    let inner = rect.shrink(9.0);
    let at = |x: f32, y: f32| Pos2::new(inner.left() + x * inner.width(), inner.top() + y * inner.height());
    let (a, b) = if l == ChartLook::Tints { (k.c[0].gamma_multiply(0.45), k.c[0]) } else { (k.c[0], k.c[1]) };
    if l != ChartLook::Labelled {
        for y in [0.25, 0.55] {
            painter.line_segment([at(0.0, y), at(1.0, y)], Stroke::new(0.5, k.axis.gamma_multiply(0.5)));
        }
    }
    painter.line_segment([at(0.0, 0.95), at(1.0, 0.95)], Stroke::new(0.8, k.axis));
    let v1 = [0.7, 0.55, 0.42, 0.62];
    let v2 = [0.82, 0.6, 0.38, 0.7];
    for i in 0..4 {
        let x = 0.04 + i as f32 * 0.25;
        for (j, (v, c)) in [(v1[i], a), (v2[i], b)].into_iter().enumerate() {
            let x0 = x + j as f32 * 0.09;
            let r = ERect::from_two_pos(at(x0, 0.95 - v), at(x0 + 0.09, 0.95));
            match l {
                ChartLook::Rounded => {
                    painter.rect_filled(r, 2.0, c);
                }
                ChartLook::Outline => {
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, c), egui::StrokeKind::Inside);
                }
                _ => {
                    painter.rect_filled(r, 0.0, c);
                }
            }
            if l == ChartLook::Labelled {
                // A value over each bar, as a short dash.
                let y = r.top() - 3.0;
                painter.line_segment([Pos2::new(r.left(), y), Pos2::new(r.right(), y)], Stroke::new(1.2, k.axis));
            }
        }
    }
    let p = Palette::of(ui.ctx());
    let border = if on { Stroke::new(2.0, style::accent()) } else if resp.hovered() { Stroke::new(1.0, p.weak) } else { Stroke::new(1.0, p.border) };
    painter.rect_stroke(rect.shrink(1.0), 4.0, border, egui::StrokeKind::Inside);
    resp.on_hover_text(name)
}

fn percent_row(insp: &mut Insp, ui: &mut Ui, label: &str, value: f64, max: f64, set: impl Fn(f64) -> ChartFormat) {
    w::row_w(ui, label, 120.0, |ui| {
        let mut v = value;
        let r = w::number_field(ui, &mut v, Num::pct().range(0.0, max).step(0.05).width(64.0));
        insp.track(&r);
        if r.changed() {
            format(insp, set(v.clamp(0.0, max)));
        }
    });
}

pub fn chart_tab(insp: &mut Insp, ui: &mut Ui) {
    let el = insp.el.clone();
    let ElementKind::Chart(c) = &el.kind else { return };
    ui.add_space(4.0);
    let wdt = ui.available_width();
    if w::push_button_ext(ui, &crate::t!("chart-edit-data"), w::Size::Regular, true, Some(wdt)).clicked() {
        open_data(insp.app, &el.id);
    }
    let plot = c.plot(&insp.app.editor.doc);
    if plot.cut {
        ui.add_space(4.0);
        w::caption(ui, cut_note(&plot));
    }
    w::section(ui, &crate::t!("chart-section-style"), |ui| {
        let th = insp.theme.clone();
        let cols = 3;
        let gap = 6.0;
        let cw = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
        for row in ChartLook::ALL.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for l in row {
                    if look_thumb(ui, &th, *l, egui::vec2(cw, (cw * 0.62).round()), c.look == *l).clicked() && c.look != *l {
                        format(insp, ChartFormat { look: Some(*l), ..Default::default() });
                    }
                }
            });
        }
    });
    w::section(ui, &crate::t!("chart-section-options"), |ui| {
        let mut titled = !c.title.is_empty();
        if w::checkbox(ui, &mut titled, &crate::t!("chart-title")).changed() {
            let t = if titled { crate::t!("chart-default-title") } else { String::new() };
            format(insp, ChartFormat { title: Some(t), ..Default::default() });
        }
        if titled {
            let mut t = c.title.clone();
            let r = w::text_field(ui, &mut t, ui.available_width(), &crate::t!("chart-title-hint"));
            insp.track(&r);
            if r.changed() && !t.is_empty() {
                format(insp, ChartFormat { title: Some(t), ..Default::default() });
            }
        }
        let mut legend = c.legend;
        if w::checkbox(ui, &mut legend, &crate::t!("chart-legend")).changed() {
            format(insp, ChartFormat { legend: Some(legend), ..Default::default() });
        }
        // The layout lists so many; say so rather than drop them quietly.
        let plot = c.plot(&insp.app.editor.doc);
        let entries = if c.chart_type.is_round() { plot.categories.len() } else { plot.series.len() };
        if legend && entries > chart::LEGEND_ENTRIES {
            let count = chart::LEGEND_ENTRIES as u64;
            w::caption(ui, crate::t!("chart-legend-cut", count = count));
        }
        let mut labels = c.value_labels || c.look == ChartLook::Labelled;
        if w::checkbox(ui, &mut labels, &crate::t!("chart-value-labels")).changed() {
            format(insp, ChartFormat { value_labels: Some(labels), ..Default::default() });
        }
        let mut border = c.border;
        if w::checkbox(ui, &mut border, &crate::t!("chart-border")).changed() {
            format(insp, ChartFormat { border: Some(border), ..Default::default() });
        }
        w::row_w(ui, &crate::t!("chart-font-size"), 120.0, |ui| {
            let label = |f: f64| if (f - 1.0).abs() < 1e-9 { crate::t!("chart-font-size-theme") } else { format!("{}%", (f * 100.0).round()) };
            w::popup_button(ui, "chart-font-size", &label(c.font_size), 110.0, |ui| {
                for f in [0.6, 0.75, 0.9, 1.0, 1.15, 1.3, 1.5, 2.0] {
                    if w::menu_check(ui, (c.font_size - f).abs() < 1e-9, &label(f), None).clicked() {
                        format(insp, ChartFormat { font_size: Some(f), ..Default::default() });
                    }
                }
            });
        });
    });
    w::section(ui, &crate::t!("chart-section-type"), |ui| {
        let wdt = ui.available_width();
        w::popup_button(ui, "chart-type", &type_name(c.chart_type), wdt, |ui| {
            for t in ChartType::ALL {
                if w::menu_check(ui, c.chart_type == *t, &type_name(*t), None).clicked() && c.chart_type != *t {
                    format(insp, ChartFormat { chart_type: Some(*t), ..Default::default() });
                }
            }
        });
    });
    if c.chart_type.has_bars() {
        w::section(ui, &crate::t!("chart-section-gap"), |ui| {
            if !matches!(c.chart_type, ChartType::StackedColumn | ChartType::PercentColumn | ChartType::StackedBar | ChartType::Waterfall) {
                percent_row(insp, ui, &crate::t!("chart-gap-bars"), c.gap, 1.0, |v| ChartFormat { gap: Some(v), ..Default::default() });
            }
            percent_row(insp, ui, &crate::t!("chart-gap-sets"), c.gap_sets, 5.0, |v| ChartFormat { gap_sets: Some(v), ..Default::default() });
        });
    }
    if c.chart_type == ChartType::Donut {
        w::section(ui, "", |ui| {
            w::row_w(ui, &crate::t!("chart-hole"), 120.0, |ui| {
                let mut v = c.hole;
                let r = w::number_field(ui, &mut v, Num::pct().range(0.1, 0.9).step(0.05).width(64.0));
                insp.track(&r);
                if r.changed() {
                    format(insp, ChartFormat { hole: Some(v.clamp(0.1, 0.9)), ..Default::default() });
                }
            });
        });
    }
}

fn format_name(k: CellType) -> String {
    match k {
        CellType::Number => crate::t!("table-format-number"),
        CellType::Currency => crate::t!("table-format-currency"),
        CellType::Percentage => crate::t!("table-format-percentage"),
        _ => crate::t!("table-format-automatic"),
    }
}

/// A bound field: a number, or empty for automatic.
fn bound_row(insp: &mut Insp, ui: &mut Ui, id: &str, label: &str, now: Option<f64>, set: impl Fn(AxisValue) -> ChartFormat) {
    w::row_w(ui, label, 120.0, |ui| {
        let key = egui::Id::unique(("chart-bound", id, insp.el.id.clone()));
        let mut text: String = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| now.map(table::plain_number).unwrap_or_default());
        let r = w::text_field(ui, &mut text, 96.0, &crate::t!("chart-axis-auto"));
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(key, text.clone()));
        }
        if r.lost_focus() {
            ui.data_mut(|d| d.remove::<String>(key));
            let t = text.trim().replace(',', "");
            let v = if t.is_empty() { Some(AxisValue::Auto(AxisAuto::Auto)) } else { t.parse::<f64>().ok().filter(|v| v.is_finite()).map(AxisValue::Value) };
            match v {
                Some(v) if v.get() != now || (v.get().is_none() && now.is_some()) => format(insp, set(v)),
                Some(_) => {}
                // Letters: the field goes back to what it was.
                None => insp.app.note(crate::t!("chart-axis-not-a-number")),
            }
        }
    });
}

pub fn axis_tab(insp: &mut Insp, ui: &mut Ui) {
    let el = insp.el.clone();
    let ElementKind::Chart(c) = &el.kind else { return };
    if !c.chart_type.has_axes() {
        ui.add_space(8.0);
        w::caption(ui, crate::t!("chart-axis-none"));
        return;
    }
    w::section(ui, &crate::t!("chart-section-value-axis"), |ui| {
        bound_row(insp, ui, "min", &crate::t!("chart-axis-min"), c.axis_min, |v| ChartFormat { axis_min: Some(v), ..Default::default() });
        bound_row(insp, ui, "max", &crate::t!("chart-axis-max"), c.axis_max, |v| ChartFormat { axis_max: Some(v), ..Default::default() });
        bound_row(insp, ui, "step", &crate::t!("chart-axis-step"), c.axis_step, |v| ChartFormat { axis_step: Some(v), ..Default::default() });
        if c.step_ignored(&c.plot(&insp.app.editor.doc)) {
            let count = chart::MAX_AXIS_STEPS as u64;
            w::caption(ui, crate::t!("chart-axis-step-ignored", count = count));
        }
        let mut grid = c.gridlines;
        if w::checkbox(ui, &mut grid, &crate::t!("chart-gridlines")).changed() {
            format(insp, ChartFormat { gridlines: Some(grid), ..Default::default() });
        }
    });
    w::section(ui, &crate::t!("chart-section-number"), |ui| {
        let n = c.number.clone();
        w::popup_button(ui, "chart-number", &format_name(n.kind), ui.available_width(), |ui| {
            for k in [CellType::Automatic, CellType::Number, CellType::Currency, CellType::Percentage] {
                if w::menu_check(ui, n.kind == k, &format_name(k), None).clicked() {
                    format(insp, ChartFormat { number: Some(DataFormat { kind: k, ..n.clone() }), ..Default::default() });
                }
            }
        });
        if n.kind != CellType::Automatic {
            w::row_w(ui, &crate::t!("table-decimals"), 120.0, |ui| {
                let mut d = n.decimals.unwrap_or(0) as f64;
                let r = w::number_field(ui, &mut d, Num::default().range(0.0, 10.0).step(1.0).width(48.0));
                if r.changed() {
                    format(insp, ChartFormat { number: Some(DataFormat { decimals: Some(d.round().clamp(0.0, 10.0) as u8), ..n.clone() }), ..Default::default() });
                }
            });
            let mut th = n.thousands;
            if n.kind != CellType::Percentage && w::checkbox(ui, &mut th, &crate::t!("table-thousands")).changed() {
                format(insp, ChartFormat { number: Some(DataFormat { thousands: th, ..n.clone() }), ..Default::default() });
            }
        }
    });
    w::section(ui, &crate::t!("chart-section-axis-titles"), |ui| {
        for (which, label, now) in [(0, crate::t!("chart-axis-title-x"), c.x_title.clone()), (1, crate::t!("chart-axis-title-y"), c.y_title.clone())] {
            w::row_w(ui, &label, 120.0, |ui| {
                let mut t = now.clone();
                let r = w::text_field(ui, &mut t, ui.available_width(), &crate::t!("chart-axis-title-hint"));
                insp.track(&r);
                if r.changed() {
                    let f = if which == 0 { ChartFormat { x_title: Some(t), ..Default::default() } } else { ChartFormat { y_title: Some(t), ..Default::default() } };
                    format(insp, f);
                }
            });
        }
    });
}

pub fn series_tab(insp: &mut Insp, ui: &mut Ui) {
    let el = insp.el.clone();
    let ElementKind::Chart(c) = &el.kind else { return };
    let plot = c.plot(&insp.app.editor.doc);
    if c.chart_type == ChartType::Waterfall {
        waterfall_section(insp, ui, c, &plot);
        return;
    }
    let n = if c.chart_type.is_round() { plot.categories.len() } else { plot.series.len() };
    if n == 0 {
        ui.add_space(8.0);
        w::caption(ui, crate::t!("chart-series-none"));
        return;
    }
    let names: Vec<String> = if c.chart_type.is_round() { plot.categories.clone() } else { plot.series.iter().map(|s| s.name.clone()).collect() };
    let key = egui::Id::unique(("chart-series-pick", el.id.clone()));
    let mut k: usize = ui.data(|d| d.get_temp(key)).unwrap_or(0).min(n - 1);
    w::section(ui, "", |ui| {
        let which = if c.chart_type.is_round() { crate::t!("chart-slice") } else { crate::t!("chart-series") };
        w::row_w(ui, &which, 80.0, |ui| {
            w::popup_button(ui, "chart-series", &names[k], ui.available_width(), |ui| {
                for (i, name) in names.iter().enumerate() {
                    if w::menu_check(ui, i == k, name, None).clicked() {
                        k = i;
                    }
                }
            });
        });
    });
    ui.data_mut(|d| d.insert_temp(key, k));
    let f = c.series.get(k).cloned().unwrap_or_default();
    let patch = |p: SeriesPatch| ChartFormat { series: vec![SeriesPatch { index: k, ..p }], ..Default::default() };
    w::section(ui, &crate::t!("chart-series-colour"), |ui| {
        let slots: Vec<(&str, String)> = vec![
            ("theme:shape1", crate::t!("chart-colour-n", n = 1)),
            ("theme:shape2", crate::t!("chart-colour-n", n = 2)),
            ("theme:shape3", crate::t!("chart-colour-n", n = 3)),
            ("theme:shape4", crate::t!("chart-colour-n", n = 4)),
            ("theme:shape5", crate::t!("chart-colour-n", n = 5)),
            ("theme:shape6", crate::t!("chart-colour-n", n = 6)),
        ];
        let sw = crate::inspector::theme_swatches(insp, &slots);
        // No colour of its own: its slot in the theme shows as chosen.
        let cur = f.color.clone().unwrap_or_else(|| format!("theme:shape{}", k % 6 + 1));
        let def = insp.color(&format!("theme:shape{}", k % 6 + 1));
        if let Some(col) = crate::inspector::colour_picker(ui, &format!("chart-series-{k}"), &cur, def, &sw) {
            format(insp, patch(SeriesPatch { color: Some(col), ..Default::default() }));
        }
    });
    if c.chart_type.is_round() {
        return;
    }
    w::section(ui, "", |ui| {
        let mut labels = f.labels.unwrap_or(c.value_labels || c.look == ChartLook::Labelled);
        if w::checkbox(ui, &mut labels, &crate::t!("chart-series-labels")).changed() {
            format(insp, patch(SeriesPatch { labels: Some(labels), ..Default::default() }));
        }
        let lined = c.chart_type.has_lines() && c.as_line(k, plot.series.len()) || matches!(c.chart_type, ChartType::Scatter);
        if c.chart_type == ChartType::Combo {
            let mut as_line = c.as_line(k, plot.series.len());
            if w::checkbox(ui, &mut as_line, &crate::t!("chart-series-as-line")).changed() {
                format(insp, patch(SeriesPatch { as_line: Some(as_line), ..Default::default() }));
            }
        }
        if lined {
            w::row_w(ui, &crate::t!("chart-series-line-width"), 120.0, |ui| {
                let mut v = f.line_width.unwrap_or(2.25);
                let r = w::number_field(ui, &mut v, Num::pt().range(0.5, 100.0).step(0.5).decimals(1).width(64.0));
                insp.track(&r);
                if r.changed() {
                    format(insp, patch(SeriesPatch { line_width: Some(v.clamp(0.5, 100.0)), ..Default::default() }));
                }
            });
            if c.chart_type != ChartType::Step {
                let mut markers = f.markers.unwrap_or(matches!(c.chart_type, ChartType::Line | ChartType::Scatter | ChartType::Combo));
                if w::checkbox(ui, &mut markers, &crate::t!("chart-series-markers")).changed() {
                    format(insp, patch(SeriesPatch { markers: Some(markers), ..Default::default() }));
                }
                let mut smooth = f.smooth;
                if w::checkbox(ui, &mut smooth, &crate::t!("chart-series-smooth")).changed() {
                    format(insp, patch(SeriesPatch { smooth: Some(smooth), ..Default::default() }));
                }
            }
        }
    });
}

fn waterfall_section(insp: &mut Insp, ui: &mut Ui, c: &ChartData, plot: &chart::Plot) {
    w::section(ui, &crate::t!("chart-section-totals"), |ui| {
        w::caption(ui, crate::t!("chart-totals-caption"));
        // Every point, scrolling past a dozen.
        let mut changed = None;
        egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show_rows(ui, 24.0, plot.categories.len(), |ui, rows| {
            for i in rows {
                let mut on = c.totals.contains(&i) || i == 0;
                let r = ui.add_enabled_ui(i > 0, |ui| w::checkbox(ui, &mut on, &plot.categories[i])).inner;
                if r.changed() {
                    changed = Some((i, on));
                }
            }
        });
        if let Some((i, on)) = changed {
            let mut t: Vec<usize> = c.totals.iter().copied().filter(|x| *x != i).collect();
            if on {
                t.push(i);
            }
            format(insp, ChartFormat { totals: Some(t), ..Default::default() });
        }
    });
}

/// What a chart over the drawing limits leaves out, in words.
pub fn cut_note(p: &chart::Plot) -> String {
    let (a, b, c, d) = (p.categories.len() as u64, p.all_categories as u64, p.series.len() as u64, p.all_series as u64);
    match (a < b, c < d) {
        (true, true) => crate::t!("chart-cut-both", categories = a, all_categories = b, series = c, all_series = d),
        (false, true) => crate::t!("chart-cut-series", shown = c, all = d),
        _ => crate::t!("chart-cut-categories", shown = a, all = b),
    }
}

/// The kind name in the inspector header.
pub fn kind_name() -> String {
    crate::t!("inspector-kind-chart")
}

/// For the canvas when the GPU is not there: a box with the title.
pub fn fallback_label(c: &ChartData) -> String {
    if c.title.is_empty() { type_name(c.chart_type) } else { c.title.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_and_look_has_a_name() {
        for t in ChartType::ALL {
            assert!(crate::i18n::LOADER.has(&format!("chart-type-{}", t.key().replace('_', "-"))), "{t:?}");
        }
        for l in ChartLook::ALL {
            assert!(crate::i18n::LOADER.has(&format!("chart-look-{}", l.key())), "{l:?}");
        }
    }
}
