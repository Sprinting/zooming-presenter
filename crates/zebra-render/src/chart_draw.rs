//! Charts in the display list.
//!
//! A chart is laid out by BetterOffice's DrawingML chart geometry (axes,
//! ticks, bars, slices, legends) in a design space whose text is 10 px, and
//! drawn scaled to the element. Series are handed to the layout with stand-in
//! colours, so each mark can be told apart and given its look here: the
//! theme's colour (contrast enforced against what the chart sits on),
//! rounded or outlined bars, tints of one colour, a framed ground. Waterfall
//! is built on stacked columns with a clear base; Step rewrites the lines.
//!
//! The laid-out chart is cached per element (bounded by count and bytes) and
//! laid out again only when what it draws changed: its data (by row
//! identity, so typing in a linked table re-lays only its charts), options,
//! size or theme. Far away a chart draws without its text (under
//! [`TEXT_PIXELS`]) or as one block (under [`BLOCK_PIXELS`]).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use super::*;
use ooxml_drawingml::chart::{
    self as dml, PlotAxis, PlotAxisKind, PlotAxisRange, PlotChart, PlotChartText, PlotDataLabels, PlotFill, PlotGroup, PlotLegend, PlotLine, PlotMarker, PlotMarkerSymbol,
    PlotOp, PlotPoint, PlotRect, PlotSink, PlotTextAlign, PlotTextStyle,
};
use ooxml_drawingml::GeometryPathCommand;
use zebra_core::chart::{ChartData, ChartLook, ChartType, Plot};
use zebra_core::table::{CellType, DataFormat};
use zebra_core::table_style::{self, adjust_hue, contrast, mix, readable};
use zebra_core::{TextData, TextStyle};

/// A chart shorter than this on screen (device px) draws no text.
pub const TEXT_PIXELS: f64 = 48.0;
/// A chart shorter than this is one block.
pub const BLOCK_PIXELS: f64 = 4.0;
/// Label text in the design space (px), as the layout is made for.
const DESIGN_LABEL: f64 = 10.0;
use zebra_core::chart::LABEL_OF_BODY;
/// One design px in EMU (line widths).
const EMU_PX: f64 = 9525.0;

/// One thing a chart draws, in design space.
#[derive(Clone)]
pub(crate) enum Mark {
    Fill(BezPath, Rgba),
    Stroke(BezPath, Rgba, f64),
    /// A laid-out text and where it goes (layout space to design space).
    Text(Arc<TextLayout>, Affine, Rgba),
}

/// A laid-out chart: its marks in design space and the scale to the element.
pub(crate) struct Drawn {
    marks: Vec<Mark>,
    /// Design space to element space (top-left origin).
    k: f64,
    /// For far away: what the chart mostly is.
    block: Rgba,
    bytes: usize,
}

/// Laid-out charts, by element id.
#[derive(Default)]
pub(crate) struct ChartCache {
    charts: HashMap<String, (u64, Arc<Drawn>, u64)>,
    bytes: usize,
    tick: u64,
    /// Layouts done (for tests).
    pub(crate) layouts: u64,
}

impl ChartCache {
    const MAX_CHARTS: usize = 256;
    const MAX_BYTES: usize = 32 << 20;

    fn get(&mut self, id: &str, sig: u64) -> Option<Arc<Drawn>> {
        self.tick += 1;
        let tick = self.tick;
        match self.charts.get_mut(id) {
            Some((s, d, t)) if *s == sig => {
                *t = tick;
                Some(d.clone())
            }
            _ => None,
        }
    }

    fn put(&mut self, id: &str, sig: u64, d: Arc<Drawn>) {
        self.layouts += 1;
        if let Some((_, old, _)) = self.charts.remove(id) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        self.bytes += d.bytes;
        self.charts.insert(id.to_string(), (sig, d, self.tick));
        // Oldest out, past either bound.
        while self.charts.len() > Self::MAX_CHARTS || (self.bytes > Self::MAX_BYTES && self.charts.len() > 1) {
            let Some(oldest) = self.charts.iter().filter(|(k, _)| k.as_str() != id).min_by_key(|(_, v)| v.2).map(|(k, _)| k.clone()) else { break };
            if let Some((_, d, _)) = self.charts.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(d.bytes);
            }
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.charts.len()
    }
}

/// What a chart's drawing depends on.
fn signature(e: &Element, c: &ChartData, doc: &zebra_core::Document, th: &Theme) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    e.size.w.to_bits().hash(&mut h);
    e.size.h.to_bits().hash(&mut h);
    // Everything but the cells, then the cells by row identity.
    let mut head = c.clone();
    head.data.cells.clear();
    serde_json::to_string(&head).unwrap_or_default().hash(&mut h);
    let rows = |t: &zebra_core::table::TableData, h: &mut std::collections::hash_map::DefaultHasher| {
        t.n_cols().hash(h);
        (t.header_rows, t.header_columns, t.footer_rows).hash(h);
        for row in &t.cells {
            row.identity().hash(h);
        }
    };
    rows(&c.data, &mut h);
    if let Some(l) = &c.link {
        if let Some(ElementKind::Table(t)) = doc.elements.get(&l.table).map(|e| &e.kind) {
            rows(t, &mut h);
        }
    }
    serde_json::to_string(th).unwrap_or_default().hash(&mut h);
    h.finish()
}

