//! Tables in the display list.
//!
//! Only the rows and columns on screen are drawn (the screen is mapped
//! back into the table and the row and column edges searched). Far away a
//! table is drawn as it is when its text is too small to read: a grey bar
//! for each cell's text, and with rows under a pixel high, a block for
//! each part of the table. Colours come from the style in the theme, with
//! contrast enforced cell by cell ([`zebra_core::table_style`]).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use super::*;
use zebra_core::table::{self, BorderEdges, CellRange, LineStyle, NumLook, Role, TableData, VAlign};
use zebra_core::table_style::{self, FootRule, Look, THIN};
use zebra_core::{Align, TextData, TextStyle};

/// Rows shorter than this on screen (device px) draw their text as bars.
pub const ROW_MIN_PIXELS: f64 = 4.0;
/// Rows shorter than this draw each part of the table as one block.
pub const ROW_BLOCK_PIXELS: f64 = 1.0;

/// Column looks ([`TableData::column_look`]) of the tables drawn lately,
/// worked out when a column's cells change and not on every frame.
#[derive(Default)]
pub(crate) struct LookCache {
    tables: HashMap<String, (u64, Vec<Option<Option<NumLook>>>)>,
}

impl LookCache {
    const MAX_TABLES: usize = 256;
}

/// What a table's column looks depend on: its rows (by identity: an
/// edited row is a new one) and how many are headers.
fn signature(t: &TableData) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.header_rows.hash(&mut h);
    t.n_cols().hash(&mut h);
    for row in &t.cells {
        (&**row as *const Vec<table::Cell> as usize).hash(&mut h);
    }
    h.finish()
}

/// A line along a row or column edge, in table units.
fn hline(items: &mut Vec<Item>, tl: &Affine, x0: f64, x1: f64, y: f64, w: f64, color: Rgba) {
    if x1 > x0 && w > 0.0 {
        fill(items, &Rect::new(x0, y - w / 2.0, x1, y + w / 2.0), tl, color);
    }
}

fn vline(items: &mut Vec<Item>, tl: &Affine, x: f64, y0: f64, y1: f64, w: f64, color: Rgba) {
    if y1 > y0 && w > 0.0 {
        fill(items, &Rect::new(x - w / 2.0, y0, x + w / 2.0, y1), tl, color);
    }
}

/// One cell edge: `H(e, c)` is the line at the top of row `e` over column
/// `c` (`e` = the row count for the bottom); `V(r, e)` the line at the left
/// of column `e` beside row `r`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Edge {
    H(usize, usize),
    V(usize, usize),
}

/// The edges in view that cell borders own, each with the index of the
/// format that owns it (the last one to set it wins, as it applies last).
type Owners = HashMap<Edge, usize>;

fn border_owners(t: &TableData, v: View) -> Owners {
    let mut out = Owners::new();
    let view = CellRange::new(v.r0, v.c0, v.r1, v.c1);
    for (i, rf) in t.cell_formats.iter().enumerate() {
        let Some(b) = &rf.format.border else { continue };
        let Some(r) = rf.range.clip(t.n_rows(), t.n_cols()) else { continue };
        if !r.intersects(&view) {
            continue;
        }
        let (top, bottom, left, right, inner_h, inner_v) = match b.edges {
            BorderEdges::All => (true, true, true, true, true, true),
            BorderEdges::Outer => (true, true, true, true, false, false),
            BorderEdges::Inner => (false, false, false, false, true, true),
            BorderEdges::Horizontal => (false, false, false, false, true, false),
            BorderEdges::Vertical => (false, false, false, false, false, true),
            BorderEdges::Top => (true, false, false, false, false, false),
            BorderEdges::Bottom => (false, true, false, false, false, false),
            BorderEdges::Left => (false, false, true, false, false, false),
            BorderEdges::Right => (false, false, false, true, false, false),
        };
        // Only what is in view: a screenful of edges at most.
        let (cs, rs) = (r.c0.max(v.c0)..=r.c1.min(v.c1), r.r0.max(v.r0)..=r.r1.min(v.r1));
        let mut put = |e: Edge| {
            out.insert(e, i);
        };
        for c in cs.clone() {
            if top {
                put(Edge::H(r.r0, c));
            }
            if bottom {
                put(Edge::H(r.r1 + 1, c));
            }
            if inner_h {
                for e in (r.r0 + 1).max(v.r0)..=r.r1.min(v.r1 + 1) {
                    put(Edge::H(e, c));
                }
            }
        }
        for row in rs {
            if left {
                put(Edge::V(row, r.c0));
            }
            if right {
                put(Edge::V(row, r.c1 + 1));
            }
            if inner_v {
                for e in (r.c0 + 1).max(v.c0)..=r.c1.min(v.c1 + 1) {
                    put(Edge::V(row, e));
                }
            }
        }
    }
    out
}

