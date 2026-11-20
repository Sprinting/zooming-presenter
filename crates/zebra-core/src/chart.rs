//! Charts (element id prefix `h`): a chart drawn natively from its own
//! small sheet of data, or from a table on the canvas it is linked to
//! (Link to Table). Laid out and drawn by the renderer; everything here is
//! plain data shared by the editor, the renderer, the web player and the
//! app.
//!
//! The data is a [`TableData`] (the same cells as a table, formulas and
//! all): row 1 holds the series names, column A the category names, and
//! the rest the values; with series in rows, the same the other way round.
//! A linked chart keeps no copy of the table: it reads the table's cells
//! (the saved values of its formulas) when it is drawn. When the link
//! would break (the table deleted, the chart copied without it), the chart
//! takes the values it showed as its own data, in the same undo step.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::model::{ColorRef, Document, ElementId, ElementKind};
use crate::table::{Cell, CellRange, DataFormat, TableData};

/// Most series a chart draws (colours repeat past six).
pub const MAX_SERIES: usize = 64;
/// Most categories a chart draws.
pub const MAX_CATEGORIES: usize = 5_000;
/// Most cells a chart's own data can have (rows x columns).
pub const MAX_CELLS: usize = 10_000;
/// A new chart's size (the mockup's 4:3, a third of a frame).
pub const DEFAULT_W: f64 = 640.0;
pub const DEFAULT_H: f64 = 440.0;
/// Longest label drawn, in characters a person sees (grapheme clusters).
pub const MAX_LABEL: usize = 120;

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_true(b: &bool) -> bool {
    *b
}
fn yes() -> bool {
    true
}
fn one() -> f64 {
    1.0
}
fn is_one(v: &f64) -> bool {
    *v == 1.0
}
fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
fn d_gap() -> f64 {
    0.08
}
fn is_d_gap(v: &f64) -> bool {
    *v == d_gap()
}
fn d_gap_sets() -> f64 {
    0.6
}
fn is_d_gap_sets(v: &f64) -> bool {
    *v == d_gap_sets()
}
fn d_hole() -> f64 {
    0.5
}
fn is_d_hole(v: &f64) -> bool {
    *v == d_hole()
}

macro_rules! catalogue {
    ($(#[$tm:meta])* $name:ident { $($(#[$m:meta])* $v:ident = $id:literal, $code:literal),* $(,)? }) => {
        $(#[$tm])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
        pub enum $name {
            $($(#[$m])* #[serde(rename = $id)] $v),*
        }
        impl $name {
            /// In menu order.
            pub const ALL: &'static [$name] = &[$($name::$v),*];
            /// The name in files and the API (`column`, `stacked_bar`...).
            pub fn key(self) -> &'static str {
                match self { $($name::$v => $id),* }
            }
            /// The id the catalogue gave it (CT1, CL1...).
            pub fn code(self) -> &'static str {
                match self { $($name::$v => $code),* }
            }
            pub fn from_key(s: &str) -> Option<$name> {
                $name::ALL.iter().copied().find(|v| v.key().eq_ignore_ascii_case(s) || v.code().eq_ignore_ascii_case(s))
            }
        }
    };
}

catalogue! {
    /// The chart types (catalogue CT1 to CT16).
    ChartType {
        #[default]
        Column = "column", "CT1",
        StackedColumn = "stacked_column", "CT2",
        PercentColumn = "percent_column", "CT3",
        Bar = "bar", "CT4",
        StackedBar = "stacked_bar", "CT5",
        Line = "line", "CT6",
        Area = "area", "CT7",
        StackedArea = "stacked_area", "CT8",
        Pie = "pie", "CT9",
        Donut = "donut", "CT10",
        Scatter = "scatter", "CT11",
        Bubble = "bubble", "CT12",
        Combo = "combo", "CT13",
        Radar = "radar", "CT14",
        Waterfall = "waterfall", "CT15",
        Step = "step", "CT16",
    }
}

catalogue! {
    /// How the marks and grid are drawn, for any type (catalogue CL1 to
    /// CL7, without CL6): the Chart Style swatches.
    ChartLook {
        #[default]
        Flat = "flat", "CL1",
        Rounded = "rounded", "CL2",
        Outline = "outline", "CL3",
        Tints = "tints", "CL4",
        Labelled = "labelled", "CL5",
        Framed = "framed", "CL7",
    }
}

impl ChartType {
    /// Pie and Donut: one series, no axes.
    pub fn is_round(self) -> bool {
        matches!(self, ChartType::Pie | ChartType::Donut)
    }
    /// Has a value axis and gridlines.
    pub fn has_axes(self) -> bool {
        !self.is_round()
    }
    /// Columns of values per series: 2 for scatter (X, Y), 3 for bubble
    /// (X, Y, size), else 1.
    pub fn values_per_series(self) -> usize {
        match self {
            ChartType::Scatter => 2,
            ChartType::Bubble => 3,
            _ => 1,
        }
    }
    /// Bars or columns (gaps apply).
    pub fn has_bars(self) -> bool {
        matches!(self, ChartType::Column | ChartType::StackedColumn | ChartType::PercentColumn | ChartType::Bar | ChartType::StackedBar | ChartType::Combo | ChartType::Waterfall)
    }
    /// Lines (width, markers, smoothing apply).
    pub fn has_lines(self) -> bool {
        matches!(self, ChartType::Line | ChartType::Combo | ChartType::Radar | ChartType::Step | ChartType::Scatter)
    }
}

/// Which way the data is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SeriesIn {
    /// Each column is a series (row 1 names them).
    #[default]
    Columns,
    /// Each row is a series (column A names them).
    Rows,
}

/// A chart that follows a table on the canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChartLink {
    /// The table's id ("b1").
    pub table: ElementId,
    /// The cells it draws ("A1:C5"). None: the whole table but its footer
    /// rows, growing with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<CellRange>,
}