fn parse(c: &str) -> Option<Rgba> {
    let h = c.strip_prefix('#').unwrap_or(c);
    let p = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok());
    match h.len() {
        6 => Some([p(0)?, p(2)?, p(4)?, 255]),
        8 => Some([p(0)?, p(2)?, p(4)?, p(6)?]),
        _ => None,
    }
}

/// Stand-in colours the layout is given, told apart again in the sink.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stand {
    Series(usize),
    Point(usize),
    Clear,
}

fn stand_series(i: usize) -> String {
    format!("#01{:02X}00", i.min(255))
}
fn stand_point(i: usize) -> String {
    format!("#02{:04X}", i.min(0xffff))
}
const STAND_CLEAR: &str = "#030000";

fn stand(c: &str) -> Option<Stand> {
    let h = c.strip_prefix('#').unwrap_or(c);
    if h.len() < 6 {
        return None;
    }
    let n = |r: std::ops::Range<usize>| usize::from_str_radix(&h[r], 16).ok();
    match &h[..2] {
        "01" if &h[4..6] == "00" => n(2..4).map(Stand::Series),
        "02" => n(2..6).map(Stand::Point),
        "03" if &h[2..6] == "0000" => Some(Stand::Clear),
        _ => None,
    }
}

/// The colours a chart is drawn in.
struct Paint {
    under: Rgba,
    ground: Option<Rgba>,
    text: Rgba,
    axis: Rgba,
    grid: Rgba,
    series: Vec<Rgba>,
    points: Vec<Rgba>,
}

/// An Excel-style number code for a value axis or label, or None for the
/// layout's own (Automatic, with no big numbers).
fn number_code(f: &DataFormat, big: bool) -> Option<String> {
    let dec = |d: Option<u8>, default: u8| {
        let d = d.unwrap_or(default).min(10) as usize;
        if d == 0 { String::new() } else { format!(".{}", "0".repeat(d)) }
    };
    let whole = if f.thousands { "#,##0" } else { "0" };
    match f.kind {
        CellType::Automatic => big.then(|| "#,##0".to_string()),
        CellType::Number => Some(format!("{whole}{}", dec(f.decimals, 0))),
        CellType::Currency => Some(format!("\"{}\"{whole}{}", f.symbol().replace('"', ""), dec(f.decimals, 0))),
        CellType::Percentage => Some(format!("0{}%", dec(f.decimals, 0))),
        _ => None,
    }
}