fn on_outline(e: &Edge, rows: usize, cols: usize) -> bool {
    match *e {
        Edge::H(r, _) => r == 0 || r == rows,
        Edge::V(_, c) => c == 0 || c == cols,
    }
}

/// Edges joined into straight runs: (first, last) of each run of
/// neighbours along the same line.
fn runs(edges: impl Iterator<Item = Edge>) -> Vec<(Edge, Edge)> {
    let mut v: Vec<Edge> = edges.collect();
    v.sort();
    v.dedup();
    let mut out: Vec<(Edge, Edge)> = vec![];
    for e in v {
        if let Some((_, last)) = out.last_mut() {
            let next = match (*last, e) {
                (Edge::H(a, c), Edge::H(b, d)) => a == b && d == c + 1,
                (Edge::V(r, a), Edge::V(s, b)) => a == b && s == r + 1,
                _ => false,
            };
            if next {
                *last = e;
                continue;
            }
        }
        out.push((e, e));
    }
    out
}

/// Stroke a run of edges from `a` to `z`.
fn segment(items: &mut Vec<Item>, l: &table::Layout, tl: &Affine, a: Edge, z: Edge, color: Rgba, st: Stroke) {
    let (p, q) = match (a, z) {
        (Edge::H(e, c0), Edge::H(_, c1)) => ((l.xs[c0], l.ys[e]), (l.xs[c1 + 1], l.ys[e])),
        (Edge::V(r0, e), Edge::V(r1, _)) => ((l.xs[e], l.ys[r0]), (l.xs[e], l.ys[r1 + 1])),
        _ => return,
    };
    let mut path = BezPath::new();
    path.move_to(p);
    path.line_to(q);
    stroke(items, &path, tl, color, st);
}

/// The part of the table on screen: rows r0..=r1, columns c0..=c1.
#[derive(Debug, Clone, Copy)]
struct View {
    r0: usize,
    r1: usize,
    c0: usize,
    c1: usize,
}