/// How one series looks (by its index; overrides of the chart's look).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct SeriesFormat {
    /// A theme slot ("theme:shape3") or #rrggbb. None: the theme's next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRef>,
    /// Value labels on this series. None: as the chart has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<bool>,
    /// Line width, in points at the chart's size. None: the look's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_width: Option<f64>,
    /// Dots on the line's points. None: as the type has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markers: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub smooth: bool,
    /// Drawn as a line (Combo). None: the last series is the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_line: Option<bool>,
}

impl SeriesFormat {
    pub fn is_empty(&self) -> bool {
        *self == SeriesFormat::default()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChartData {
    #[serde(default)]
    pub chart_type: ChartType,
    #[serde(default, skip_serializing_if = "is_default")]
    pub look: ChartLook,
    /// Its own data: row 1 the series names, column A the categories.
    pub data: TableData,
    #[serde(default, skip_serializing_if = "is_default")]
    pub series_in: SeriesIn,
    /// Draws from a table instead of `data`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<ChartLink>,
    /// Shown above the chart when not empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub legend: bool,
    /// A line round the chart.
    #[serde(default, skip_serializing_if = "is_false")]
    pub border: bool,
    /// Text size relative to the chart's own (1 = as designed).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub font_size: f64,
    /// Value labels on every series (Labelled turns them on too).
    #[serde(default, skip_serializing_if = "is_false")]
    pub value_labels: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub gridlines: bool,
    /// Gap between the bars of one category, as a share of a bar (0..1).
    #[serde(default = "d_gap", skip_serializing_if = "is_d_gap")]
    pub gap: f64,
    /// Gap between categories, as a share of a bar (0..5).
    #[serde(default = "d_gap_sets", skip_serializing_if = "is_d_gap_sets")]
    pub gap_sets: f64,
    /// Donut hole, as a share of the radius (0.1..0.9).
    #[serde(default = "d_hole", skip_serializing_if = "is_d_hole")]
    pub hole: f64,
    /// Value axis bounds and step. None: worked out from the data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis_max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis_step: Option<f64>,
    /// How the value axis and value labels show numbers.
    #[serde(default, skip_serializing_if = "is_default_format")]
    pub number: DataFormat,
    /// Axis titles (empty: none).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub x_title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub y_title: String,
    /// Waterfall: the categories (0-based) that stand from zero as totals.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub totals: Vec<usize>,
    /// By series index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<SeriesFormat>,
}

fn is_default_format(f: &DataFormat) -> bool {
    *f == DataFormat::default()
}

/// The sample a new chart starts with (four regions, two quarters).
pub const SAMPLE: [[&str; 3]; 5] = [["", "Q2", "Q3"], ["North", "1240", "1410"], ["South", "980", "1050"], ["East", "760", "702"], ["West", "1105", "1230"]];

/// A waterfall's sample: a start, rises and falls, and the total.
pub const SAMPLE_WATERFALL: [[&str; 2]; 6] = [["", "Change"], ["Start", "1200"], ["Sales", "300"], ["Costs", "-180"], ["Other", "120"], ["End", "1440"]];

impl ChartData {
    /// A chart of `chart_type` over `rows` (row 1: series names, column A:
    /// categories), for theme body text `body` high.
    pub fn new(chart_type: ChartType, rows: &[Vec<String>], body: f64) -> ChartData {
        let nr = rows.len().max(1);
        let nc = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
        let mut data = TableData::new(nr, nc, body);
        for (r, row) in rows.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                if let Some(cell) = data.cells.get_mut(r).and_then(|row| row.get_mut(c)) {
                    *cell = Cell::new(v.clone());
                }
            }
        }
        data.header_columns = 1.min(nc as u8);
        ChartData {
            chart_type,
            look: ChartLook::default(),
            data,
            series_in: SeriesIn::default(),
            link: None,
            title: String::new(),
            legend: true,
            border: false,
            font_size: 1.0,
            value_labels: false,
            gridlines: true,
            gap: d_gap(),
            gap_sets: d_gap_sets(),
            hole: d_hole(),
            axis_min: None,
            axis_max: None,
            axis_step: None,
            number: DataFormat::default(),
            x_title: String::new(),
            y_title: String::new(),
            totals: if chart_type == ChartType::Waterfall { vec![nr.saturating_sub(2)] } else { vec![] },
            series: vec![],
        }
    }