impl Ctx<'_> {
    pub(super) fn chart(&self, e: &Element, c: &ChartData, a: &Affine, items: &mut Vec<Item>) {
        let (w, h) = (e.size.w, e.size.h);
        let s = affine_scale(a);
        if !(s.is_finite() && s > 0.0 && w > 0.0 && h > 0.0) {
            return;
        }
        let tl = *a * Affine::translate((-w / 2.0, -h / 2.0));
        let doc = &self.editor.doc;
        let sig = signature(e, c, doc, self.theme);
        let px = w.min(h) * s;
        let cached = self.r.charts.lock().ok().and_then(|mut cache| cache.get(&e.id, sig));
        let drawn = match cached {
            Some(d) => d,
            None => {
                let d = Arc::new(self.lay_out(c, w, h));
                if let Ok(mut cache) = self.r.charts.lock() {
                    cache.put(&e.id, sig, d.clone());
                }
                d
            }
        };
        if px < BLOCK_PIXELS && !self.full_res_images {
            fill(items, &Rect::new(0.0, 0.0, w, h), &tl, drawn.block);
            return;
        }
        let texts = px >= TEXT_PIXELS || self.full_res_images;
        let to = tl * Affine::scale(drawn.k);
        for m in &drawn.marks {
            match m {
                Mark::Fill(p, color) => fill(items, p, &to, *color),
                Mark::Stroke(p, color, width) => stroke(items, p, &to, *color, Stroke::new(*width).with_join(Join::Round).with_caps(Cap::Butt)),
                Mark::Text(layout, at, color) if texts => {
                    items.push(Item::Text { layout: layout.clone(), transform: to * *at, color: *color, run_colors: Arc::from([]) });
                }
                Mark::Text(..) => {}
            }
        }
    }

    /// The chart laid out for a `w` x `h` element.
    pub(crate) fn lay_out(&self, c: &ChartData, w: f64, h: f64) -> Drawn {
        let th = self.theme;
        let doc = &self.editor.doc;
        let body = th.text_style(TextStyle::Body).size.max(1.0);
        let k = (body * LABEL_OF_BODY / DESIGN_LABEL).max(1e-6);
        let rect = PlotRect { x: 0.0, y: 0.0, w: (w / k).clamp(1.0, 1e6), h: (h / k).clamp(1.0, 1e6) };
        let plot = c.plot(doc);
        let paint = self.paint(c, &plot);
        let mut sink = Sink { ctx: self, c, paint: &paint, marks: vec![], bytes: 0, body, placed: vec![] };
        let big = plot.series.iter().flat_map(|s| s.values.iter()).any(|v| v.abs() >= 1000.0);
        let pool = Pool::new(&plot, number_code(&c.number, big));
        if c.chart_type == ChartType::Waterfall {
            let wf = Waterfall::of(c, &plot);
            sink.plot(&wf.chart(c, &plot, &pool), rect);
        } else {
            let input = Input::of(c, &plot);
            sink.plot(&input.chart(c, &plot, &pool), rect);
        }
        let block = paint.ground.unwrap_or_else(|| mix(paint.series.first().copied().unwrap_or(paint.axis), paint.under, 0.35));
        let bytes = sink.bytes + 256;
        Drawn { marks: sink.marks, k, block, bytes }
    }

    fn paint(&self, c: &ChartData, plot: &Plot) -> Paint {
        let th = self.theme;
        let canvas = th.color(&"theme:background".to_string());
        let ground = (c.look == ChartLook::Framed).then(|| th.color(&th.frame_fill));
        let under = ground.unwrap_or(canvas);
        // The default chart reads at the stricter ratios.
        let default = c.chart_type == ChartType::Column && c.look == ChartLook::Flat;
        let (text_ratio, mark_ratio) = if default { (table_style::DEFAULT_TEXT, table_style::MIN_TEXT) } else { (table_style::MIN_TEXT, table_style::MIN_MARK) };
        let text = readable(th.color(&"theme:body".to_string()), under, text_ratio, th);
        let axis = adjust_hue(mix(text, under, 0.55), under, table_style::MIN_MARK);
        let grid = mix(text, under, 0.16);
        let slot = |i: usize| th.color(&format!("theme:shape{}", i % 6 + 1));
        let mark = |want: Rgba| adjust_hue(want, under, mark_ratio);
        let n = plot.series.len().max(1);
        let series: Vec<Rgba> = (0..n)
            .map(|i| {
                let own = c.series.get(i).and_then(|f| f.color.as_ref()).map(|s| th.color(s));
                let base = match (own, c.look) {
                    (Some(o), _) => o,
                    // One colour: lighter tints first, the full colour last.
                    (None, ChartLook::Tints) if n > 1 => mix(slot(0), under, 0.45 + 0.55 * i as f64 / (n - 1) as f64),
                    (None, _) => slot(i),
                };
                let base = if c.look == ChartLook::Tints && own.is_none() { base } else { mark(base) };
                // Past six the colours come round again, lighter.
                if i >= 6 && own.is_none() {
                    let light = mix(base, under, 0.6);
                    if contrast(light, under) >= table_style::MIN_MARK { light } else { base }
                } else {
                    base
                }
            })
            .collect();
        let points = match c.chart_type {
            ChartType::Pie | ChartType::Donut => {
                let n = plot.categories.len();
                (0..n)
                    .map(|i| {
                        let own = c.series.get(i).and_then(|f| f.color.as_ref()).map(|s| th.color(s));
                        let base = own.unwrap_or_else(|| if c.look == ChartLook::Tints { mix(slot(0), under, 0.35 + 0.65 * (n - 1 - i.min(n - 1)) as f64 / (n.max(2) - 1) as f64) } else { slot(i) });
                        mark(base)
                    })
                    .collect()
            }
            ChartType::Waterfall => {
                // Totals, rises, falls.
                let pick = |i: usize, s: Option<&String>| mark(s.map(|s| th.color(s)).unwrap_or_else(|| slot(i)));
                vec![pick(0, c.series.first().and_then(|f| f.color.as_ref())), pick(2, c.series.get(1).and_then(|f| f.color.as_ref())), pick(5, c.series.get(2).and_then(|f| f.color.as_ref()))]
            }
            _ => vec![],
        };
        Paint { under, ground, text, axis, grid, series, points }
    }
}

/// The layout's input for a chart (not Waterfall).
struct Input<'a> {
    categories: &'a [String],
    groups: Vec<(&'static str, Option<&'static str>, Vec<usize>)>,
}

