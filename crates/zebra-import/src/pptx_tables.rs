//! PowerPoint tables and charts, as Zebra tables and charts (plan-21nov/28).
//!
//! As Keynote does, a table comes in editable and looking as it did: its
//! style is worked out cell by cell (whole table, bands, first and last
//! row and column, then the cell's own settings) and kept as the cells'
//! own fills, lines and text colours over a plain Zebra style. A chart
//! comes in editable, holding the values the file saved, in the series
//! colours it had.
use std::collections::BTreeMap;

use ooxml_drawingml::chart::{ChartSeries, ChartSpace};
use ooxml_drawingml::{ResolvedCellStyle, ShapeOutline, TableCellPosition, TableStyleFlags};
use pptx_parse::Table;
use zebra_core::chart::{ChartData, ChartType, SeriesFormat};
use zebra_core::import::{ImportNote, ImportedItem};
use zebra_core::table::{Border, BorderEdges, Cell, CellFormat, CellRange, CellRow, Column, Gridlines, LineStyle, RangeFormat, Row, TableData, TableStyle, VAlign};
use zebra_core::{Align, Element, ElementKind, Size, TextStyle, Transform};

use super::{Cascade, Placed, SlideReader, EMU_PER_PT};

/// A cell's look after the style cascade, as Zebra keeps it.
#[derive(Clone, PartialEq, Default)]
struct Look {
    fill: Option<String>,
    color: Option<String>,
    bold: bool,
    italic: bool,
    align: Option<Align>,
    valign: Option<VAlign>,
}

/// A line on one edge of a cell: colour, width (local units), style.
#[derive(Clone, PartialEq)]
struct Line {
    color: String,
    width: f64,
    style: LineStyle,
}

/// Transparent: a cell with no fill shows what's behind the table, not
/// the plain style's own fill.
const NO_FILL: &str = "#ffffff00";