    /// The sample data for `chart_type`.
    pub fn sample(chart_type: ChartType, body: f64) -> ChartData {
        let rows: Vec<Vec<String>> = match chart_type {
            ChartType::Waterfall => SAMPLE_WATERFALL.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect(),
            ChartType::Scatter => scatter_sample(false),
            ChartType::Bubble => scatter_sample(true),
            _ => SAMPLE.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect(),
        };
        ChartData::new(chart_type, &rows, body)
    }

    /// The sheet the chart draws from: its own data, or the linked table
    /// (when the link holds).
    pub fn source<'a>(&'a self, doc: &'a Document) -> Source<'a> {
        if let Some(l) = &self.link {
            if let Some(ElementKind::Table(t)) = doc.elements.get(&l.table).map(|e| &e.kind) {
                if let Some(range) = link_range(t, l.range.as_ref()) {
                    return Source { table: t, range, linked: true };
                }
            }
        }
        Source { table: &self.data, range: self.data.whole(), linked: false }
    }

    /// What the chart draws: categories and series, cut to the limits.
    pub fn plot(&self, doc: &Document) -> Plot {
        let s = self.source(doc);
        // Own data always has its names in row 1 and column A; a table's
        // names are its header row and column, or a first row or column
        // that holds no numbers.
        let (names_row, names_col) = if s.linked { s.guess_names() } else { (true, true) };
        extract(&s, self.series_in, self.chart_type, names_row, names_col)
    }

    /// How many series and categories the data holds.
    pub fn shape(&self, doc: &Document) -> (usize, usize) {
        let p = self.plot(doc);
        (p.series.len(), p.categories.len())
    }

    /// Whether `series` is drawn as a line (Combo: the last one unless set).
    pub fn as_line(&self, series: usize, count: usize) -> bool {
        match self.chart_type {
            ChartType::Combo => self.series.get(series).and_then(|f| f.as_line).unwrap_or(count > 1 && series + 1 == count),
            ChartType::Line | ChartType::Step | ChartType::Radar => true,
            _ => false,
        }
    }

    /// The table this chart follows, if any.
    pub fn linked_table(&self) -> Option<&str> {
        self.link.as_ref().map(|l| l.table.as_str())
    }