impl<'a> Input<'a> {
    fn of(c: &ChartData, plot: &'a Plot) -> Input<'a> {
        let n = plot.series.len();
        let all: Vec<usize> = (0..n).collect();
        let groups = match c.chart_type {
            ChartType::Column => vec![("column", Some("clustered"), all)],
            ChartType::StackedColumn => vec![("column", Some("stacked"), all)],
            ChartType::PercentColumn => vec![("column", Some("percentStacked"), all)],
            ChartType::Bar => vec![("bar", Some("clustered"), all)],
            ChartType::StackedBar => vec![("bar", Some("stacked"), all)],
            ChartType::Line | ChartType::Step => vec![("line", Some("standard"), all)],
            ChartType::Area => vec![("area", Some("standard"), all)],
            ChartType::StackedArea => vec![("area", Some("stacked"), all)],
            ChartType::Pie => vec![("pie", None, all.into_iter().take(1).collect())],
            ChartType::Donut => vec![("doughnut", None, all.into_iter().take(1).collect())],
            ChartType::Scatter => vec![("scatter", None, all)],
            ChartType::Bubble => vec![("bubble", None, all)],
            ChartType::Radar => vec![("radar", None, all)],
            ChartType::Combo => {
                let (lines, bars): (Vec<usize>, Vec<usize>) = all.into_iter().partition(|i| c.as_line(*i, n));
                let mut g = vec![];
                if !bars.is_empty() {
                    g.push(("column", Some("clustered"), bars));
                }
                if !lines.is_empty() {
                    g.push(("line", Some("standard"), lines));
                }
                g
            }
            ChartType::Waterfall => unreachable!("waterfalls are laid out by Waterfall"),
        };
        Input { categories: &plot.categories, groups }
    }

    fn chart(&'a self, c: &'a ChartData, plot: &'a Plot, pool: &'a Pool) -> Owned<'a> {
        let labels_all = c.value_labels || c.look == ChartLook::Labelled;
        let mut owned = Owned { groups: vec![], code: pool.code.as_deref(), legend: c.legend, step: !c.step_ignored(plot) };
        for (ty, grouping, members) in &self.groups {
            let mut series = vec![];
            for &i in members {
                let s = &plot.series[i];
                let fmt = c.series.get(i);
                let lined = matches!(*ty, "line" | "radar" | "scatter");
                let width = fmt.and_then(|f| f.line_width).unwrap_or(if c.chart_type == ChartType::Radar { 1.6 } else { 2.25 });
                let markers = fmt.and_then(|f| f.markers).unwrap_or(matches!(c.chart_type, ChartType::Line | ChartType::Scatter | ChartType::Combo));
                let labels = fmt.and_then(|f| f.labels).unwrap_or(labels_all);
                series.push(dml::PlotSeries {
                    name: Some(s.name.as_str()),
                    categories: self.categories,
                    values: &s.values,
                    x_values: &s.x,
                    bubble_sizes: &s.sizes,
                    color: Some(pool.series[i].as_str()),
                    points: if matches!(*ty, "pie" | "doughnut") {
                        (0..self.categories.len()).map(|j| PlotPoint { index: Some(j), color: Some(pool.points[j].as_str()), ..Default::default() }).collect()
                    } else {
                        vec![]
                    },
                    smooth: fmt.is_some_and(|f| f.smooth),
                    marker: Some(PlotMarker {
                        size: Some(if c.chart_type == ChartType::Scatter { 7.0 } else { 5.0 }),
                        symbol: Some(if markers && c.chart_type != ChartType::Step { PlotMarkerSymbol::Circle } else { PlotMarkerSymbol::None }),
                    }),
                    labels: labels.then(|| pool.labels()),
                    line: Some(if c.chart_type == ChartType::Scatter && fmt.and_then(|f| f.line_width).is_none() {
                        PlotLine { none: true, ..Default::default() }
                    } else if lined {
                        PlotLine { none: false, color: None, width_emu: Some(width * EMU_PX) }
                    } else {
                        PlotLine::default()
                    }),
                    ..Default::default()
                });
            }
            owned.groups.push(PlotGroup {
                chart_type: Some(ty),
                grouping: *grouping,
                series,
                overlap: Some(if matches!(*grouping, Some("stacked" | "percentStacked")) { 100.0 } else { -(c.gap * 100.0) }),
                gap_width: Some(c.gap_sets * 100.0),
                hole_size: Some(c.hole * 100.0),
                vary_colors: matches!(*ty, "pie" | "doughnut"),
                scatter_style: (*ty == "scatter").then_some("lineMarker"),
                radar_style: (*ty == "radar").then_some("filled"),
                markers: None,
                axis_ids: if matches!(*ty, "scatter" | "bubble") { vec!["x", "v"] } else { vec!["c", "v"] },
                ..Default::default()
            });
        }
        owned
    }
}

/// A waterfall as stacked columns: a clear base, then the visible part
/// above zero and below it, each point coloured as a total, a rise or a fall.
struct Waterfall {
    base: Vec<f64>,
    up: Vec<f64>,
    down: Vec<f64>,
    /// 0 total, 1 rise, 2 fall, by point.
    kind: Vec<usize>,
}

impl Waterfall {
    fn of(c: &ChartData, plot: &Plot) -> Waterfall {
        let values: &[f64] = plot.series.first().map(|s| s.values.as_slice()).unwrap_or(&[]);
        let mut wf = Waterfall { base: vec![], up: vec![], down: vec![], kind: vec![] };
        let mut run = 0.0f64;
        for (i, v) in values.iter().enumerate() {
            let total = c.totals.contains(&i) || i == 0;
            let v = if v.is_finite() { *v } else { 0.0 };
            let (from, to, kind) = if total {
                run = v;
                (0.0, v, 0)
            } else {
                let from = run;
                run += v;
                (from, run, if v >= 0.0 { 1 } else { 2 })
            };
            let (lo, hi) = (from.min(to), from.max(to));
            // Stacks: positive parts upwards from zero, negative downwards.
            let (base, up, down) = if lo >= 0.0 {
                (lo, hi - lo, 0.0)
            } else if hi <= 0.0 {
                (hi, 0.0, lo - hi)
            } else {
                (0.0, hi, lo)
            };
            wf.base.push(base);
            wf.up.push(up);
            wf.down.push(down);
            wf.kind.push(kind);
        }
        wf
    }