impl<'a> SlideReader<'a> {
    /// A table over `at`: rows, columns, merges, text and look.
    pub(super) fn table(&mut self, at: Placed, t: &'a Table, group: Option<&str>) {
        let k = self.k;
        let n_rows = t.rows.len();
        let n_cols = t.grid.len().max(t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0));
        if n_rows == 0 || n_cols == 0 {
            return;
        }
        let p = &t.properties;
        let flags = TableStyleFlags { first_row: p.first_row, last_row: p.last_row, first_column: p.first_col, last_column: p.last_col, band_row: p.band_row };
        let style = p.style_id.as_deref().and_then(|id| self.pkg.table_styles.style(Some(id)).or_else(|| pptx_parse::builtin_table_style(id)));
        // Column bands: the parser doesn't read a style's band1V and
        // band2V, so they're left out and noted.
        if p.style_id.is_some() && (style.is_none() || p.band_col) {
            self.note(ImportNote::TableStyle);
        }
        // Widths from the grid; a column the grid doesn't give shares the
        // frame's leftover width.
        let given: f64 = t.grid.iter().map(|w| (*w).max(0) as f64).sum();
        let spare = ((at.w - given).max(0.0) / (n_cols - t.grid.len().min(n_cols)).max(1) as f64).max(EMU_PER_PT * 36.0);
        let columns: Vec<Column> = (0..n_cols).map(|c| Column::new((t.grid.get(c).map_or(spare, |w| (*w).max(0) as f64).max(EMU_PER_PT) * k).max(1.0))).collect();
        let mut rows: Vec<Row> = t.rows.iter().map(|r| Row::new((r.height.max(0) as f64 * k).max(1.0))).collect();
        let mut grid: Vec<Vec<Cell>> = Vec::with_capacity(n_rows);
        let mut looks: Vec<Vec<Look>> = Vec::with_capacity(n_rows);
        // Lines: horizontal ones above each row (and below the last),
        // vertical ones left of each column (and right of the last).
        let mut h_lines: Vec<Vec<Option<Line>>> = vec![vec![None; n_cols]; n_rows + 1];
        let mut v_lines: Vec<Vec<Option<Line>>> = vec![vec![None; n_cols + 1]; n_rows];
        let mut merges = vec![];
        let mut sizes: BTreeMap<i64, usize> = BTreeMap::new();
        let mut cell_pt: Vec<Vec<f64>> = Vec::with_capacity(n_rows);
        let mut simplified = false;
        for (r, row) in t.rows.iter().enumerate() {
            let mut cells = Vec::with_capacity(n_cols);
            let mut row_looks = Vec::with_capacity(n_cols);
            let mut row_pt = Vec::with_capacity(n_cols);
            for c in 0..n_cols {
                let Some(tc) = row.cells.get(c) else {
                    cells.push(Cell::default());
                    row_looks.push(Look { fill: Some(NO_FILL.into()), ..Default::default() });
                    row_pt.push(0.0);
                    continue;
                };
                let mut rs: ResolvedCellStyle = style.map(|s| s.resolve_cell(flags, TableCellPosition { row: r, column: c, row_count: n_rows, column_count: n_cols })).unwrap_or_default();
                if let Some(f) = &tc.fill {
                    rs.fill = Some(f.clone());
                }
                for (edge, own) in [(&mut rs.left, &tc.borders.left), (&mut rs.right, &tc.borders.right), (&mut rs.top, &tc.borders.top), (&mut rs.bottom, &tc.borders.bottom)] {
                    if let Some(o) = own {
                        *edge = Some(o.clone());
                    }
                }
                if tc.picture_fill.is_some() {
                    self.note(ImportNote::Picture);
                }
                if !tc.merged && (tc.grid_span > 1 || tc.row_span > 1) {
                    let r1 = (r + tc.row_span.max(1) as usize - 1).min(n_rows - 1);
                    let c1 = (c + tc.grid_span.max(1) as usize - 1).min(n_cols - 1);
                    let m = CellRange::new(r, c, r1, c1);
                    // A broken file's merges can overlap: the first one
                    // stays, as the cells it covers do (their text is kept).
                    if (r1 > r || c1 > c) && !merges.iter().any(|o: &CellRange| o.intersects(&m)) {
                        merges.push(m);
                    }
                }
                let fill = rs.fill.as_ref().and_then(|f| self.fill_color(f)).unwrap_or_else(|| NO_FILL.into());
                let style_color = rs.color.clone();
                let cascade = Cascade {
                    primary: Some(&tc.text),
                    layout: None,
                    master: None,
                    placeholder: None,
                    style_color: style_color.as_ref(),
                    style_bold: rs.bold,
                    style_italic: rs.italic,
                    strike: [None; 3],
                };
                let built = if tc.merged { None } else { self.build_text(&cascade, 1.0) };
                let valign = match tc.text.anchor.as_deref() {
                    Some("ctr") => Some(VAlign::Middle),
                    Some("b") => Some(VAlign::Bottom),
                    _ => Some(VAlign::Top),
                };
                // Left unless the text says otherwise (PowerPoint's
                // default, numbers too; Zebra's own would put them right).
                let mut look = Look { fill: Some(fill), valign, align: Some(Align::Left), ..Default::default() };
                let mut cell = Cell::default();
                let mut pt = 0.0;
                if let Some(b) = built {
                    simplified |= b.simplified;
                    pt = b.base_pt;
                    *sizes.entry((b.base_pt * 2.0).round() as i64).or_default() += b.chars;
                    let d = b.data;
                    look.color = d.color.clone();
                    look.bold = d.bold;
                    look.italic = d.italic;
                    look.align = Some(d.align);
                    let mut input = d.text;
                    let mut spans = d.spans;
                    // Never a formula from a file: kept as typed text.
                    if input.starts_with('=') {
                        input.insert(0, '\'');
                        spans.clear();
                    }
                    cell = Cell::new(input);
                    cell.spans = spans;
                } else {
                    look.color = style_color.as_ref().and_then(|c| self.hex(c));
                    look.bold = rs.bold.unwrap_or(false);
                    look.italic = rs.italic.unwrap_or(false);
                }
                cells.push(cell);
                row_looks.push(look);
                row_pt.push(pt);
                // Where two cells share an edge, the one below (or right)
                // wins, as PowerPoint draws it over the other.
                let line = |o: &Option<ShapeOutline>, me: &Self| o.as_ref().map(|o| me.line_of(o));
                if let Some(l) = line(&rs.top, self) {
                    h_lines[r][c] = l;
                }
                if let Some(l) = line(&rs.bottom, self) {
                    if h_lines[r + 1][c].is_none() || r + 1 == n_rows {
                        h_lines[r + 1][c] = l;
                    }
                }
                if let Some(l) = line(&rs.left, self) {
                    v_lines[r][c] = l;
                }
                if let Some(l) = line(&rs.right, self) {
                    if v_lines[r][c + 1].is_none() || c + 1 == n_cols {
                        v_lines[r][c + 1] = l;
                    }
                }
            }
            grid.push(cells);
            looks.push(row_looks);
            cell_pt.push(row_pt);
        }
        if simplified {
            self.deck.note_slide(ImportNote::Simplified, self.number);
        }
        // No lines inside a merged cell.
        for m in &merges {
            for row in v_lines.iter_mut().take(m.r1 + 1).skip(m.r0) {
                for x in m.c0 + 1..=m.c1 {
                    row[x] = None;
                }
            }
            for row in h_lines.iter_mut().take(m.r1 + 1).skip(m.r0 + 1) {
                for l in row.iter_mut().take(m.c1 + 1).skip(m.c0) {
                    *l = None;
                }
            }
        }
        // One text size for the table: the one most characters have. Cells
        // with another keep their words, at the table's size.
        let base_pt = sizes.iter().max_by_key(|(s, n)| (**n, -**s)).map(|(s, _)| *s as f64 / 2.0).unwrap_or(18.0).max(1.0);
        if cell_pt.iter().flatten().any(|pt| *pt > 0.0 && (pt / base_pt - 1.0).abs() > 0.02) {
            self.note(ImportNote::TableText);
        }
        let body = self.zebra.text_style(TextStyle::Body).size.max(1.0);
        let font_size = (base_pt * EMU_PER_PT * k / body).clamp(0.05, 50.0);
        for r in &mut rows {
            r.height = r.height.max(1.0);
        }
        let mut data = TableData::new(1, 1, body);
        data.cells = grid.into_iter().map(CellRow::new).collect();
        data.columns = columns;
        data.rows = rows;
        data.merges = merges;
        data.header_rows = u8::from(p.first_row);
        data.footer_rows = u8::from(p.last_row && n_rows > 1 + usize::from(p.first_row));
        data.header_columns = u8::from(p.first_col);
        data.style = TableStyle::Soft;
        data.banded = Some(false);
        data.gridlines = Some(Gridlines { horizontal: false, vertical: false, outline: false });
        data.font_size = font_size;
        data.cell_formats = formats(&looks, &h_lines, &v_lines);
        let (w, h) = (data.columns.iter().map(|c| c.width).sum::<f64>(), data.rows.iter().map(|r| r.height).sum::<f64>());
        // The table's top-left corner where the frame's is.
        let (x0, y0) = ((at.cx - at.w / 2.0) * k, (at.cy - at.h / 2.0) * k);
        self.items.push(ImportedItem {
            element: Element {
                id: String::new(),
                transform: Transform { x: x0 + w / 2.0, y: y0 + h / 2.0, scale: 1.0, rotation: at.rot },
                size: Size { w, h },
                locked: false,
                name: String::new(),
                placeholder: false,
                group: group.map(str::to_string),
                kind: ElementKind::Table(data),
            },
            fit: None,
        });
    }

    /// A cell edge's line as Zebra draws it (none when it has no colour).
    fn line_of(&self, o: &ShapeOutline) -> Option<Line> {
        let color = o.color.as_ref().and_then(|c| self.hex(c)).or_else(|| o.gradient.as_ref().and_then(|g| g.stops.first()).and_then(|s| self.hex(&s.color)))?;
        let style = match o.style.as_deref() {
            Some("dot" | "sysDot") => LineStyle::Dotted,
            Some("solid") | None => LineStyle::Solid,
            Some(_) => LineStyle::Dashed,
        };
        let width = (o.width.unwrap_or(12_700.0).max(0.0) * self.k).max(0.01);
        Some(Line { color, width, style })
    }

    /// A chart over `at` from the chart part `part` (false: none read).
    pub(super) fn chart(&mut self, at: Placed, part: Option<&str>, group: Option<&str>) -> bool {
        let Some(part) = part else { return false };
        let Some(cs) = self.pkg.charts.iter().find(|c| c.part_path == part).map(|c| &c.chart) else { return false };
        let Some((data, nearest)) = chart_data(cs, self.zebra.text_style(TextStyle::Body).size.max(1.0)) else { return false };
        if nearest {
            self.note(ImportNote::ChartNearest);
        }
        let k = self.k;
        // Scaled so its labels are the size the file gave them (10 pt
        // unless it says), as Zebra sizes a chart's text by its scale.
        let body = self.zebra.text_style(TextStyle::Body).size.max(1.0);
        let label_pt = cs.text.as_ref().and_then(|t| t.size_pt).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(10.0).clamp(1.0, 400.0);
        let s = (label_pt * EMU_PER_PT * k / (body * zebra_core::chart::LABEL_OF_BODY)).clamp(1e-4, 1e4);
        self.items.push(ImportedItem {
            element: Element {
                id: String::new(),
                transform: Transform { x: at.cx * k, y: at.cy * k, scale: s, rotation: at.rot },
                size: Size { w: (at.w * k / s).max(1.0), h: (at.h * k / s).max(1.0) },
                locked: false,
                name: String::new(),
                placeholder: false,
                group: group.map(str::to_string),
                kind: ElementKind::Chart(data),
            },
            fit: None,
        });
        true
    }
}