    /// Unlink, keeping the values the chart showed as its own data
    /// (formulas become their values; names as shown).
    pub fn bake(&mut self, doc: &Document, body: f64) {
        if self.link.is_none() {
            return;
        }
        let s = self.source(doc);
        if s.linked {
            let (names_row, names_col) = s.guess_names();
            let fits = s.range.rows().saturating_mul(s.range.cols()) <= MAX_CELLS;
            // As the table has it, when it fits and names its series and
            // categories; else what the chart draws, named.
            self.data = if fits && names_row && names_col {
                let rows: Vec<Vec<String>> = (s.range.r0..=s.range.r1)
                    .map(|r| {
                        (s.range.c0..=s.range.c1)
                            .map(|c| match s.table.cell(r, c) {
                                Some(cell) if cell.is_formula() => match cell.typed().number() {
                                    Some(v) => crate::table::plain_number(v),
                                    None => s.table.shown(r, c, None).text,
                                },
                                Some(cell) => cell.input.clone(),
                                None => String::new(),
                            })
                            .collect()
                    })
                    .collect();
                ChartData::new(self.chart_type, &rows, body).data
            } else {
                from_plot(self.chart_type, &self.plot(doc), self.series_in, body).data
            };
        }
        self.link = None;
    }

    /// What's wrong with the chart, if anything (see [`ChartData::repair`]).
    pub fn check(&self) -> Result<(), String> {
        self.data.check()?;
        if self.data.n_rows().saturating_mul(self.data.n_cols()) > MAX_CELLS {
            return Err(format!("a chart's data has at most {MAX_CELLS} cells"));
        }
        let fin = |v: f64, lo: f64, hi: f64| v.is_finite() && v >= lo && v <= hi;
        if !fin(self.font_size, 0.25, 4.0) {
            return Err("font_size is 0.25 to 4".into());
        }
        if !(fin(self.gap, 0.0, 1.0) && fin(self.gap_sets, 0.0, 5.0) && fin(self.hole, 0.1, 0.9)) {
            return Err("gap is 0 to 1, gap_sets 0 to 5 and hole 0.1 to 0.9".into());
        }
        if [self.axis_min, self.axis_max].iter().flatten().any(|v| !v.is_finite()) || self.axis_step.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
            return Err("axis_min and axis_max must be finite, axis_step > 0".into());
        }
        if let (Some(a), Some(b)) = (self.axis_min, self.axis_max) {
            if a >= b {
                return Err("axis_min must be below axis_max".into());
            }
        }
        if self.series.len() > MAX_SERIES {
            return Err(format!("at most {MAX_SERIES} series"));
        }
        for f in &self.series {
            if f.color.as_deref().is_some_and(|c| !crate::theme::valid_color(c)) {
                return Err("a series colour is #rrggbb or a theme slot".into());
            }
            if f.line_width.is_some_and(|w| !(w.is_finite() && w > 0.0 && w <= 100.0)) {
                return Err("a series line_width is above 0 and at most 100".into());
            }
        }
        if self.totals.iter().any(|t| *t >= MAX_CATEGORIES) {
            return Err(format!("totals are category indexes below {MAX_CATEGORIES}"));
        }
        Ok(())
    }

    /// Bring every field back in range (a file from another build, a hand
    /// edit). Returns whether anything changed.
    pub fn repair(&mut self, body: f64) -> bool {
        let before = self.clone();
        self.data.repair(body);
        let (nr, nc) = (self.data.n_rows(), self.data.n_cols());
        if nr.saturating_mul(nc) > MAX_CELLS {
            let keep_r = (MAX_CELLS / nc.max(1)).max(1);
            if nr > keep_r {
                self.data.delete_rows(keep_r, nr - keep_r);
            }
            let nc = self.data.n_cols();
            let keep_c = (MAX_CELLS / self.data.n_rows().max(1)).max(1);
            if nc > keep_c {
                self.data.delete_columns(keep_c, nc - keep_c);
            }
        }
        let fix = |v: &mut f64, lo: f64, hi: f64, d: f64| {
            if !v.is_finite() {
                *v = d;
            }
            *v = v.clamp(lo, hi);
        };
        fix(&mut self.font_size, 0.25, 4.0, 1.0);
        fix(&mut self.gap, 0.0, 1.0, d_gap());
        fix(&mut self.gap_sets, 0.0, 5.0, d_gap_sets());
        fix(&mut self.hole, 0.1, 0.9, d_hole());
        for v in [&mut self.axis_min, &mut self.axis_max] {
            if v.is_some_and(|x| !x.is_finite()) {
                *v = None;
            }
        }
        if self.axis_step.is_some_and(|x| !(x.is_finite() && x > 0.0)) {
            self.axis_step = None;
        }
        if let (Some(a), Some(b)) = (self.axis_min, self.axis_max) {
            if a > b {
                self.axis_min = Some(b);
                self.axis_max = Some(a);
            } else if a == b {
                self.axis_max = None;
            }
        }
        self.series.truncate(MAX_SERIES);
        for f in &mut self.series {
            if f.color.as_deref().is_some_and(|c| !crate::theme::valid_color(c)) {
                f.color = None;
            }
            if f.line_width.is_some_and(|w| !(w.is_finite() && w > 0.0 && w <= 100.0)) {
                f.line_width = None;
            }
        }
        while self.series.last().is_some_and(SeriesFormat::is_empty) {
            self.series.pop();
        }
        self.totals.retain(|t| *t < MAX_CATEGORIES);
        self.totals.sort_unstable();
        self.totals.dedup();
        for s in [&mut self.title, &mut self.x_title, &mut self.y_title] {
            if s.chars().count() > crate::table::MAX_INPUT {
                *s = s.chars().take(crate::table::MAX_INPUT).collect();
            }
        }
        *self != before
    }
}