    fn chart<'a>(&'a self, c: &'a ChartData, plot: &'a Plot, pool: &'a Pool) -> Owned<'a> {
        let mut owned = Owned { groups: vec![], code: pool.code.as_deref(), legend: false, step: !c.step_ignored(plot) };
        let labels = c.value_labels || c.look == ChartLook::Labelled;
        let mut series = vec![];
        for (n, values) in [&self.base, &self.up, &self.down].into_iter().enumerate() {
            let points = if n == 0 {
                vec![]
            } else {
                self.kind.iter().enumerate().map(|(j, k)| PlotPoint { index: Some(j), color: Some(pool.points[*k].as_str()), ..Default::default() }).collect()
            };
            series.push(dml::PlotSeries {
                name: Some(""),
                categories: &plot.categories,
                values,
                color: Some(if n == 0 { pool.clear.as_str() } else { pool.points[0].as_str() }),
                points,
                labels: (labels && n > 0).then(|| pool.labels()),
                ..Default::default()
            });
        }
        owned.groups.push(PlotGroup {
            chart_type: Some("column"),
            grouping: Some("stacked"),
            series,
            overlap: Some(100.0),
            gap_width: Some(c.gap_sets * 100.0),
            axis_ids: vec!["c", "v"],
            ..Default::default()
        });
        owned
    }
}

/// The strings the layout's input borrows: stand-in colours and the
/// number code.
struct Pool {
    series: Vec<String>,
    points: Vec<String>,
    clear: String,
    code: Option<String>,
}

impl Pool {
    fn new(plot: &Plot, code: Option<String>) -> Pool {
        Pool {
            series: (0..plot.series.len()).map(stand_series).collect(),
            points: (0..plot.categories.len().max(3)).map(stand_point).collect(),
            clear: STAND_CLEAR.to_string(),
            code,
        }
    }

    fn labels(&self) -> PlotDataLabels<'_> {
        PlotDataLabels { show_value: true, number_format: self.code.as_deref(), position: Some("outEnd"), ..Default::default() }
    }
}

/// The layout's input.
struct Owned<'a> {
    groups: Vec<PlotGroup<'a>>,
    code: Option<&'a str>,
    legend: bool,
    /// The axis step set is drawn (not too fine for the data).
    step: bool,
}

/// Receives the layout's ops and makes marks of them.
struct Sink<'s, 'c> {
    ctx: &'s Ctx<'c>,
    c: &'s ChartData,
    paint: &'s Paint,
    marks: Vec<Mark>,
    bytes: usize,
    body: f64,
    /// Level text placed so far: its box, baseline and centre.
    placed: Vec<(Rect, f64, f64)>,
}

impl Sink<'_, '_> {
    fn plot(&mut self, input: &Owned<'_>, rect: PlotRect) {
        let c = self.c;
        let fs = c.font_size;
        let label = PlotTextStyle { size_pt: Some(DESIGN_LABEL * fs * 0.75), ..Default::default() };
        let code = input.code;
        let scatter = matches!(c.chart_type, ChartType::Scatter | ChartType::Bubble);
        let grid = c.gridlines && c.look != ChartLook::Labelled;
        let x_title = (!c.x_title.trim().is_empty()).then_some(c.x_title.as_str());
        let y_title = (!c.y_title.trim().is_empty()).then_some(c.y_title.as_str());
        let mut axes = vec![
            PlotAxis {
                id: Some(if scatter { "x" } else { "c" }),
                kind: if scatter { PlotAxisKind::Value } else { PlotAxisKind::Category },
                major_tick_mark: Some("none"),
                // Bars read top to bottom, as the data does.
                reversed: matches!(c.chart_type, ChartType::Bar | ChartType::StackedBar),
                title: x_title,
                position: Some(if matches!(c.chart_type, ChartType::Bar | ChartType::StackedBar) { "l" } else { "b" }),
                text: label,
                ..Default::default()
            },
            PlotAxis {
                id: Some("v"),
                kind: PlotAxisKind::Value,
                range: PlotAxisRange { min: c.axis_min, max: c.axis_max },
                // A step that would draw too many gridlines is automatic (the
                // Axis tab says so).
                major_unit: c.axis_step.filter(|_| input.step),
                major_gridlines: grid,
                major_tick_mark: Some("none"),
                number_format: code,
                // Labelled: the values are on the marks.
                hidden: c.look == ChartLook::Labelled,
                title: y_title,
                position: Some(if matches!(c.chart_type, ChartType::Bar | ChartType::StackedBar) { "b" } else { "l" }),
                text: label,
                line: Some(PlotLine { none: true, ..Default::default() }),
                ..Default::default()
            },
        ];
        if c.chart_type == ChartType::PercentColumn {
            axes[1].number_format = Some("0%");
        }
        // Columns and lines share one value axis: one scale for both.
        if c.chart_type == ChartType::Combo && c.axis_min.is_none() && c.axis_max.is_none() {
            let (lo, hi) = self.combo_span();
            if let Some((min, max, step)) = nice_span(lo, hi) {
                axes[1].range = PlotAxisRange { min: Some(min), max: Some(max) };
                axes[1].major_unit = axes[1].major_unit.or(Some(step));
            }
        }
        // A percent axis runs to 100%, as PowerPoint's does.
        if c.chart_type == ChartType::PercentColumn && c.axis_max.is_none() {
            axes[1].range.max = Some(1.0);
        }
        let title = (!c.title.trim().is_empty()).then_some(c.title.as_str());
        // No step set: one the axis has room to label (PowerPoint's
        // automatic step also widens as the chart gets smaller).
        if axes[1].major_unit.is_none() && !axes[1].hidden {
            axes[1].major_unit = self.roomy_step(rect, DESIGN_LABEL * fs, title.is_some(), input.legend);
        }
        let chart = PlotChart {
            chart_type: input.groups.first().and_then(|g| g.chart_type).unwrap_or("column"),
            title,
            legend: Some(PlotLegend { position: Some("bottom"), visible: Some(input.legend), overlay: false }),
            value_axis: None,
            axis_titles: Default::default(),
            series: vec![],
            plot_groups: input.groups.clone(),
            text: PlotChartText {
                chart: label,
                title: PlotTextStyle { size_pt: Some(14.0 * fs * 0.75), bold: Some(true), ..Default::default() },
                legend: label,
            },
            axes,
            fill: Some(match self.paint.ground {
                Some(_) => PlotFill::Solid(STAND_GROUND),
                None => PlotFill::None,
            }),
            plot_layout: None,
        };
        dml::plot_chart_into(&chart, rect, self);
        if c.border {
            let p = Rect::new(rect.x, rect.y, rect.x + rect.w, rect.y + rect.h).inset(-0.5).to_path(0.1);
            self.marks.push(Mark::Stroke(p, self.paint.axis, 1.0));
        }
    }