impl Ctx<'_> {
    pub(super) fn table(&self, e: &Element, t: &TableData, a: &Affine, items: &mut Vec<Item>) {
        let th = self.theme;
        if t.n_rows() == 0 || t.n_cols() == 0 {
            return;
        }
        let body = th.text_style(TextStyle::Body).size;
        let l = t.layout(body);
        let (w, h) = (e.size.w, e.size.h);
        let tl = *a * Affine::translate((-w / 2.0, -h / 2.0));
        let k = affine_scale(a);
        if !(k.is_finite() && k > 0.0) {
            return;
        }
        let look = table_style::look(t, th);
        let rows_h = l.height() - l.caption;
        let row_px = rows_h / t.n_rows() as f64 * k;
        let font_px = l.font * k;

        if t.show_name && !e.name.trim().is_empty() && l.caption > 0.0 {
            self.caption(&e.name, &l, t, &look, &tl, font_px, items);
        }
        if row_px < ROW_BLOCK_PIXELS {
            self.blocks(t, &l, &look, &tl, items);
            return;
        }
        let Some(v) = self.on_screen(&l, &tl) else { return };
        let bars = !self.full_res_images && (font_px < TEXT_MIN_PIXELS || row_px < ROW_MIN_PIXELS);

        // Merges that reach into view, by each cell they cover.
        let view = CellRange::new(v.r0, v.c0, v.r1, v.c1);
        let mut merged: HashMap<(usize, usize), CellRange> = HashMap::new();
        for m in t.merges.iter().filter(|m| m.intersects(&view)) {
            for r in m.r0.max(v.r0)..=m.r1.min(v.r1) {
                for c in m.c0.max(v.c0)..=m.c1.min(v.c1) {
                    merged.insert((r, c), *m);
                }
            }
        }
        // Each cell (or merge) once, by its top-left.
        let mut cells: Vec<(CellRange, table_style::CellPaint)> = Vec::new();
        for r in v.r0..=v.r1 {
            for c in v.c0..=v.c1 {
                let span = merged.get(&(r, c)).copied().unwrap_or(CellRange::cell(r, c));
                let first = (r, c) == (span.r0.max(v.r0), span.c0.max(v.c0));
                if !first {
                    continue;
                }
                let paint = table_style::cell_paint(t, th, &look, span.r0, span.c0);
                cells.push((span, paint));
            }
        }

        // Fills, a run of equal ones along a row as one rectangle.
        let mut run: Option<(f64, f64, f64, f64, Rgba)> = None;
        let flush = |run: &mut Option<(f64, f64, f64, f64, Rgba)>, items: &mut Vec<Item>| {
            if let Some((x0, y0, x1, y1, col)) = run.take() {
                fill(items, &Rect::new(x0, y0, x1, y1), &tl, col);
            }
        };
        for (span, paint) in &cells {
            let Some(f) = paint.fill else {
                flush(&mut run, items);
                continue;
            };
            let (x0, y0, x1, y1) = l.range_box(span);
            match &mut run {
                Some((_, ry0, rx1, ry1, rc)) if *ry0 == y0 && *ry1 == y1 && *rc == f && (*rx1 - x0).abs() < 1e-9 => *rx1 = x1,
                _ => {
                    flush(&mut run, items);
                    run = Some((x0, y0, x1, y1, f));
                }
            }
        }
        flush(&mut run, items);

        // Cell borders own their edges: the style's lines stay off them,
        // and "None" leaves them bare.
        let owners = border_owners(t, v);
        self.rules(t, &l, &look, &tl, v, &merged, row_px, &owners, items);
        self.borders(t, &l, &tl, &owners, items);

        let mut looks = self.column_looks(e, t);
        for (span, paint) in &cells {
            self.cell_text(t, &l, &tl, *span, paint, bars, &mut looks, items);
        }
        if let Ok(mut cache) = self.r.table_looks.lock() {
            if cache.tables.len() >= LookCache::MAX_TABLES {
                cache.tables.clear();
            }
            cache.tables.insert(e.id.clone(), looks);
        }
    }

    /// The rows and columns the screen shows, or None.
    fn on_screen(&self, l: &table::Layout, tl: &Affine) -> Option<View> {
        let (sw, sh) = (self.screen.0 as f64, self.screen.1 as f64);
        let det = tl.determinant();
        if !det.is_finite() || det.abs() < 1e-18 {
            return None;
        }
        let inv = tl.inverse();
        let pts = [Point::new(0.0, 0.0), Point::new(sw, 0.0), Point::new(0.0, sh), Point::new(sw, sh)].map(|p| inv * p);
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in pts {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return None;
        }
        let top = l.ys.first().copied().unwrap_or(0.0);
        if x1 < 0.0 || y1 < top || x0 > l.width() || y0 > l.height() {
            return None;
        }
        Some(View { r0: l.row_at(y0), r1: l.row_at(y1), c0: l.col_at(x0), c1: l.col_at(x1) })
    }

    fn caption(&self, name: &str, l: &table::Layout, t: &TableData, look: &Look, tl: &Affine, font_px: f64, items: &mut Vec<Item>) {
        let th = self.theme;
        let color = table_style::readable(th.color(&"theme:title".to_string()), look.canvas, table_style::MIN_TEXT, th);
        let size = 1.15 * t.font_size;
        if font_px < TEXT_MIN_PIXELS && !self.full_res_images {
            let len = (name.chars().count() as f64 * 0.55 * l.font * 1.15).min(l.width());
            fill(items, &Rect::new(0.0, l.caption * 0.3, len, l.caption * 0.65), tl, with_alpha(color, 0.35));
            return;
        }
        let td = TextData { text: name.chars().take(500).collect(), bold: true, ..Default::default() };
        let layout = self.r.fonts.layout(&td, th, None);
        let y = ((l.caption - layout.height * size) / 2.0 - l.font * 0.1).max(0.0);
        let transform = *tl * Affine::translate((0.0, y)) * Affine::scale(size);
        items.push(Item::Text { layout, transform, color, run_colors: Arc::from([]) });
    }

    /// Rows under a pixel: header, body and footer as blocks, each column's
    /// text as a faint band.
    fn blocks(&self, t: &TableData, l: &table::Layout, look: &Look, tl: &Affine, items: &mut Vec<Item>) {
        let n = t.n_rows();
        let hr = (t.header_rows as usize).min(n);
        let fr = (t.footer_rows as usize).min(n - hr);
        let y = |r: usize| l.ys[r.min(n)];
        let width = l.width();
        let body_fill = match (look.body_fill, look.band_fill) {
            (b, Some(band)) => Some(table_style::mix(band, b.unwrap_or(look.canvas), 0.5)),
            (b, None) => b,
        };
        let parts = [(0, hr, look.header_fill, look.header_text), (hr, n - fr, body_fill, look.body_text), (n - fr, n, look.footer_fill.or(look.body_fill), look.body_text)];
        for (a, b, f, _) in parts {
            if let Some(f) = f {
                if b > a {
                    fill(items, &Rect::new(0.0, y(a), width, y(b)), tl, f);
                }
            }
        }
        for (a, b, f, text) in parts {
            if b <= a {
                continue;
            }
            let under = f.unwrap_or(look.canvas);
            let col = with_alpha(table_style::readable(text, under, 1.0, self.theme), 0.18);
            for c in 0..t.n_cols() {
                let (x0, x1) = (l.xs[c] + l.pad_x, l.xs[c + 1] - l.pad_x);
                if x1 > x0 {
                    fill(items, &Rect::new(x0, y(a), x1, y(b)), tl, col);
                }
            }
        }
        if let Some((c, wd)) = look.header_rule {
            if hr > 0 {
                hline(items, tl, 0.0, width, y(hr), wd * l.font, c);
            }
        }
        if fr > 0 {
            self.foot_rule(look, l, tl, 0.0, width, y(n - fr), items);
        }
        if let Some((c, wd)) = look.outline {
            stroke(items, &Rect::new(0.0, y(0), width, y(n)), tl, c, Stroke::new(wd * l.font));
        }
    }

    fn foot_rule(&self, look: &Look, l: &table::Layout, tl: &Affine, x0: f64, x1: f64, y: f64, items: &mut Vec<Item>) {
        match look.footer_rule {
            FootRule::None => {}
            FootRule::Line(c, wd) => hline(items, tl, x0, x1, y, wd * l.font, c),
            FootRule::Double(c) => {
                let wd = THIN * l.font;
                hline(items, tl, x0, x1, y - wd * 1.5, wd, c);
                hline(items, tl, x0, x1, y + wd * 1.5, wd, c);
            }
        }
    }

    /// The style's lines over the part on screen; none through a merge.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn rules(&self, t: &TableData, l: &table::Layout, look: &Look, tl: &Affine, v: View, merged: &HashMap<(usize, usize), CellRange>, row_px: f64, owners: &Owners, items: &mut Vec<Item>) {
        let n = t.n_rows();
        let hr = t.header_rows as usize;
        let foot = n.saturating_sub(t.footer_rows as usize);
        let thin = THIN * l.font;
        // Inside a merge across the edge below row r at column c?
        let inside_h = |r: usize, c: usize| merged.get(&(r, c)).is_some_and(|m| m.r1 > r);
        let inside_v = |r: usize, c: usize| merged.get(&(r, c)).is_some_and(|m| m.c1 > c);
        // Horizontal edges below rows v.r0 - 1 ..= v.r1 (edge e lies at ys[e]).
        let e0 = v.r0.max(1);
        let e1 = (v.r1 + 1).min(n - 1);
        for edge in e0..=e1 {
            if edge > n - 1 {
                break;
            }
            let (color, width, double) = if hr > 0 && edge == hr && hr < n {
                match look.header_rule {
                    Some((c, wd)) => (Some(c), wd * l.font, false),
                    None => (look.row_rule, thin, false),
                }
            } else if t.footer_rows > 0 && edge == foot && foot > hr {
                match look.footer_rule {
                    FootRule::None => (look.row_rule, thin, false),
                    FootRule::Line(c, wd) => (Some(c), wd * l.font, false),
                    FootRule::Double(c) => (Some(c), thin, true),
                }
            } else if row_px >= 3.0 {
                (look.row_rule, thin, false)
            } else {
                (None, 0.0, false)
            };
            let Some(color) = color else { continue };
            let y = l.ys[edge];
            let mut start: Option<f64> = None;
            for c in v.c0..=v.c1 + 1 {
                let open = c <= v.c1 && !inside_h(edge - 1, c) && !owners.contains_key(&Edge::H(edge, c));
                match (open, start) {
                    (true, None) => start = Some(l.xs[c]),
                    (false, Some(x0)) => {
                        if double {
                            hline(items, tl, x0, l.xs[c], y - width * 1.5, width, color);
                            hline(items, tl, x0, l.xs[c], y + width * 1.5, width, color);
                        } else {
                            hline(items, tl, x0, l.xs[c], y, width, color);
                        }
                        start = None;
                    }
                    _ => {}
                }
            }
        }
        if let Some(color) = look.column_rule {
            for edge in v.c0.max(1)..=(v.c1 + 1).min(t.n_cols() - 1) {
                let x = l.xs[edge];
                let mut start: Option<f64> = None;
                for r in v.r0..=v.r1 + 1 {
                    let open = r <= v.r1 && !inside_v(r, edge - 1) && !owners.contains_key(&Edge::V(r, edge));
                    match (open, start) {
                        (true, None) => start = Some(l.ys[r]),
                        (false, Some(y0)) => {
                            vline(items, tl, x, y0, l.ys[r], thin, color);
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
        }
        if let Some((c, wd)) = look.outline {
            let (rows, cols) = (t.n_rows(), t.n_cols());
            if !owners.keys().any(|e| on_outline(e, rows, cols)) {
                stroke(items, &Rect::new(0.0, l.ys[0], l.width(), l.height()), tl, c, Stroke::new(wd * l.font));
            } else {
                // Some of the outline belongs to cell borders: the rest, edge by edge.
                let st = Stroke::new(wd * l.font).with_caps(Cap::Square);
                let edges = (v.c0..=v.c1)
                    .flat_map(|c| [Edge::H(0, c), Edge::H(rows, c)])
                    .chain((v.r0..=v.r1).flat_map(|r| [Edge::V(r, 0), Edge::V(r, cols)]))
                    .filter(|e| !owners.contains_key(e));
                for (a, b) in runs(edges) {
                    segment(items, l, tl, a, b, c, st.clone());
                }
            }
        }
    }

    /// Borders set on ranges, over the part on screen.
    fn borders(&self, t: &TableData, l: &table::Layout, tl: &Affine, owners: &Owners, items: &mut Vec<Item>) {
        let th = self.theme;
        // Runs of edges owned by the same format, each drawn in its style.
        let mut by: HashMap<usize, Vec<Edge>> = HashMap::new();
        for (e, i) in owners {
            by.entry(*i).or_default().push(*e);
        }
        for (i, edges) in by {
            let Some(b) = t.cell_formats.get(i).and_then(|f| f.format.border.as_ref()) else { continue };
            if b.style == LineStyle::None {
                continue;
            }
            let width = b.width.filter(|w| w.is_finite() && *w > 0.0).unwrap_or(1.0).min(l.font);
            let color = match &b.color {
                Some(c) if !c.is_empty() => th.color(c),
                _ => th.color(&"theme:frame".to_string()),
            };
            let st = match b.style {
                LineStyle::Solid | LineStyle::None => Stroke::new(width).with_caps(Cap::Butt),
                LineStyle::Dashed => Stroke::new(width).with_caps(Cap::Butt).with_dashes(0.0, [width * 3.0, width * 2.0]),
                LineStyle::Dotted => Stroke::new(width).with_caps(Cap::Round).with_dashes(0.0, [0.0, width * 2.0]),
            };
            for (a, z) in runs(edges.into_iter()) {
                segment(items, l, tl, a, z, color, st.clone());
            }
        }
    }

    /// The column looks of `t` from the cache, if its rows are the same.
    fn column_looks(&self, e: &Element, t: &TableData) -> (u64, Vec<Option<Option<NumLook>>>) {
        let sig = signature(t);
        if let Ok(mut cache) = self.r.table_looks.lock() {
            if let Some((s, v)) = cache.tables.remove(&e.id) {
                if s == sig && v.len() == t.n_cols() {
                    return (s, v);
                }
            }
        }
        (sig, vec![None; t.n_cols()])
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_text(&self, t: &TableData, l: &table::Layout, tl: &Affine, span: CellRange, paint: &table_style::CellPaint, bars: bool, looks: &mut (u64, Vec<Option<Option<NumLook>>>), items: &mut Vec<Item>) {
        let th = self.theme;
        let (r, c) = (span.r0, span.c0);
        let Some(cell) = t.cell(r, c) else { return };
        if cell.is_empty() && t.data_format(r, c).kind != table::CellType::TickBox {
            return;
        }
        // An Automatic formula result looks like the column's numbers.
        let needs_look = cell.is_formula() && cell.format.is_none();
        let look = if needs_look {
            let slot = &mut looks.1[c];
            if slot.is_none() {
                *slot = Some(t.column_look(c));
            }
            slot.as_ref().and_then(|x| x.as_ref())
        } else {
            None
        };
        let shown = t.shown(r, c, look);
        let (x0, y0, x1, y1) = l.range_box(&span);
        let f = t.format_at(r, c);
        let valign = f.valign.unwrap_or_default();
        let align = t.align_at(r, c, &shown);
        let (ax0, ax1) = (x0 + l.pad_x, x1 - l.pad_x);
        let avail = ax1 - ax0;
        if avail <= 0.0 || y1 <= y0 {
            return;
        }
        let color = if shown.error.is_some() || shown.red { paint.red } else { paint.text };
        let fs = t.font_size;

        if let Some(on) = shown.tick {
            let side = (l.font * 0.8).min(avail).min(y1 - y0);
            let cx = match align {
                Align::Left => ax0 + side / 2.0,
                Align::Right => ax1 - side / 2.0,
                Align::Center => (x0 + x1) / 2.0,
            };
            let cy = (y0 + y1) / 2.0;
            let rect = Rect::new(cx - side / 2.0, cy - side / 2.0, cx + side / 2.0, cy + side / 2.0);
            let rr = RoundedRect::from_rect(rect, side * 0.18);
            if on {
                fill(items, &rr, tl, paint.text);
                let mut p = BezPath::new();
                p.move_to((cx - side * 0.27, cy + side * 0.02));
                p.line_to((cx - side * 0.07, cy + side * 0.22));
                p.line_to((cx + side * 0.28, cy - side * 0.2));
                stroke(items, &p, tl, paint.under, Stroke::new(side * 0.13).with_caps(Cap::Round).with_join(Join::Round));
            } else {
                stroke(items, &rr, tl, paint.text, Stroke::new(side * 0.08));
            }
            return;
        }
        if shown.text.is_empty() {
            return;
        }
        if shown.error.is_some() {
            // The small red triangle in the corner.
            let s = (l.font * 0.45).min((y1 - y0) * 0.5).min(x1 - x0);
            let mut p = BezPath::new();
            p.move_to((x0, y0));
            p.line_to((x0 + s, y0));
            p.line_to((x0, y0 + s));
            p.close_path();
            fill(items, &p, tl, paint.mark);
        }
        let bold = f.bold.unwrap_or(!matches!(t.role(r, c), Role::Body { .. }));
        let italic = f.italic.unwrap_or(false);
        let one_line = shown.numeric || shown.error.is_some();
        if bars {
            // As small text is drawn: a faint bar per cell.
            let len = (shown.text.chars().count() as f64 * 0.5 * l.font).min(avail);
            let bx0 = match align {
                Align::Left => ax0,
                Align::Right => ax1 - len,
                Align::Center => (ax0 + ax1 - len) / 2.0,
            };
            let cy = (y0 + y1) / 2.0;
            let bh = (l.font * 0.5).min((y1 - y0) * 0.6);
            fill(items, &Rect::new(bx0, cy - bh / 2.0, bx0 + len, cy + bh / 2.0), tl, with_alpha(color, 0.35));
            return;
        }
        let text: String = table::drawn_text(&shown.text).chars().take(table::MAX_INPUT).collect();
        let (layout, x) = if one_line {
            let td = TextData { text, bold, italic, style: TextStyle::Body, ..Default::default() };
            let lay = self.r.fonts.layout(&td, th, None);
            if lay.width * fs > avail + 1e-6 && shown.numeric {
                // Too narrow for the number.
                let td = TextData { text: table::HASHES.into(), bold, italic, ..Default::default() };
                let lay = self.r.fonts.layout(&td, th, None);
                (lay.clone(), place(align, ax0, ax1, lay.width * fs))
            } else {
                let wd = lay.width * fs;
                (lay, place(align, ax0, ax1, wd))
            }
        } else {
            let td = TextData { text, bold, italic, align, fixed_width: true, ..Default::default() };
            (self.r.fonts.layout(&td, th, Some(avail / fs)), ax0)
        };
        let lh = layout.height * fs;
        let y = match valign {
            VAlign::Top => y0 + l.pad_y,
            VAlign::Middle => (y0 + y1 - lh) / 2.0,
            VAlign::Bottom => y1 - l.pad_y - lh,
        };
        let overflows = layout.width * fs > avail + 1e-6 || lh > (y1 - y0) + 1e-6;
        if overflows {
            let mut clip = Rect::new(x0, y0, x1, y1).to_path(0.01);
            clip.apply_affine(*tl);
            items.push(Item::PushClip(clip));
        }
        let transform = *tl * Affine::translate((x, y)) * Affine::scale(fs);
        items.push(Item::Text { layout, transform, color, run_colors: Arc::from([]) });
        if overflows {
            items.push(Item::PopClip);
        }
    }
}

/// Left edge of something `w` wide aligned between `x0` and `x1`.
fn place(a: Align, x0: f64, x1: f64, w: f64) -> f64 {
    match a {
        Align::Left => x0,
        Align::Right => x1 - w,
        Align::Center => (x0 + x1 - w) / 2.0,
    }
}