/// The cells' looks and lines as format ranges: neighbouring cells that
/// look the same share one, and so do rows of them that repeat.
fn formats(looks: &[Vec<Look>], h_lines: &[Vec<Option<Line>>], v_lines: &[Vec<Option<Line>>]) -> Vec<RangeFormat> {
    let mut out: Vec<RangeFormat> = vec![];
    // Runs along each row, then grown down while the next row repeats them.
    let mut open: Vec<(CellRange, CellFormat)> = vec![];
    for (r, row) in looks.iter().enumerate() {
        let mut runs: Vec<(usize, usize, &Look)> = vec![];
        for (c, l) in row.iter().enumerate() {
            match runs.last_mut() {
                Some((_, c1, last)) if *last == l => *c1 = c,
                _ => runs.push((c, c, l)),
            }
        }
        let mut next_open = vec![];
        for (c0, c1, l) in runs {
            let f = CellFormat {
                fill: l.fill.clone(),
                color: l.color.clone(),
                bold: Some(l.bold),
                italic: Some(l.italic),
                align: l.align,
                valign: l.valign,
                ..Default::default()
            };
            if let Some(i) = open.iter().position(|(rg, of)| rg.c0 == c0 && rg.c1 == c1 && rg.r1 + 1 == r && *of == f) {
                let (mut rg, of) = open.swap_remove(i);
                rg.r1 = r;
                next_open.push((rg, of));
            } else {
                next_open.push((CellRange::new(r, c0, r, c1), f));
            }
        }
        out.extend(open.drain(..).map(|(range, format)| RangeFormat { range, format }));
        open = next_open;
    }
    out.extend(open.into_iter().map(|(range, format)| RangeFormat { range, format }));
    let border = |l: &Line, edges: BorderEdges| CellFormat {
        border: Some(Border { edges, style: l.style, color: Some(l.color.clone()), width: Some(l.width) }),
        ..Default::default()
    };
    // Horizontal lines: runs along each line, as the top of the row below
    // (the bottom of the last row for the last line).
    let n_rows = looks.len();
    for (i, line) in h_lines.iter().enumerate() {
        let (r, edges) = if i < n_rows { (i, BorderEdges::Top) } else { (n_rows - 1, BorderEdges::Bottom) };
        let mut c = 0;
        while c < line.len() {
            let Some(l) = &line[c] else {
                c += 1;
                continue;
            };
            let mut c1 = c;
            while c1 + 1 < line.len() && line[c1 + 1].as_ref() == Some(l) {
                c1 += 1;
            }
            out.push(RangeFormat { range: CellRange::new(r, c, r, c1), format: border(l, edges) });
            c = c1 + 1;
        }
    }
    // Vertical lines: runs down each line.
    let n_cols = v_lines.first().map_or(0, |r| r.len().saturating_sub(1));
    for x in 0..=n_cols {
        let (c, edges) = if x < n_cols { (x, BorderEdges::Left) } else { (n_cols.saturating_sub(1), BorderEdges::Right) };
        let mut r = 0;
        while r < v_lines.len() {
            let Some(l) = &v_lines[r][x] else {
                r += 1;
                continue;
            };
            let mut r1 = r;
            while r1 + 1 < v_lines.len() && v_lines[r1 + 1][x].as_ref() == Some(l) {
                r1 += 1;
            }
            out.push(RangeFormat { range: CellRange::new(r, c, r1, c), format: border(l, edges) });
            r = r1 + 1;
        }
    }
    out
}