    /// A value-axis step coarse enough that its labels don't run into one
    /// another in `rect` (labels `px` high), or None when the layout's own
    /// step already fits.
    fn roomy_step(&self, rect: PlotRect, px: f64, title: bool, legend: bool) -> Option<f64> {
        let c = self.c;
        let plot = c.plot(&self.ctx.editor.doc);
        let n = plot.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        let (mut lo, mut hi) = (0.0f64, 0.0f64);
        match c.chart_type {
            ChartType::Pie | ChartType::Donut | ChartType::Radar | ChartType::Waterfall => return None,
            ChartType::PercentColumn => hi = 1.0,
            ChartType::StackedColumn | ChartType::StackedBar | ChartType::StackedArea => {
                for i in 0..n {
                    let (mut up, mut down) = (0.0, 0.0);
                    for v in plot.series.iter().filter_map(|s| s.values.get(i)).filter(|v| v.is_finite()) {
                        if *v > 0.0 {
                            up += v;
                        } else {
                            down += v;
                        }
                    }
                    (lo, hi) = (lo.min(down), hi.max(up));
                }
            }
            _ => {
                for v in plot.series.iter().flat_map(|s| s.values.iter()).filter(|v| v.is_finite()) {
                    (lo, hi) = (lo.min(*v), hi.max(*v));
                }
            }
        }
        lo = c.axis_min.filter(|v| v.is_finite()).unwrap_or(lo);
        hi = c.axis_max.filter(|v| v.is_finite()).unwrap_or(hi);
        let span = (hi - lo) * 1.1;
        if !(span.is_finite() && span > 0.0 && px > 0.0) {
            return None;
        }
        let wide = |t: &str| self.text_layout(t, false).width * px / self.body;
        let (room, pitch) = if matches!(c.chart_type, ChartType::Bar | ChartType::StackedBar) {
            let names = plot.categories.iter().take(200).map(|t| wide(t)).fold(0.0, f64::max).min(rect.w * 0.4);
            let widest = [lo, hi].iter().map(|v| wide(&grouped(*v))).fold(0.0, f64::max);
            (rect.w - names - px * 2.0, widest + px)
        } else {
            let lines = 2.5 + if title { 2.0 } else { 0.0 } + if legend { 1.5 } else { 0.0 };
            (rect.h - lines * px * 1.4, px * 1.6)
        };
        let fit = (room / pitch).floor().max(1.0);
        let step = nice_step(span / fit)?;
        (step > excel_unit(span)).then_some(step)
    }

    /// The lowest and highest value a combo chart draws.
    fn combo_span(&self) -> (f64, f64) {
        let plot = self.c.plot(&self.ctx.editor.doc);
        plot.series.iter().flat_map(|s| s.values.iter()).filter(|v| v.is_finite()).fold((0.0f64, 0.0f64), |(a, b), v| (a.min(*v), b.max(*v)))
    }

    fn color(&self, s: &str) -> Option<Rgba> {
        if s.eq_ignore_ascii_case(STAND_GROUND) {
            return self.paint.ground;
        }
        match stand(s) {
            Some(Stand::Series(i)) => self.paint.series.get(i).copied().or(Some(self.paint.axis)),
            Some(Stand::Point(i)) => self.paint.points.get(i).copied().or(Some(self.paint.axis)),
            Some(Stand::Clear) => None,
            None => {
                let up = s.to_ascii_uppercase();
                let up = up.trim_start_matches('#');
                Some(match up {
                    "D9D9D9" => self.paint.grid,
                    "666666" | "595959" | "868686" | "BFBFBF" | "A6A6A6" => self.paint.axis,
                    _ => parse(s).map(|c| readable_mark(c, self.paint.under)).unwrap_or(self.paint.axis),
                })
            }
        }
    }

    /// Whether `s` is a series' stand-in (bars, slices, markers, lines).
    fn is_data(s: &str) -> bool {
        matches!(stand(s), Some(Stand::Series(_) | Stand::Point(_)))
    }