fn scatter_sample(bubble: bool) -> Vec<Vec<String>> {
    let pts: [(f64, f64, f64, f64, f64, f64); 4] =
        [(2.0, 3.0, 4.0, 3.5, 5.0, 3.0), (4.0, 5.0, 2.0, 6.0, 4.5, 5.0), (6.0, 6.5, 3.0, 7.0, 7.0, 2.0), (8.0, 9.0, 6.0, 9.5, 8.5, 4.0)];
    let mut head = vec![String::new()];
    let per = if bubble { 3 } else { 2 };
    for name in ["Q2", "Q3"] {
        head.push(name.into());
        for _ in 1..per {
            head.push(String::new());
        }
    }
    let mut rows = vec![head];
    for (i, p) in pts.iter().enumerate() {
        let mut row = vec![format!("{}", i + 1)];
        let vals: Vec<f64> = if bubble { vec![p.0, p.1, p.2, p.3, p.4, p.5] } else { vec![p.0, p.1, p.3, p.4] };
        row.extend(vals.iter().map(|v| crate::table::plain_number(*v)));
        rows.push(row);
    }
    rows
}

/// Own data for what `plot` shows (names in row 1 and column A).
fn from_plot(chart_type: ChartType, plot: &Plot, series_in: SeriesIn, body: f64) -> ChartData {
    let per = chart_type.values_per_series();
    let mut rows: Vec<Vec<String>> = vec![];
    let mut head = vec![String::new()];
    for s in &plot.series {
        head.push(s.name.clone());
        for _ in 1..per {
            head.push(String::new());
        }
    }
    rows.push(head);
    let num = |v: f64| if v.is_finite() { crate::table::plain_number(v) } else { String::new() };
    for (i, cat) in plot.categories.iter().enumerate() {
        let mut row = vec![cat.clone()];
        for s in &plot.series {
            match per {
                2 => {
                    row.push(num(s.x.get(i).copied().unwrap_or(f64::NAN)));
                    row.push(num(s.values.get(i).copied().unwrap_or(f64::NAN)));
                }
                3 => {
                    row.push(num(s.x.get(i).copied().unwrap_or(f64::NAN)));
                    row.push(num(s.values.get(i).copied().unwrap_or(f64::NAN)));
                    row.push(num(s.sizes.get(i).copied().unwrap_or(f64::NAN)));
                }
                _ => row.push(num(s.values.get(i).copied().unwrap_or(f64::NAN))),
            }
        }
        rows.push(row);
    }
    if series_in == SeriesIn::Rows {
        rows = transpose(&rows);
    }
    ChartData::new(chart_type, &rows, body)
}

fn transpose(rows: &[Vec<String>]) -> Vec<Vec<String>> {
    let nc = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    (0..nc).map(|c| rows.iter().map(|r| r.get(c).cloned().unwrap_or_default()).collect()).collect()
}