/// A chart's saved values and settings as a Zebra chart, and whether its
/// type had to be the nearest Zebra has. None when it holds no series.
fn chart_data(cs: &ChartSpace, body: f64) -> Option<(ChartData, bool)> {
    let groups = &cs.plot_groups;
    let all: Vec<(&ChartSeries, usize)> = groups.iter().enumerate().flat_map(|(g, pg)| pg.series.iter().map(move |s| (s, g))).collect();
    if all.is_empty() {
        return None;
    }
    let kind = |g: usize| groups.get(g).and_then(|pg| pg.chart_type.as_deref()).unwrap_or("");
    let grouping = |g: usize| groups.get(g).and_then(|pg| pg.grouping.as_deref()).unwrap_or("");
    let has = |t: &str| groups.iter().any(|pg| pg.chart_type.as_deref() == Some(t));
    let mut nearest = false;
    let mut t = match (kind(0), grouping(0)) {
        ("column", "stacked") => ChartType::StackedColumn,
        ("column", "percentStacked") => ChartType::PercentColumn,
        ("column", _) => ChartType::Column,
        ("bar", "stacked") => ChartType::StackedBar,
        ("bar", "percentStacked") => {
            nearest = true;
            ChartType::StackedBar
        }
        ("bar", _) => ChartType::Bar,
        ("line", _) => ChartType::Line,
        ("area", "stacked") => ChartType::StackedArea,
        ("area", "percentStacked") => {
            nearest = true;
            ChartType::StackedArea
        }
        ("area", _) => ChartType::Area,
        ("pie", _) => ChartType::Pie,
        ("doughnut", _) => ChartType::Donut,
        ("scatter", _) => ChartType::Scatter,
        ("bubble", _) => ChartType::Bubble,
        ("radar", _) => ChartType::Radar,
        ("ofPie", _) => {
            nearest = true;
            ChartType::Pie
        }
        ("surface", _) => {
            nearest = true;
            ChartType::Area
        }
        _ => {
            // Stock and anything else: its values as lines.
            nearest = true;
            ChartType::Line
        }
    };
    // Columns with a line on the same chart.
    let mixed = groups.len() > 1 && has("line") && (has("column") || has("bar"));
    if mixed {
        nearest |= has("bar") || groups.len() > 2;
        t = ChartType::Combo;
    }
    let per = t.values_per_series();
    let n_points = all.iter().map(|(s, _)| s.values.len().max(s.categories.len()).max(s.x_values.as_ref().map_or(0, |x| x.len()))).max().unwrap_or(0);
    let cats: Vec<String> = all.iter().map(|(s, _)| &s.categories).find(|c| !c.is_empty()).cloned().unwrap_or_default();
    let num = |v: Option<&f64>| v.filter(|v| v.is_finite()).map(|v| zebra_core::table::plain_number(*v)).unwrap_or_default();
    let mut head = vec![String::new()];
    for (i, (s, _)) in all.iter().enumerate() {
        head.push(s.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Series {}", i + 1)));
        for _ in 1..per {
            head.push(String::new());
        }
    }
    let mut rows = vec![head];
    for p in 0..n_points {
        let mut row = vec![cats.get(p).cloned().unwrap_or_else(|| (p + 1).to_string())];
        for (s, _) in &all {
            match per {
                1 => row.push(num(s.values.get(p))),
                _ => {
                    row.push(num(s.x_values.as_ref().and_then(|x| x.get(p)).or(Some(&((p + 1) as f64)))));
                    row.push(num(s.values.get(p)));
                    if per == 3 {
                        row.push(num(s.bubble_sizes.as_ref().and_then(|b| b.get(p))));
                    }
                }
            }
        }
        rows.push(row);
    }
    let mut c = ChartData::new(t, &rows, body);
    c.title = cs.title.clone().unwrap_or_default().trim().to_string();
    c.legend = cs.legend.as_ref().is_some_and(|l| l.visible);
    let bars = groups.iter().find(|pg| matches!(pg.chart_type.as_deref(), Some("column" | "bar")));
    if let Some(g) = bars.and_then(|pg| pg.gap_width).filter(|v| v.is_finite()) {
        c.gap_sets = (g / 100.0).clamp(0.0, 5.0);
    }
    if let Some(o) = bars.and_then(|pg| pg.overlap).filter(|v| v.is_finite()) {
        c.gap = (-o / 100.0).clamp(0.0, 1.0);
    }
    if let Some(h) = groups.iter().find_map(|pg| pg.hole_size).filter(|v| v.is_finite()) {
        c.hole = (h / 100.0).clamp(0.1, 0.9);
    }
    c.value_labels = groups.iter().any(|pg| pg.show_data_labels);
    if let Some(ax) = &cs.axes {
        if let Some(v) = &ax.value {
            c.axis_min = v.min.filter(|v| v.is_finite());
            c.axis_max = v.max.filter(|v| v.is_finite());
            c.axis_step = v.major_unit.filter(|v| v.is_finite() && *v > 0.0);
            c.gridlines = v.major_gridlines;
            c.y_title = v.title.clone().unwrap_or_default();
        }
        if let Some(cat) = &ax.category {
            c.x_title = cat.title.clone().unwrap_or_default();
        }
    }
    if let (Some(lo), Some(hi)) = (c.axis_min, c.axis_max) {
        if lo > hi {
            (c.axis_min, c.axis_max) = (Some(hi), Some(lo));
        }
    }
    // The deck's colours stay (a Chart Style clears them).
    c.series = all
        .iter()
        .map(|(s, g)| {
            let color = Some(s.color.trim().to_ascii_lowercase()).filter(|h| h.starts_with('#') && (h.len() == 7 || h.len() == 9) && h[1..].chars().all(|ch| ch.is_ascii_hexdigit()));
            SeriesFormat {
                color: color.map(|h| h[..7].to_string()),
                markers: s.marker.as_ref().and_then(|m| m.symbol.as_deref()).map(|m| m != "none"),
                smooth: s.smooth.unwrap_or(false),
                as_line: (t == ChartType::Combo).then_some(kind(*g) == "line"),
                ..Default::default()
            }
        })
        .collect();
    // A pie's or donut's wedges take their own colours (PowerPoint varies
    // them through the deck's accents, or the file sets each one): Zebra
    // keeps a wedge's colour where a series' would be.
    if matches!(t, ChartType::Pie | ChartType::Donut) {
        if let Some((s, _)) = all.first() {
            let n = s.categories.len().max(s.values.len());
            let hex = |h: &str| Some(h.trim().to_ascii_lowercase()).filter(|h| h.starts_with('#') && (h.len() == 7 || h.len() == 9) && h[1..].chars().all(|ch| ch.is_ascii_hexdigit())).map(|h| h[..7].to_string());
            let points = s.points.as_deref().unwrap_or_default();
            c.series = (0..n)
                .map(|i| SeriesFormat { color: points.iter().find(|p| p.index.is_some_and(|x| x == i as f64)).and_then(|p| hex(&p.color)), ..Default::default() })
                .collect();
        }
    }
    if c.series.iter().all(|f| f.is_empty()) {
        c.series.clear();
    }
    Some((c, nearest))
}