    fn push(&mut self, m: Mark) {
        self.bytes += match &m {
            Mark::Fill(p, _) | Mark::Stroke(p, _, _) => 32 + p.elements().len() * 56,
            Mark::Text(..) => 96,
        };
        self.marks.push(m);
    }

    fn text_layout(&self, text: &str, bold: bool) -> Arc<TextLayout> {
        let td = TextData { text: zebra_core::chart::label(text), bold, style: TextStyle::Body, ..Default::default() };
        self.ctx.r.fonts.layout(&td, self.ctx.theme, None)
    }

    fn bar(&mut self, r: Rect, color: Rgba) {
        let look = self.c.look;
        match look {
            ChartLook::Rounded => {
                let rad = (r.width().min(r.height()) * 0.3).min(5.0);
                self.push(Mark::Fill(RoundedRect::from_rect(r, rad).to_path(0.05), color));
            }
            ChartLook::Outline => {
                let w = 1.4f64.min(r.width() / 3.0).min(r.height() / 3.0).max(0.2);
                self.push(Mark::Stroke(r.inset(-w / 2.0).to_path(0.05), color, w));
            }
            _ => self.push(Mark::Fill(r.to_path(0.05), color)),
        }
    }
}

/// Round bounds and a step for values from `lo` to `hi` (about five steps
/// of 1, 2, 2.5 or 5 times a power of ten).
fn nice_span(lo: f64, hi: f64) -> Option<(f64, f64, f64)> {
    let span = hi - lo;
    if !(span.is_finite() && span > 0.0) {
        return None;
    }
    let raw = span / 5.0;
    let p = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * p).find(|s| *s >= raw)?;
    let min = (lo / step).floor() * step;
    let max = (hi / step).ceil() * step;
    (min.is_finite() && max.is_finite()).then_some((min, max, step))
}

/// The smallest step of 1, 2 or 5 times a power of ten that is at least
/// `raw`.
fn nice_step(raw: f64) -> Option<f64> {
    if !(raw.is_finite() && raw > 0.0) {
        return None;
    }
    let p = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * p).find(|s| *s >= raw * (1.0 - 1e-9))
}

/// The step the layout picks for a value axis spanning `range` (as Excel
/// does: 10 to 20 steps of a power of ten, else 2 or 5 times it).
fn excel_unit(range: f64) -> f64 {
    if !(range.is_finite() && range > 0.0) {
        return 1.0;
    }
    let major = 10f64.powf(range.log10().round() - 1.0);
    let steps = range / major;
    if steps >= 20.0 {
        major * 5.0
    } else if steps >= 10.0 {
        major * 2.0
    } else {
        major
    }
}

/// `v` rounded, with thousands grouped (to measure an axis label).
fn grouped(v: f64) -> String {
    let digits = format!("{:.0}", v.abs());
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if v < 0.0 {
        out.insert(0, '-');
    }
    out
}

/// A stand-in for the framed ground.
const STAND_GROUND: &str = "#040000";

/// A colour the layout chose itself (data label inside a slice...), made
/// to read on the chart.
fn readable_mark(c: Rgba, under: Rgba) -> Rgba {
    adjust_hue(c, under, table_style::MIN_MARK)
}