/// The cells of `t` a link with `range` draws: the range clipped to the
/// table, or the whole table without its footer rows. None when nothing
/// is left.
pub fn link_range(t: &TableData, range: Option<&CellRange>) -> Option<CellRange> {
    match range {
        Some(r) => r.clip(t.n_rows(), t.n_cols()),
        None => {
            let last = t.n_rows().saturating_sub(t.footer_rows as usize).max(1) - 1;
            (t.n_cols() > 0).then(|| CellRange::new(0, 0, last, t.n_cols() - 1))
        }
    }
}

/// Where a chart's numbers come from.
pub struct Source<'a> {
    pub table: &'a TableData,
    pub range: CellRange,
    /// From a linked table (else the chart's own data).
    pub linked: bool,
}

impl Source<'_> {
    fn number(&self, r: usize, c: usize) -> f64 {
        self.table.cell(r, c).and_then(|cell| cell.typed().number()).filter(|v| v.is_finite()).unwrap_or(f64::NAN)
    }

    fn name(&self, r: usize, c: usize) -> String {
        let shown = self.table.shown(r, c, None).text;
        label(&shown)
    }

    /// Whether the first row and column of the range are names: the
    /// table's header row and column, or ones that hold no numbers.
    pub fn guess_names(&self) -> (bool, bool) {
        let r = self.range;
        let t = self.table;
        let no_number = |r: usize, c: usize| t.cell(r, c).is_none_or(|x| x.is_empty() || x.typed().number().is_none());
        let row = r.r0 < t.header_rows as usize || (r.rows() > 1 && (r.c0 + 1..=r.c1).all(|c| no_number(r.r0, c)));
        let col = r.c0 < t.header_columns as usize || (r.cols() > 1 && (r.r0 + 1..=r.r1).all(|rr| no_number(rr, r.c0)));
        (row, col)
    }
}

/// A label as drawn: one line, at most [`MAX_LABEL`] characters (never
/// splitting one).
pub fn label(s: &str) -> String {
    let one_line = s.lines().next().unwrap_or("").trim();
    let mut g = one_line.graphemes(true);
    let cut: String = g.by_ref().take(MAX_LABEL).collect();
    if g.next().is_some() {
        let mut c: String = cut.graphemes(true).take(MAX_LABEL - 1).collect();
        c.push('…');
        c
    } else {
        cut
    }
}

/// One series as drawn.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlotSeries {
    pub name: String,
    /// By category; NaN for a gap.
    pub values: Vec<f64>,
    /// Scatter and bubble: x by point.
    pub x: Vec<f64>,
    /// Bubble: size by point.
    pub sizes: Vec<f64>,
}

/// What a chart draws.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plot {
    pub categories: Vec<String>,
    pub series: Vec<PlotSeries>,
    /// Some data was left out (over the limits).
    pub cut: bool,
}