/// A layout path (its commands are in chart space already).
fn path_of(commands: &[GeometryPathCommand]) -> BezPath {
    let (x, y) = (0.0, 0.0);
    let mut p = BezPath::new();
    for c in commands {
        match *c {
            GeometryPathCommand::Move { x: a, y: b } => p.move_to((x + a, y + b)),
            GeometryPathCommand::Line { x: a, y: b } => p.line_to((x + a, y + b)),
            GeometryPathCommand::Quad { cpx, cpy, x: a, y: b } => p.quad_to((x + cpx, y + cpy), (x + a, y + b)),
            GeometryPathCommand::Cubic { cp1x, cp1y, cp2x, cp2y, x: a, y: b } => p.curve_to((x + cp1x, y + cp1y), (x + cp2x, y + cp2y), (x + a, y + b)),
            GeometryPathCommand::Close => p.close_path(),
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }
    p
}

/// A polyline as steps: level to halfway, then up or down.
fn stepped(p: &BezPath) -> BezPath {
    use kurbo::PathEl;
    let mut out = BezPath::new();
    let mut last: Option<Point> = None;
    for el in p.elements() {
        match *el {
            PathEl::MoveTo(q) => {
                out.move_to(q);
                last = Some(q);
            }
            PathEl::LineTo(q) | PathEl::QuadTo(_, q) | PathEl::CurveTo(_, _, q) => {
                if let Some(a) = last {
                    let mx = (a.x + q.x) / 2.0;
                    out.line_to((mx, a.y));
                    out.line_to((mx, q.y));
                }
                out.line_to(q);
                last = Some(q);
            }
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

impl PlotSink for Sink<'_, '_> {
    fn measure_text(&mut self, text: &str, font: &dml::PlotFont) -> Option<f64> {
        let l = self.text_layout(text, font.weight >= 600);
        Some(l.width * font.size_px / self.body)
    }

    fn turns_text(&self) -> bool {
        true
    }

    fn push_op(&mut self, op: PlotOp) -> bool {
        if self.marks.len() > dml::MAX_PLOT_OPS {
            return false;
        }
        let ty = self.c.chart_type;
        match op {
            PlotOp::Rect { x, y, w, h, fill } => {
                if !(w > 0.0 && h > 0.0) {
                    return true;
                }
                let Some(color) = self.color(&fill) else { return true };
                let r = Rect::new(x, y, x + w, y + h);
                if Self::is_data(&fill) {
                    self.bar(r, color);
                } else {
                    self.push(Mark::Fill(r.to_path(0.05), color));
                }
            }
            PlotOp::Line { x1, y1, x2, y2, color, width } => {
                let Some(col) = self.color(&color) else { return true };
                let mut p = BezPath::new();
                p.move_to((x1, y1));
                p.line_to((x2, y2));
                if ty == ChartType::Step && Self::is_data(&color) {
                    p = stepped(&p);
                }
                self.push(Mark::Stroke(p, col, width.max(0.1)));
            }
            PlotOp::Path { commands, fill, stroke, .. } => {
                let mut p = path_of(&commands);
                let data = Self::is_data(&fill) || stroke.as_ref().is_some_and(|s| Self::is_data(&s.color));
                if ty == ChartType::Step && data && (fill.is_empty() || fill.eq_ignore_ascii_case("none")) {
                    p = stepped(&p);
                }
                if let Some(full) = (!fill.is_empty() && !fill.eq_ignore_ascii_case("none")).then(|| self.color(&fill)).flatten() {
                    let mut color = full;
                    // Areas and bubbles see through to what's behind.
                    let see_through = match ty {
                        ChartType::Area => 0.7,
                        ChartType::Bubble => 0.6,
                        ChartType::Radar => 0.25,
                        _ => 1.0,
                    };
                    if Self::is_data(&fill) && see_through < 1.0 {
                        color = with_alpha(color, see_through);
                    }
                    if Self::is_data(&fill) && self.c.look == ChartLook::Outline && !matches!(ty, ChartType::Line | ChartType::Step | ChartType::Scatter) {
                        self.push(Mark::Stroke(p.clone(), color, 1.4));
                    } else {
                        self.push(Mark::Fill(p.clone(), color));
                        // Slices part with a line in what the chart is on.
                        if matches!(stand(&fill), Some(Stand::Point(_))) && matches!(ty, ChartType::Pie | ChartType::Donut) {
                            self.push(Mark::Stroke(p.clone(), self.paint.ground.unwrap_or(self.paint.under), 1.2));
                        }
                    }
                    // A radar's fill gets its outline.
                    if ty == ChartType::Radar && Self::is_data(&fill) && stroke.is_none() {
                        self.push(Mark::Stroke(p.clone(), full, 1.6));
                    }
                }
                if let Some(s) = stroke {
                    if let Some(col) = self.color(&s.color) {
                        self.push(Mark::Stroke(p, col, s.width.max(0.1)));
                    }
                }
            }
            PlotOp::Text { text, x, baseline_y, width, font, color, align, rotation_deg } => {
                if text.trim().is_empty() || !(font.size_px > 0.0) {
                    return true;
                }
                let layout = self.text_layout(&text, font.weight >= 600);
                let s = font.size_px / self.body;
                let lw = layout.width * s;
                let base = layout.runs.first().and_then(|r| r.glyphs.first()).map_or(layout.height * 0.8, |g| g.y as f64) * s;
                let left = match align {
                    PlotTextAlign::Center => x + (width - lw) / 2.0,
                    _ => x,
                };
                let top = baseline_y - base;
                let mut at = Affine::translate((left, top)) * Affine::scale(s);
                if rotation_deg.is_finite() && rotation_deg != 0.0 {
                    let centre = Point::new(left + lw / 2.0, top + layout.height * s / 2.0);
                    at = Affine::rotate_about(rotation_deg.to_radians(), centre) * at;
                }
                let want = if color.eq_ignore_ascii_case(dml::CHART_TEXT_COLOR) { self.paint.text } else { self.color(&color).unwrap_or(self.paint.text) };
                let col = readable(want, self.paint.under, table_style::MIN_TEXT, self.ctx.theme);
                // Two plot groups on one axis label it twice.
                if ty == ChartType::Combo && self.marks.iter().any(|m| matches!(m, Mark::Text(l, a, _) if l.text == layout.text && (a.translation() - at.translation()).hypot() < 0.5)) {
                    return true;
                }
                // Labels in one row or column that would run into one
                // another: the later one is left out (PowerPoint skips
                // crowded category labels the same way).
                if !(rotation_deg.is_finite() && rotation_deg != 0.0) {
                    let h = layout.height * s;
                    let r = Rect::new(left, top, left + lw, top + h);
                    let pad = h * 0.25;
                    let near = |a: f64, b: f64| (a - b).abs() < 0.5;
                    let crowded = self.placed.iter().any(|(p, b, c)| {
                        let lined_up = near(*b, baseline_y) || near(p.x0, r.x0) || near(p.x1, r.x1) || near(*c, r.center().x);
                        lined_up && r.x0 < p.x1 + pad && p.x0 < r.x1 + pad && r.y0 < p.y1 && p.y0 < r.y1
                    });
                    if crowded {
                        return true;
                    }
                    self.placed.push((r, baseline_y, r.center().x));
                }
                self.push(Mark::Text(layout, at, col));
            }
        }
        true
    }
}