/// Read `s` into categories and series.
pub fn extract(s: &Source, series_in: SeriesIn, chart_type: ChartType, names_row: bool, names_col: bool) -> Plot {
    let r = s.range;
    // In "series" terms: `lanes` run across series, `points` along them.
    let (lane0, lane1, point0, point1) = match series_in {
        SeriesIn::Columns => (r.c0, r.c1, r.r0, r.r1),
        SeriesIn::Rows => (r.r0, r.r1, r.c0, r.c1),
    };
    let (names_lane, names_point) = match series_in {
        SeriesIn::Columns => (names_row, names_col),
        SeriesIn::Rows => (names_col, names_row),
    };
    let at = |lane: usize, point: usize| match series_in {
        SeriesIn::Columns => (point, lane),
        SeriesIn::Rows => (lane, point),
    };
    let first_point = point0 + names_lane as usize;
    let first_lane = lane0 + names_point as usize;
    let mut plot = Plot::default();
    if first_point > point1 || first_lane > lane1 {
        return plot;
    }
    let per = chart_type.values_per_series();
    let lanes = lane1 - first_lane + 1;
    let n_series = lanes.div_ceil(per);
    let take = n_series.min(MAX_SERIES);
    plot.cut |= n_series > take;
    // At most MAX_CELLS cells with the names, so what is drawn always fits
    // a chart's own data.
    let n_points = point1 - first_point + 1;
    let points = n_points.min(MAX_CATEGORIES).min((MAX_CELLS / (take * per + 1)).saturating_sub(1).max(1));
    plot.cut |= n_points > points;
    plot.categories = (0..points)
        .map(|i| {
            let p = first_point + i;
            if names_point {
                let (rr, cc) = at(lane0, p);
                s.name(rr, cc)
            } else {
                format!("{}", i + 1)
            }
        })
        .collect();
    for k in 0..take {
        let l0 = first_lane + k * per;
        let name = if names_lane {
            // A scatter's name sits over its Y column, or its X.
            let lanes_here: Vec<usize> = (l0..(l0 + per).min(lane1 + 1)).collect();
            lanes_here
                .iter()
                .map(|l| {
                    let (rr, cc) = at(*l, point0);
                    s.name(rr, cc)
                })
                .find(|n| !n.is_empty())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let name = if name.is_empty() { format!("Series {}", k + 1) } else { name };
        let col = |off: usize| -> Vec<f64> {
            let lane = l0 + off;
            (0..points)
                .map(|i| {
                    if lane > lane1 {
                        return f64::NAN;
                    }
                    let (rr, cc) = at(lane, first_point + i);
                    s.number(rr, cc)
                })
                .collect()
        };
        let series = match per {
            2 => PlotSeries { name, x: col(0), values: col(1), sizes: vec![] },
            3 => PlotSeries { name, x: col(0), values: col(1), sizes: col(2) },
            _ => PlotSeries { name, values: col(0), x: vec![], sizes: vec![] },
        };
        plot.series.push(series);
    }
    plot
}

/// Where a linked range goes when its table changes shape (`rows` x
/// `cols` before the change): as a formula's range does. Rows inserted
/// inside it grow it; deleted ones shrink it; moved ones take it with them
/// (it then spans where its cells went). None when every cell of it was
/// deleted.
pub fn move_range(r: CellRange, op: &crate::calc::Reshape, rows: usize, cols: usize) -> Option<CellRange> {
    use crate::calc::Reshape as R;
    fn insert(a: usize, b: usize, at: usize, n: usize) -> (usize, usize) {
        (if at <= a { a + n } else { a }, if at <= b { b + n } else { b })
    }
    fn delete(a: usize, b: usize, at: usize, n: usize) -> Option<(usize, usize)> {
        let end = at.saturating_add(n);
        let map = |i: usize| if i < at { i } else { i - n };
        let first = (a..=b).find(|i| *i < at || *i >= end)?;
        let last = (a..=b).rev().find(|i| *i < at || *i >= end)?;
        Some((map(first), map(last)))
    }
    fn moved(a: usize, b: usize, n: usize, from: usize, count: usize, to: usize) -> (usize, usize) {
        let order = TableData::move_order(n, from, count, to);
        let mut lo = usize::MAX;
        let mut hi = 0;
        for (new, old) in order.iter().enumerate() {
            if (a..=b).contains(old) {
                lo = lo.min(new);
                hi = hi.max(new);
            }
        }
        if lo == usize::MAX { (a, b) } else { (lo, hi) }
    }
    let (mut r0, mut r1, mut c0, mut c1) = (r.r0, r.r1, r.c0, r.c1);
    match *op {
        R::InsertRows { at, count } => (r0, r1) = insert(r0, r1, at, count),
        R::InsertColumns { at, count } => (c0, c1) = insert(c0, c1, at, count),
        R::DeleteRows { at, count } => (r0, r1) = delete(r0, r1, at, count)?,
        R::DeleteColumns { at, count } => (c0, c1) = delete(c0, c1, at, count)?,
        R::MoveRows { from, count, to } => (r0, r1) = moved(r0, r1, rows, from, count, to),
        R::MoveColumns { from, count, to } => (c0, c1) = moved(c0, c1, cols, from, count, to),
        // A sort reorders values inside the body; the range stays.
        R::SortRows { .. } => {}
    }
    Some(CellRange::new(r0, c0, r1, c1))
}

/// The limit a chart's own data would break at `rows` x `cols`, if any.
pub fn over_limit(rows: usize, cols: usize) -> bool {
    rows == 0 || cols == 0 || rows.saturating_mul(cols) > MAX_CELLS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own(rows: &[&[&str]]) -> ChartData {
        let rows: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect();
        ChartData::new(ChartType::Column, &rows, 28.0)
    }

    #[test]
    fn reads_series_by_column_or_row() {
        let c = own(&[&["", "Q2", "Q3"], &["North", "1240", "1410"], &["South", "980", "x"]]);
        let doc = Document::default();
        let p = c.plot(&doc);
        assert_eq!(p.categories, ["North", "South"]);
        assert_eq!(p.series.len(), 2);
        assert_eq!(p.series[0].name, "Q2");
        assert_eq!(p.series[0].values, [1240.0, 980.0]);
        assert!(p.series[1].values[1].is_nan(), "text is a gap");
        let mut r = c.clone();
        r.series_in = SeriesIn::Rows;
        let p = r.plot(&doc);
        assert_eq!(p.categories, ["Q2", "Q3"]);
        assert_eq!(p.series[0].name, "North");
        assert_eq!(p.series[0].values, [1240.0, 1410.0]);
    }

    #[test]
    fn scatter_and_bubble_read_pairs_and_triples() {
        let doc = Document::default();
        let s = ChartData::sample(ChartType::Scatter, 28.0);
        let p = s.plot(&doc);
        assert_eq!(p.series.len(), 2);
        assert_eq!(p.series[0].x.len(), 4);
        assert_eq!(p.series[0].name, "Q2");
        let b = ChartData::sample(ChartType::Bubble, 28.0);
        let p = b.plot(&doc);
        assert_eq!(p.series.len(), 2);
        assert_eq!(p.series[1].sizes.len(), 4);
    }

    #[test]
    fn labels_never_split_a_character() {
        let flag = "🇬🇧";
        let long: String = std::iter::repeat_n(flag, 200).collect();
        let l = label(&long);
        assert!(l.ends_with('…'));
        assert_eq!(l.graphemes(true).count(), MAX_LABEL);
        assert!(l.trim_end_matches('…').graphemes(true).all(|g| g == flag));
        assert_eq!(label("one\ntwo"), "one");
    }

    #[test]
    fn empty_and_degenerate_data_draw_nothing() {
        let doc = Document::default();
        for rows in [&[&[""][..]][..], &[&["", "Q2"][..]][..], &[&["", ""][..], &["", ""][..]][..]] {
            let p = own(rows).plot(&doc);
            assert!(p.series.iter().all(|s| s.values.iter().all(|v| v.is_nan())), "{p:?}");
        }
    }

    #[test]
    fn linked_ranges_move_as_formula_ranges_do() {
        use crate::calc::Reshape as R;
        let r = CellRange::parse("B2:C5").unwrap();
        let m = |op: R| move_range(r, &op, 10, 10).map(|r| r.a1());
        assert_eq!(m(R::InsertRows { at: 0, count: 2 }).unwrap(), "B4:C7");
        assert_eq!(m(R::InsertRows { at: 3, count: 1 }).unwrap(), "B2:C6");
        assert_eq!(m(R::InsertRows { at: 5, count: 1 }).unwrap(), "B2:C5");
        assert_eq!(m(R::InsertColumns { at: 1, count: 1 }).unwrap(), "C2:D5");
        assert_eq!(m(R::DeleteRows { at: 0, count: 2 }).unwrap(), "B1:C3");
        assert_eq!(m(R::DeleteRows { at: 2, count: 1 }).unwrap(), "B2:C4");
        assert_eq!(m(R::DeleteColumns { at: 1, count: 2 }), None);
        assert_eq!(m(R::MoveRows { from: 1, count: 4, to: 8 }).unwrap(), "B5:C8");
        assert_eq!(m(R::SortRows { order: vec![] }).unwrap(), "B2:C5");
    }

    #[test]
    fn repair_brings_options_back_in_range() {
        let mut c = own(&[&["", "a"], &["x", "1"]]);
        c.font_size = f64::NAN;
        c.gap = -3.0;
        c.hole = 7.0;
        c.axis_min = Some(10.0);
        c.axis_max = Some(1.0);
        c.axis_step = Some(0.0);
        c.series = vec![SeriesFormat { color: Some("not a colour".into()), line_width: Some(f64::INFINITY), ..Default::default() }];
        assert!(c.repair(28.0));
        assert_eq!(c.font_size, 1.0);
        assert_eq!(c.gap, 0.0);
        assert_eq!(c.hole, 0.9);
        assert_eq!((c.axis_min, c.axis_max, c.axis_step), (Some(1.0), Some(10.0), None));
        assert!(c.series.is_empty());
        assert!(!c.repair(28.0));
    }
}
