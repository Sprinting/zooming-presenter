//! Tables (element id prefix `b`, as in board): a grid of cells drawn
//! natively, recoloured by the theme, with Excel-style formulas worked out
//! by IronCalc ([`crate::calc`]).
//!
//! What a cell keeps: what was typed (`input`: `1240`, `North`,
//! `=SUM(B2:B5)`) and, for a formula, the value last worked out, so a file
//! opens, plays and exports without recalculating. A plain input is its
//! own value. The same cell format serves a chart's data later
//! ([`Grid`]).
//!
//! Everything here is plain data and arithmetic on it, shared by the
//! editor, the renderer, the web player and the app: layout (column and
//! row edges, merged cells, hit testing), what a cell shows (number
//! formats, `###`, errors), and the structural edits (rows and columns
//! in and out, merges, sorting). Formula references are rewritten by the
//! editor through [`crate::calc`] around these.
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{Align, ColorRef, TextSpan};

/// Most header rows, header columns or footer rows (as in Keynote).
pub const MAX_HEADERS: u8 = 5;
/// Most rows and columns a table can have.
pub const MAX_ROWS: usize = 10_000;
pub const MAX_COLUMNS: usize = 256;
/// Most cells a table can have (rows x columns).
pub const MAX_CELLS: usize = 100_000;
/// Longest input a cell takes, in characters (Excel's limit).
pub const MAX_INPUT: usize = 32_767;
/// Longest formula, in characters (Excel's limit).
pub const MAX_FORMULA: usize = 8_192;

/// Why an input can't go in a cell, or None when it can.
pub fn input_problem(s: &str) -> Option<InputProblem> {
    // Bytes bound characters from above: most inputs skip the count.
    if s.len() <= MAX_FORMULA {
        return None;
    }
    let n = s.chars().count();
    if is_formula(s) && n > MAX_FORMULA {
        Some(InputProblem::FormulaTooLong)
    } else if n > MAX_INPUT {
        Some(InputProblem::TooLong)
    } else {
        None
    }
}

/// See [`input_problem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputProblem {
    TooLong,
    FormulaTooLong,
}

/// Whether a `rows` x `cols` table is within the limits.
pub fn fits_limits(rows: usize, cols: usize) -> bool {
    rows >= 1 && cols >= 1 && rows <= MAX_ROWS && cols <= MAX_COLUMNS && rows.saturating_mul(cols) <= MAX_CELLS
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A number no other row or column has had in this run of the app.
pub(crate) fn fresh_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_true(b: &bool) -> bool {
    *b
}
fn yes() -> bool {
    true
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
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
fn d_header_rows() -> u8 {
    1
}

/// Rows of cells: what a table holds, and (later) a chart's own data.
pub type Grid = Vec<CellRow>;

/// One row of cells.
///
/// The cells are shared between copies of a table (the undo journal's
/// copy of what an edit touched, a renderer's snapshot) until one of the
/// copies changes the row: copying a table copies a pointer per row, and
/// changing a cell copies just its row ([`DerefMut`] does that). The `id`
/// stays with the row however it moves, so an undo step can tell rows
/// inserted, deleted or reordered from rows edited ([`crate::delta`]).
/// It is not saved.
#[derive(Debug, Clone)]
pub struct CellRow {
    id: u64,
    cells: Arc<Vec<Cell>>,
}

impl CellRow {
    pub fn new(cells: Vec<Cell>) -> CellRow {
        CellRow { id: fresh_id(), cells: Arc::new(cells) }
    }
    /// `n` empty cells.
    pub fn empty(n: usize) -> CellRow {
        CellRow::new(vec![Cell::default(); n])
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    /// The same cells in memory (not just equal ones).
    pub fn same(&self, o: &CellRow) -> bool {
        Arc::ptr_eq(&self.cells, &o.cells)
    }
    /// Replace the cells, keeping the row's id.
    pub fn set(&mut self, cells: Vec<Cell>) {
        self.cells = Arc::new(cells);
    }
}

impl Deref for CellRow {
    type Target = Vec<Cell>;
    fn deref(&self) -> &Vec<Cell> {
        &self.cells
    }
}

impl DerefMut for CellRow {
    fn deref_mut(&mut self) -> &mut Vec<Cell> {
        Arc::make_mut(&mut self.cells)
    }
}

impl<'a> IntoIterator for &'a CellRow {
    type Item = &'a Cell;
    type IntoIter = std::slice::Iter<'a, Cell>;
    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter()
    }
}

impl PartialEq for CellRow {
    fn eq(&self, o: &CellRow) -> bool {
        self.same(o) || *self.cells == *o.cells
    }
}

impl Serialize for CellRow {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.cells.serialize(s)
    }
}

impl<'de> Deserialize<'de> for CellRow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(CellRow::new(Vec::<Cell>::deserialize(d)?))
    }
}

impl JsonSchema for CellRow {
    fn schema_name() -> Cow<'static, str> {
        "CellRow".into()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        Vec::<Cell>::json_schema(g)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TableData {
    /// Rows of cells, top to bottom; every row has one cell per column.
    pub cells: Grid,
    /// Width and type of each column, left to right.
    pub columns: Vec<Column>,
    /// Height of each row, top to bottom.
    pub rows: Vec<Row>,
    /// Merged ranges ("A1:C1"). The top-left cell holds the content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merges: Vec<CellRange>,
    /// 0 to 5.
    #[serde(default = "d_header_rows")]
    pub header_rows: u8,
    /// 0 to 5.
    #[serde(default, skip_serializing_if = "is_default")]
    pub header_columns: u8,
    /// 0 to 5.
    #[serde(default, skip_serializing_if = "is_default")]
    pub footer_rows: u8,
    #[serde(default)]
    pub style: TableStyle,
    /// Formatting set on ranges of cells, later entries over earlier ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cell_formats: Vec<RangeFormat>,
    /// Text size relative to the theme's body text (1 = the same).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub font_size: f64,
    /// Alternating row colour. None: as the style has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banded: Option<bool>,
    /// Gridlines. None: as the style has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gridlines: Option<Gridlines>,
    /// Show the table's name (the element's `name`) above it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_name: bool,
}

/// One cell: what was typed and, for a formula, its last value.
///
/// In a file a plain cell is just its text (`"North"`, `"1240"`); a
/// formula keeps its value: `{"input": "=SUM(B2:B5)", "value": 4085}`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cell {
    /// What was typed. A formula starts with `=`.
    pub input: String,
    /// The value last worked out, for a formula (None for plain inputs,
    /// which are their own value).
    pub value: Option<CellValue>,
    /// The number format the formula engine gave the value (a date from
    /// `TODAY()`), Excel style. Used when the cell's format is Automatic.
    pub format: Option<String>,
    /// Character formatting on parts of the text (from an import).
    pub spans: Vec<TextSpan>,
}

impl Cell {
    pub fn new(input: impl Into<String>) -> Cell {
        Cell { input: input.into(), ..Default::default() }
    }
    pub fn is_formula(&self) -> bool {
        is_formula(&self.input)
    }
    pub fn is_empty(&self) -> bool {
        self.input.trim().is_empty()
    }
    /// What the cell's value is: the worked-out value of a formula, else
    /// the input read as a number, date, tick or text.
    pub fn typed(&self) -> Typed {
        if self.is_formula() {
            return match &self.value {
                None => Typed::Empty,
                Some(CellValue::Number(v)) => Typed::Number(*v, NumLook::default()),
                Some(CellValue::Bool(b)) => Typed::Bool(*b),
                Some(CellValue::Text(t)) if t.is_empty() => Typed::Empty,
                Some(CellValue::Text(t)) => Typed::Text(t.clone()),
                Some(CellValue::Error { error }) => Typed::Error(error.clone()),
            };
        }
        parse_input(&self.input)
    }
}

/// Starts with `=` and has more after it.
pub fn is_formula(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with('=') && t.len() > 1
}

/// A worked-out value. In a file: a number, `true`/`false`, a string, or
/// `{"error": "#DIV/0!"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum CellValue {
    Number(f64),
    Bool(bool),
    Text(String),
    Error { error: String },
}

/// How a cell is written in a file.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
enum CellRepr {
    /// A plain input: text, or a number as typed.
    Plain(String),
    /// A number written as a JSON number (read only).
    Number(f64),
    /// true / false (read only).
    Bool(bool),
    /// A formula with its value, or text with character formatting.
    Full {
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<CellValue>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        spans: Vec<TextSpan>,
    },
}

impl Serialize for Cell {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.value.is_none() && self.format.is_none() && self.spans.is_empty() {
            CellRepr::Plain(self.input.clone()).serialize(s)
        } else {
            CellRepr::Full { input: self.input.clone(), value: self.value.clone(), format: self.format.clone(), spans: self.spans.clone() }.serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for Cell {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match CellRepr::deserialize(d)? {
            CellRepr::Plain(input) => Cell::new(input),
            CellRepr::Number(v) => Cell::new(plain_number(v)),
            CellRepr::Bool(b) => Cell::new(if b { "TRUE" } else { "FALSE" }),
            CellRepr::Full { input, value, format, spans } => Cell { input, value, format, spans },
        })
    }
}

impl JsonSchema for Cell {
    fn schema_name() -> Cow<'static, str> {
        "Cell".into()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        CellRepr::json_schema(g)
    }
}

/// A number as a plain input ("1240", "0.5", "-3").
pub fn plain_number(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// A value given in a command: text, a number or true/false.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum CellInput {
    Text(String),
    Number(f64),
    Bool(bool),
    Empty(()),
}

impl CellInput {
    pub fn into_input(self) -> String {
        match self {
            CellInput::Text(s) => s,
            CellInput::Number(v) => plain_number(v),
            CellInput::Bool(b) => (if b { "TRUE" } else { "FALSE" }).into(),
            CellInput::Empty(()) => String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Column {
    /// Local units.
    pub width: f64,
    /// The column type and its number format: what its cells show when
    /// they have no format of their own, and how sorting compares them.
    #[serde(flatten)]
    pub format: DataFormat,
    /// Stays with the column however it moves (see [`CellRow`]). Not saved.
    #[serde(skip, default = "fresh_id")]
    #[schemars(skip)]
    pub(crate) id: u64,
}

impl Column {
    pub fn new(width: f64) -> Column {
        Column { width, format: DataFormat::default(), id: fresh_id() }
    }
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl PartialEq for Column {
    fn eq(&self, o: &Column) -> bool {
        self.width == o.width && self.format == o.format
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Row {
    /// Smallest height, local units. Rows grow to fit wrapped text.
    pub height: f64,
    /// The height its text needs (worked out by the editor), when more.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fit: f64,
}

impl Row {
    pub fn new(height: f64) -> Row {
        Row { height, fit: 0.0 }
    }
    /// The height drawn.
    pub fn h(&self) -> f64 {
        self.height.max(self.fit)
    }
}

/// What a column holds (its type) and a cell's Data Format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum CellType {
    /// As typed: numbers right, text left.
    #[default]
    Automatic,
    Text,
    Number,
    Currency,
    Percentage,
    DateTime,
    TickBox,
}

impl CellType {
    pub const ALL: [CellType; 7] =
        [CellType::Automatic, CellType::Number, CellType::Currency, CellType::Percentage, CellType::DateTime, CellType::Text, CellType::TickBox];
}

/// How negative numbers show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Negative {
    /// −100
    #[default]
    Minus,
    /// −100 in red
    Red,
    /// (100)
    Parentheses,
    /// (100) in red
    RedParentheses,
}

impl Negative {
    pub const ALL: [Negative; 4] = [Negative::Minus, Negative::Red, Negative::Parentheses, Negative::RedParentheses];
    pub fn red(self) -> bool {
        matches!(self, Negative::Red | Negative::RedParentheses)
    }
}

/// A Data Format: the type, decimals, thousands separator, currency and
/// negative numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DataFormat {
    #[serde(rename = "type", default, skip_serializing_if = "is_default")]
    pub kind: CellType,
    /// Digits after the point. None: as many as the number has (up to 10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    /// 1,240,000 rather than 1240000.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub thousands: bool,
    /// Currency symbol. Empty: £.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub currency: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub negative: Negative,
}

impl Default for DataFormat {
    fn default() -> Self {
        DataFormat { kind: CellType::Automatic, decimals: None, thousands: true, currency: String::new(), negative: Negative::Minus }
    }
}

impl DataFormat {
    pub fn of(kind: CellType) -> DataFormat {
        DataFormat { kind, ..Default::default() }
    }
    pub fn symbol(&self) -> &str {
        if self.currency.is_empty() { "£" } else { &self.currency }
    }
}

/// A rectangle of cells, 0-based and inclusive. Written "A1:C3" (or "B2"
/// for one cell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRange {
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
}

impl CellRange {
    pub fn new(r0: usize, c0: usize, r1: usize, c1: usize) -> CellRange {
        CellRange { r0: r0.min(r1), c0: c0.min(c1), r1: r0.max(r1), c1: c0.max(c1) }
    }
    pub fn cell(r: usize, c: usize) -> CellRange {
        CellRange { r0: r, c0: c, r1: r, c1: c }
    }
    pub fn contains(&self, r: usize, c: usize) -> bool {
        r >= self.r0 && r <= self.r1 && c >= self.c0 && c <= self.c1
    }
    pub fn covers(&self, o: &CellRange) -> bool {
        self.r0 <= o.r0 && self.r1 >= o.r1 && self.c0 <= o.c0 && self.c1 >= o.c1
    }
    pub fn intersects(&self, o: &CellRange) -> bool {
        self.r0 <= o.r1 && o.r0 <= self.r1 && self.c0 <= o.c1 && o.c0 <= self.c1
    }
    pub fn is_single(&self) -> bool {
        self.r0 == self.r1 && self.c0 == self.c1
    }
    pub fn rows(&self) -> usize {
        self.r1 - self.r0 + 1
    }
    pub fn cols(&self) -> usize {
        self.c1 - self.c0 + 1
    }
    pub fn union(&self, o: &CellRange) -> CellRange {
        CellRange { r0: self.r0.min(o.r0), c0: self.c0.min(o.c0), r1: self.r1.max(o.r1), c1: self.c1.max(o.c1) }
    }
    /// "A1:C3", or "B2" for one cell.
    pub fn a1(&self) -> String {
        if self.is_single() {
            cell_name(self.r0, self.c0)
        } else {
            format!("{}:{}", cell_name(self.r0, self.c0), cell_name(self.r1, self.c1))
        }
    }
    /// "A1:C3", "B2", or a whole column or row: "B:B", "3:3".
    pub fn parse(s: &str) -> Option<CellRange> {
        let s = s.trim();
        match s.split_once(':') {
            None => parse_cell(s).map(|(r, c)| CellRange::cell(r, c)),
            Some((a, b)) => {
                let (a, b) = (a.trim(), b.trim());
                if let (Some((r0, c0)), Some((r1, c1))) = (parse_cell(a), parse_cell(b)) {
                    return Some(CellRange::new(r0, c0, r1, c1));
                }
                if let (Some(c0), Some(c1)) = (parse_column(a), parse_column(b)) {
                    return Some(CellRange::new(0, c0, usize::MAX, c1));
                }
                if let (Ok(r0), Ok(r1)) = (a.parse::<usize>(), b.parse::<usize>()) {
                    if r0 >= 1 && r1 >= 1 {
                        return Some(CellRange::new(r0 - 1, 0, r1 - 1, usize::MAX));
                    }
                }
                None
            }
        }
    }
    /// Clipped to a `rows` x `cols` table (a whole column or row given as
    /// "B:B" or "3:3" ends at the table's edge).
    pub fn clip(&self, rows: usize, cols: usize) -> Option<CellRange> {
        if rows == 0 || cols == 0 || self.r0 >= rows || self.c0 >= cols {
            return None;
        }
        Some(CellRange { r0: self.r0, c0: self.c0, r1: self.r1.min(rows - 1), c1: self.c1.min(cols - 1) })
    }
}

impl Serialize for CellRange {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.a1())
    }
}

impl<'de> Deserialize<'de> for CellRange {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        CellRange::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("'{s}' is not a cell range like A1:C3")))
    }
}

impl JsonSchema for CellRange {
    fn schema_name() -> Cow<'static, str> {
        "CellRange".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "Cells in A1 notation: \"B2\", \"A1:C3\"; a whole column \"B:B\" or row \"3:3\".",
            "pattern": "^\\s*([A-Za-z]{1,3}[0-9]+(:[A-Za-z]{1,3}[0-9]+)?|[A-Za-z]{1,3}:[A-Za-z]{1,3}|[0-9]+:[0-9]+)\\s*$"
        })
    }
}

/// "A", "B", ... "Z", "AA" for a 0-based column.
pub fn column_name(mut c: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// "B2" for row 1, column 1 (0-based).
pub fn cell_name(r: usize, c: usize) -> String {
    format!("{}{}", column_name(c), r + 1)
}

/// "B" → 1.
pub fn parse_column(s: &str) -> Option<usize> {
    let s = s.trim().trim_start_matches('$');
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut n = 0usize;
    for b in s.bytes() {
        n = n * 26 + (b.to_ascii_uppercase() - b'A') as usize + 1;
    }
    Some(n - 1)
}

/// "B2" → (1, 1), 0-based (row, column). `$` marks are allowed.
pub fn parse_cell(s: &str) -> Option<(usize, usize)> {
    let s = s.trim();
    let split = s.find(|ch: char| ch.is_ascii_digit())?;
    let (letters, digits) = s.split_at(split);
    let c = parse_column(letters.trim_end_matches('$'))?;
    let r: usize = digits.parse().ok()?;
    (r >= 1).then(|| (r - 1, c))
}

/// Formatting on a range of cells.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RangeFormat {
    pub range: CellRange,
    #[serde(flatten)]
    pub format: CellFormat,
}

/// Formatting of cells. Unset fields leave what is there; an empty colour
/// goes back to the table style's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct CellFormat {
    /// Fill: `#rrggbb`, `#rrggbbaa` or a theme slot. "" = the style's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<ColorRef>,
    /// Text colour. "" = the style's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// Horizontal alignment. Without one, numbers go right and text left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valign: Option<VAlign>,
    /// Data Format: type, decimals, thousands separator, currency, negatives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<DataFormat>,
    /// Lines on the edges of the range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
}

impl CellFormat {
    pub fn is_empty(&self) -> bool {
        *self == CellFormat::default()
    }
    /// Take `o`'s set fields over ours.
    pub fn overlay(&mut self, o: &CellFormat) {
        macro_rules! take {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        take!(fill, color, bold, italic, align, valign, number, border);
    }
    /// Unset every field `o` sets.
    fn without(&mut self, o: &CellFormat) {
        macro_rules! drop_ {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = None; } )* };
        }
        drop_!(fill, color, bold, italic, align, valign, number);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// Lines on some edges of a range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Border {
    pub edges: BorderEdges,
    #[serde(default, skip_serializing_if = "is_default")]
    pub style: LineStyle,
    /// Default: the theme's frame colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRef>,
    /// Points (local units at the table's scale). Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
}

/// Which edges of a range a border goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BorderEdges {
    /// Every edge, inside and out.
    All,
    /// Around the range.
    Outer,
    /// Between the cells.
    Inner,
    /// Between the rows.
    Horizontal,
    /// Between the columns.
    Vertical,
    Top,
    Bottom,
    Left,
    Right,
}

impl BorderEdges {
    pub const ALL: [BorderEdges; 6] = [BorderEdges::All, BorderEdges::Outer, BorderEdges::Horizontal, BorderEdges::Vertical, BorderEdges::Top, BorderEdges::Bottom];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum LineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    /// No line (takes the table style's away).
    None,
}

/// Table-wide gridlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct Gridlines {
    /// Lines between rows.
    #[serde(default)]
    pub horizontal: bool,
    /// Lines between columns.
    #[serde(default)]
    pub vertical: bool,
    /// A line around the table.
    #[serde(default)]
    pub outline: bool,
}

macro_rules! table_styles {
    ($($(#[$m:meta])* $v:ident => $id:literal, $name:literal),* $(,)?) => {
        /// A table style. Its colours are theme slots, so a theme change
        /// recolours the table.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
        pub enum TableStyle {
            $($(#[$m])* #[serde(rename = $id)] $v),*
        }
        impl TableStyle {
            /// In menu order.
            pub const ALL: [TableStyle; 10] = [$(TableStyle::$v),*];
            /// The id in files and commands ("ts4").
            pub fn id(self) -> &'static str {
                match self { $(TableStyle::$v => $id),* }
            }
            /// The English name (the app has its own, translated).
            pub fn name(self) -> &'static str {
                match self { $(TableStyle::$v => $name),* }
            }
            pub fn from_id(s: &str) -> Option<TableStyle> {
                match s { $($id => Some(TableStyle::$v),)* _ => None }
            }
        }
    };
}

table_styles! {
    /// A filled header band.
    Banner => "ts1", "Banner",
    /// Header band and banded rows.
    Banded => "ts2", "Banded",
    /// Every cell boxed.
    Grid => "ts3", "Grid",
    /// Lines only, header in the title colour. The default.
    #[default]
    Ruled => "ts4", "Ruled",
    /// Header in the accent colour.
    Accent => "ts6", "Accent",
    /// Header row and a shaded header column.
    SideHeader => "ts7", "Side Header",
    /// Header band in a box.
    Boxed => "ts8", "Boxed",
    /// Soft fills, no lines.
    Soft => "ts9", "Soft",
    /// Dark whatever the theme.
    Dark => "ts10", "Dark",
    /// Banded with a double rule over the total.
    Ledger => "ts11", "Ledger",
}

/// Everything in a table but its cells, columns and rows: what an undo
/// step keeps whole when it changes (it is small).
#[derive(Debug, Clone, PartialEq)]
pub struct TableMeta {
    pub merges: Vec<CellRange>,
    pub header_rows: u8,
    pub header_columns: u8,
    pub footer_rows: u8,
    pub style: TableStyle,
    pub cell_formats: Vec<RangeFormat>,
    pub font_size: f64,
    pub banded: Option<bool>,
    pub gridlines: Option<Gridlines>,
    pub show_name: bool,
}

impl TableData {
    pub fn meta(&self) -> TableMeta {
        TableMeta {
            merges: self.merges.clone(),
            header_rows: self.header_rows,
            header_columns: self.header_columns,
            footer_rows: self.footer_rows,
            style: self.style,
            cell_formats: self.cell_formats.clone(),
            font_size: self.font_size,
            banded: self.banded,
            gridlines: self.gridlines,
            show_name: self.show_name,
        }
    }
    /// Same as `o.meta()`, without copying either.
    pub fn same_meta(&self, o: &TableData) -> bool {
        self.merges == o.merges
            && self.header_rows == o.header_rows
            && self.header_columns == o.header_columns
            && self.footer_rows == o.footer_rows
            && self.style == o.style
            && self.cell_formats == o.cell_formats
            && (self.font_size == o.font_size || self.font_size.is_nan() && o.font_size.is_nan())
            && self.banded == o.banded
            && self.gridlines == o.gridlines
            && self.show_name == o.show_name
    }
    pub fn set_meta(&mut self, m: TableMeta) {
        self.merges = m.merges;
        self.header_rows = m.header_rows;
        self.header_columns = m.header_columns;
        self.footer_rows = m.footer_rows;
        self.style = m.style;
        self.cell_formats = m.cell_formats;
        self.font_size = m.font_size;
        self.banded = m.banded;
        self.gridlines = m.gridlines;
        self.show_name = m.show_name;
    }
}

/// Which part of the table a cell is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Header,
    HeaderColumn,
    Footer,
    /// A body row; `band` on every other one (the first body row banded).
    Body { band: bool },
}

// ----------------------------------------------------------------------
// Reading inputs

/// How a number was written (for Automatic formatting).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NumLook {
    pub currency: Option<String>,
    pub percent: bool,
    pub thousands: bool,
    pub decimals: Option<u8>,
}

/// A cell's value read for showing and sorting.
#[derive(Debug, Clone, PartialEq)]
pub enum Typed {
    Empty,
    Number(f64, NumLook),
    Bool(bool),
    /// An Excel date serial (days since 30 Dec 1899; the fraction is the time).
    Date(f64),
    Text(String),
    Error(String),
}

impl Typed {
    pub fn number(&self) -> Option<f64> {
        match self {
            Typed::Number(v, _) | Typed::Date(v) => Some(*v),
            Typed::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
}

const CURRENCIES: [&str; 6] = ["£", "$", "€", "¥", "₹", "CHF"];

/// Read a plain input: a number (1,240 · £980 · 13.7% · (5) · 1e6), a
/// date (20/11/2026 · 2026-11-20, with an optional hh:mm), TRUE/FALSE,
/// or text.
pub fn parse_input(s: &str) -> Typed {
    let t = s.trim();
    if t.is_empty() {
        return Typed::Empty;
    }
    if t.eq_ignore_ascii_case("true") {
        return Typed::Bool(true);
    }
    if t.eq_ignore_ascii_case("false") {
        return Typed::Bool(false);
    }
    if let Some(n) = parse_number(t) {
        return n;
    }
    if let Some(d) = parse_date(t) {
        return Typed::Date(d);
    }
    Typed::Text(s.to_string())
}

fn parse_number(t: &str) -> Option<Typed> {
    let mut s = t.replace('\u{2212}', "-");
    let mut neg = false;
    if s.starts_with('(') && s.ends_with(')') && s.len() > 2 {
        neg = true;
        s = s[1..s.len() - 1].trim().to_string();
    }
    if let Some(r) = s.strip_prefix('-') {
        neg = !neg;
        s = r.trim_start().to_string();
    } else if let Some(r) = s.strip_prefix('+') {
        s = r.trim_start().to_string();
    }
    let mut look = NumLook::default();
    for c in CURRENCIES {
        if let Some(r) = s.strip_prefix(c) {
            look.currency = Some(c.to_string());
            s = r.trim_start().to_string();
            break;
        }
        if let Some(r) = s.strip_suffix(c) {
            look.currency = Some(c.to_string());
            s = r.trim_end().to_string();
            break;
        }
    }
    // "£-5" as well as "-£5".
    if look.currency.is_some() {
        if let Some(r) = s.strip_prefix('-') {
            neg = !neg;
            s = r.to_string();
        }
    }
    if let Some(r) = s.strip_suffix('%') {
        look.percent = true;
        s = r.trim_end().to_string();
    }
    if s.is_empty() || look.currency.is_some() && look.percent {
        return None;
    }
    // Thousands separators: groups of three after the first.
    let (int, rest) = match s.find(['.', 'e', 'E']) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s.as_str(), ""),
    };
    if int.contains(',') {
        let groups: Vec<&str> = int.split(',').collect();
        let ok = !groups[0].is_empty() && groups[0].len() <= 3 && groups[1..].iter().all(|g| g.len() == 3);
        if !ok || !groups.iter().all(|g| g.bytes().all(|b| b.is_ascii_digit())) {
            return None;
        }
        look.thousands = true;
    }
    let digits: String = int.chars().filter(|c| *c != ',').collect();
    if digits.is_empty() && !rest.starts_with('.') {
        return None;
    }
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if let Some(frac) = rest.strip_prefix('.') {
        let d = frac.find(['e', 'E']).unwrap_or(frac.len());
        if !frac[..d].bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        look.decimals = Some(d.min(20) as u8);
    }
    let num: String = format!("{}{}", if digits.is_empty() { "0" } else { &digits }, rest);
    // A percentage is read with its point moved, so 13.7% is exactly
    // what 0.137 is.
    let shifted = look.percent && !num.contains(['e', 'E']);
    let mut v: f64 = if shifted { format!("{num}e-2").parse().ok()? } else { num.parse().ok()? };
    if look.percent && !shifted {
        v /= 100.0;
    }
    if !v.is_finite() {
        return None;
    }
    if neg {
        v = -v;
    }
    Some(Typed::Number(v, look))
}

/// Days from 1 Jan 1970 to y-m-d (proleptic Gregorian).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Excel's day 0 (30 Dec 1899) in days from 1970.
const EXCEL_EPOCH: i64 = -25569;

/// The Excel serial of a date.
pub fn date_serial(y: i64, m: u32, d: u32) -> f64 {
    (days_from_civil(y, m, d) - EXCEL_EPOCH) as f64
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

fn parse_date(t: &str) -> Option<f64> {
    let (date, time) = match t.split_once(' ') {
        Some((a, b)) if b.contains(':') => (a.trim(), Some(b.trim())),
        _ => (t, None),
    };
    let valid = |y: i64, m: u32, d: u32| (1..=12).contains(&m) && d >= 1 && d <= 31 && (1900..=9999).contains(&y) && civil_from_days(days_from_civil(y, m, d)) == (y, m, d);
    let mut ymd = None;
    if let [a, b, c] = date.split(['/', '-', '.']).collect::<Vec<_>>().as_slice() {
        let n = |s: &str| s.parse::<i64>().ok();
        if a.len() == 4 {
            // 2026-11-20
            if let (Some(y), Some(m), Some(d)) = (n(a), n(b), n(c)) {
                ymd = Some((y, m as u32, d as u32));
            }
        } else if let (Some(d), Some(y)) = (n(a), n(c)) {
            // 20/11/2026 (British order), 20/11/26; 20-Nov-2026
            let y = if c.len() == 2 { 2000 + y } else { y };
            let m = n(b).map(|m| m as u32).or_else(|| MONTHS.iter().position(|x| b.len() >= 3 && b.to_ascii_lowercase().starts_with(x)).map(|i| i as u32 + 1));
            if let Some(m) = m {
                ymd = Some((y, m, d as u32));
            }
        }
    } else if let [a, b, c] = date.split_whitespace().collect::<Vec<_>>().as_slice() {
        // 20 Nov 2026
        if let (Ok(d), Ok(y)) = (a.parse::<u32>(), c.parse::<i64>()) {
            if let Some(i) = MONTHS.iter().position(|x| b.len() >= 3 && b.to_ascii_lowercase().starts_with(x)) {
                ymd = Some((y, i as u32 + 1, d));
            }
        }
    }
    let (y, m, d) = ymd?;
    if !valid(y, m, d) {
        return None;
    }
    let mut v = date_serial(y, m, d);
    if let Some(tm) = time {
        let (h, mi) = tm.split_once(':')?;
        let (h, mi): (u32, u32) = (h.parse().ok()?, mi.get(..2).unwrap_or(mi).parse().ok()?);
        if h > 23 || mi > 59 {
            return None;
        }
        v += (h as f64 * 60.0 + mi as f64) / 1440.0;
    }
    Some(v)
}

// ----------------------------------------------------------------------
// Showing values

/// What a cell shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shown {
    pub text: String,
    /// A number (right-aligned unless set otherwise, `###` when too narrow).
    pub numeric: bool,
    /// Negative and its format shows negatives in red.
    pub red: bool,
    /// An error such as `#DIV/0!`.
    pub error: Option<String>,
    /// A tick box: ticked or not.
    pub tick: Option<bool>,
}

/// Group thousands: 1240000 → "1,240,000".
fn group(int: &str) -> String {
    let b = int.as_bytes();
    let mut out = String::with_capacity(b.len() + b.len() / 3);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `v` (not negative) with `decimals` digits (None: as few as show it, up
/// to 10 significant), thousands grouped or not.
fn digits(v: f64, decimals: Option<u8>, thousands: bool) -> String {
    let s = match decimals {
        Some(d) => format!("{:.*}", d as usize, v),
        None => general(v),
    };
    if s.contains('e') {
        return s;
    }
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s.clone(), None),
    };
    let int = if thousands { group(&int) } else { int };
    match frac {
        Some(f) if !f.is_empty() => format!("{int}.{f}"),
        _ => int,
    }
}

/// A number in up to 10 significant digits, no trailing zeros (Excel's
/// General).
fn general(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let mag = v.abs().log10().floor() as i32;
    if !(-9..15).contains(&mag) {
        let s = format!("{:.5e}", v);
        // 1.23450e7 → 1.2345E+07
        if let Some((m, e)) = s.split_once('e') {
            let m = m.trim_end_matches('0').trim_end_matches('.');
            let e: i32 = e.parse().unwrap_or(0);
            return format!("{m}E{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
        }
        return s;
    }
    let dec = (8 - mag).clamp(0, 10) as usize;
    let s = format!("{:.*}", dec, v);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// The date of an Excel serial as dd/mm/yyyy (with hh:mm when it has a time).
pub fn show_date(v: f64) -> String {
    let days = v.floor() as i64 + EXCEL_EPOCH;
    let (y, m, d) = civil_from_days(days);
    let mins = ((v - v.floor()) * 1440.0).round() as i64;
    if mins > 0 && mins < 1440 {
        format!("{d:02}/{m:02}/{y:04} {:02}:{:02}", mins / 60, mins % 60)
    } else {
        format!("{d:02}/{m:02}/{y:04}")
    }
}

/// Show `v` in format `f`.
pub fn format_number(v: f64, f: &DataFormat) -> Shown {
    let neg = v < 0.0;
    let a = v.abs();
    let body = match f.kind {
        CellType::Percentage => {
            let p = a * 100.0;
            let d = f.decimals.or_else(|| Some(auto_decimals(p, 2)));
            format!("{}%", digits(p, d, f.thousands))
        }
        CellType::Currency => {
            let d = f.decimals.or_else(|| Some(if a.fract().abs() < 1e-9 { 0 } else { 2 }));
            format!("{}{}", f.symbol(), digits(a, d, f.thousands))
        }
        CellType::DateTime => return Shown { text: show_date(v), numeric: true, ..Default::default() },
        _ => digits(a, f.decimals, f.thousands && f.kind != CellType::Automatic),
    };
    let text = match (neg && body.trim_start_matches(['0', '.', ',', '%']).chars().any(|c| c.is_ascii_digit() && c != '0'), f.negative) {
        (true, Negative::Parentheses | Negative::RedParentheses) => format!("({body})"),
        (true, _) => format!("\u{2212}{body}"),
        (false, _) => body,
    };
    Shown { text, numeric: true, red: neg && f.negative.red(), ..Default::default() }
}

/// Decimals needed to show `v` to `max` places, trailing zeros dropped.
fn auto_decimals(v: f64, max: u8) -> u8 {
    let s = format!("{:.*}", max as usize, v);
    match s.split_once('.') {
        Some((_, f)) => f.trim_end_matches('0').len() as u8,
        None => 0,
    }
}

/// The format a number written like `look` reads best in.
fn format_like(look: &NumLook) -> DataFormat {
    let kind = if look.percent {
        CellType::Percentage
    } else if look.currency.is_some() {
        CellType::Currency
    } else {
        CellType::Number
    };
    DataFormat { kind, decimals: look.decimals, thousands: look.thousands, currency: look.currency.clone().unwrap_or_default(), negative: Negative::Minus }
}

/// The format of the formula engine's number format ("dd/mm/yyyy",
/// "0.00%", "£#,##0"), as far as Zebra shows it.
fn engine_format(f: &str) -> Option<DataFormat> {
    let lower = f.to_ascii_lowercase();
    if lower == "general" || lower.is_empty() {
        return None;
    }
    let decimals = f.split_once('.').map(|(_, r)| r.chars().take_while(|c| *c == '0').count() as u8).or(Some(0));
    if lower.contains("yy") || lower.contains("dd") || lower.contains("mmm") || lower.contains("h:mm") {
        return Some(DataFormat::of(CellType::DateTime));
    }
    let thousands = f.contains(',');
    if f.contains('%') {
        return Some(DataFormat { kind: CellType::Percentage, decimals, thousands, ..Default::default() });
    }
    if let Some(c) = CURRENCIES.iter().find(|c| f.contains(**c)) {
        return Some(DataFormat { kind: CellType::Currency, decimals, thousands, currency: c.to_string(), ..Default::default() });
    }
    Some(DataFormat { kind: CellType::Number, decimals, thousands, ..Default::default() })
}

/// Shown when a cell is too narrow for its number.
pub const HASHES: &str = "###";

// ----------------------------------------------------------------------
// The table

/// Layout measures of a table, in local units, for one theme body size.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Body text size (theme body size × font_size).
    pub font: f64,
    pub pad_x: f64,
    pub pad_y: f64,
    /// Height of the name above the table (0 without one).
    pub caption: f64,
    /// Left edges of the columns from the table's left, and the right end.
    pub xs: Vec<f64>,
    /// Top edges of the rows from the table's top (after the caption),
    /// and the bottom end.
    pub ys: Vec<f64>,
}

impl Layout {
    pub fn width(&self) -> f64 {
        *self.xs.last().unwrap_or(&0.0)
    }
    pub fn height(&self) -> f64 {
        *self.ys.last().unwrap_or(&self.caption)
    }
    /// The row at `y` (from the table's top), clamped.
    pub fn row_at(&self, y: f64) -> usize {
        let n = self.ys.len().saturating_sub(1);
        if n == 0 {
            return 0;
        }
        self.ys.partition_point(|t| *t <= y).saturating_sub(1).min(n - 1)
    }
    pub fn col_at(&self, x: f64) -> usize {
        let n = self.xs.len().saturating_sub(1);
        if n == 0 {
            return 0;
        }
        self.xs.partition_point(|t| *t <= x).saturating_sub(1).min(n - 1)
    }
    /// The box of a range: x0, y0, x1, y1 from the table's top-left.
    pub fn range_box(&self, r: &CellRange) -> (f64, f64, f64, f64) {
        let c1 = (r.c1 + 1).min(self.xs.len() - 1);
        let r1 = (r.r1 + 1).min(self.ys.len() - 1);
        (self.xs[r.c0.min(c1)], self.ys[r.r0.min(r1)], self.xs[c1], self.ys[r1])
    }
}

/// Default sizes in body-text units.
pub const COLUMN_EMS: f64 = 8.5;
pub const ROW_EMS: f64 = 2.0;

impl TableData {
    /// A `rows` x `cols` table of empty cells for theme body text `body`
    /// points high.
    pub fn new(rows: usize, cols: usize, body: f64) -> TableData {
        let cols = cols.clamp(1, MAX_COLUMNS);
        let rows = rows.clamp(1, MAX_ROWS.min(MAX_CELLS / cols));
        TableData {
            cells: (0..rows).map(|_| CellRow::empty(cols)).collect(),
            columns: (0..cols).map(|_| Column::new((body * COLUMN_EMS).round())).collect(),
            rows: vec![Row::new((body * ROW_EMS).round()); rows],
            merges: vec![],
            header_rows: 1.min(rows as u8),
            header_columns: 0,
            footer_rows: 0,
            style: TableStyle::default(),
            cell_formats: vec![],
            font_size: 1.0,
            banded: None,
            gridlines: None,
            show_name: false,
        }
    }

    pub fn n_rows(&self) -> usize {
        self.cells.len()
    }
    pub fn n_cols(&self) -> usize {
        self.columns.len()
    }

    pub fn layout(&self, body: f64) -> Layout {
        let font = body * self.font_size;
        let caption = if self.show_name { (font * 1.7).round() } else { 0.0 };
        let mut xs = Vec::with_capacity(self.columns.len() + 1);
        let mut x = 0.0;
        xs.push(0.0);
        for c in &self.columns {
            x += c.width;
            xs.push(x);
        }
        let mut ys = Vec::with_capacity(self.rows.len() + 1);
        let mut y = caption;
        ys.push(y);
        for r in &self.rows {
            y += r.h();
            ys.push(y);
        }
        Layout { font, pad_x: font * 0.5, pad_y: font * 0.3, caption, xs, ys }
    }

    /// Width and height in local units.
    pub fn size(&self, body: f64) -> (f64, f64) {
        let l = self.layout(body);
        (l.width(), l.height())
    }

    pub fn role(&self, r: usize, c: usize) -> Role {
        let n = self.n_rows();
        if r < self.header_rows as usize {
            Role::Header
        } else if r >= n.saturating_sub(self.footer_rows as usize) {
            Role::Footer
        } else if c < self.header_columns as usize {
            Role::HeaderColumn
        } else {
            Role::Body { band: (r - self.header_rows as usize) % 2 == 1 }
        }
    }

    /// Body rows: what sorting rearranges.
    pub fn body_rows(&self) -> std::ops::Range<usize> {
        let n = self.n_rows();
        let h = (self.header_rows as usize).min(n);
        h..n.saturating_sub(self.footer_rows as usize).max(h)
    }

    /// The merged range a cell is in.
    pub fn merge_at(&self, r: usize, c: usize) -> Option<&CellRange> {
        self.merges.iter().find(|m| m.contains(r, c))
    }

    /// The cell that holds the content where (r, c) is: the top-left of
    /// its merge, else itself.
    pub fn anchor(&self, r: usize, c: usize) -> (usize, usize) {
        self.merge_at(r, c).map(|m| (m.r0, m.c0)).unwrap_or((r, c))
    }

    /// The range a cell takes on screen: its merge, else itself.
    pub fn span(&self, r: usize, c: usize) -> CellRange {
        self.merge_at(r, c).copied().unwrap_or(CellRange::cell(r, c))
    }

    /// `range` grown to take in every merge it touches.
    pub fn expand_to_merges(&self, range: CellRange) -> CellRange {
        let mut r = range;
        loop {
            let before = r;
            for m in &self.merges {
                if m.intersects(&r) {
                    r = r.union(m);
                }
            }
            if r == before {
                return r;
            }
        }
    }

    pub fn cell(&self, r: usize, c: usize) -> Option<&Cell> {
        self.cells.get(r)?.get(c)
    }

    /// Every format set on (r, c), in order, folded into one.
    pub fn format_at(&self, r: usize, c: usize) -> CellFormat {
        let mut f = CellFormat::default();
        for rf in &self.cell_formats {
            if rf.range.contains(r, c) {
                f.overlay(&rf.format);
            }
        }
        f
    }

    /// The Data Format of (r, c): its own, else its column's (header rows
    /// take none from the column: they show what was typed).
    pub fn data_format(&self, r: usize, c: usize) -> DataFormat {
        if let Some(n) = self.format_at(r, c).number {
            return n;
        }
        if r < self.header_rows as usize {
            return DataFormat::default();
        }
        self.columns.get(c).map(|c| c.format.clone()).unwrap_or_default()
    }

    /// How the body's plain numbers in column `c` are mostly written:
    /// what an Automatic formula result in that column shows like (a total
    /// under amounts in pounds shows in pounds).
    pub fn column_look(&self, c: usize) -> Option<NumLook> {
        let mut looks: HashMap<(Option<String>, bool), (usize, NumLook)> = HashMap::new();
        for r in self.header_rows as usize..self.n_rows() {
            let Some(cell) = self.cell(r, c) else { continue };
            if cell.is_formula() {
                continue;
            }
            if let Typed::Number(_, l) = parse_input(&cell.input) {
                let e = looks.entry((l.currency.clone(), l.percent)).or_insert((0, NumLook::default()));
                e.0 += 1;
                e.1.currency = l.currency.clone();
                e.1.percent = l.percent;
                e.1.thousands |= l.thousands;
                e.1.decimals = e.1.decimals.max(l.decimals);
            }
        }
        looks.into_values().max_by_key(|(n, _)| *n).map(|(_, l)| l)
    }

    /// What (r, c) shows. `look`: [`TableData::column_look`] of its
    /// column, worked out once by the caller.
    pub fn shown(&self, r: usize, c: usize, look: Option<&NumLook>) -> Shown {
        let Some(cell) = self.cell(r, c) else { return Shown::default() };
        let f = self.data_format(r, c);
        let typed = cell.typed();
        if f.kind == CellType::TickBox {
            let on = match &typed {
                Typed::Bool(b) => *b,
                Typed::Number(v, _) => *v != 0.0,
                Typed::Text(t) => matches!(t.trim().to_ascii_lowercase().as_str(), "yes" | "y" | "x" | "✓" | "✔"),
                _ => false,
            };
            return Shown { tick: Some(on), ..Default::default() };
        }
        match typed {
            Typed::Empty => Shown::default(),
            Typed::Error(e) => Shown { text: e.clone(), error: Some(e), ..Default::default() },
            Typed::Text(t) => Shown { text: t, ..Default::default() },
            Typed::Bool(b) => Shown { text: (if b { "TRUE" } else { "FALSE" }).into(), ..Default::default() },
            Typed::Date(v) => match f.kind {
                CellType::Automatic | CellType::DateTime => Shown { text: if cell.is_formula() { show_date(v) } else { cell.input.trim().to_string() }, numeric: true, ..Default::default() },
                CellType::Text => Shown { text: cell.input.clone(), ..Default::default() },
                _ => format_number(v, &f),
            },
            Typed::Number(v, own) => match f.kind {
                CellType::Text => Shown { text: if cell.is_formula() { general(v) } else { cell.input.clone() }, ..Default::default() },
                CellType::Automatic if !cell.is_formula() => {
                    // As typed (a minus shown as a minus).
                    let t = cell.input.trim();
                    let text = if t.starts_with('-') { format!("\u{2212}{}", &t[1..]) } else { t.to_string() };
                    Shown { text, numeric: true, red: false, ..Default::default() }
                }
                CellType::Automatic => {
                    let auto = cell.format.as_deref().and_then(engine_format).or_else(|| look.map(format_like)).unwrap_or_else(|| format_like(&own));
                    let mut s = format_number(v, &auto);
                    if auto.kind == CellType::Number && look.is_none() && cell.format.is_none() {
                        s.text = format_number(v, &DataFormat { thousands: false, ..auto }).text;
                    }
                    s
                }
                _ => format_number(v, &f),
            },
        }
    }

    /// Whether (r, c) is drawn bold: header rows and columns and footers
    /// are, unless a format says otherwise.
    pub fn bold_at(&self, r: usize, c: usize) -> bool {
        self.format_at(r, c).bold.unwrap_or(!matches!(self.role(r, c), Role::Body { .. }))
    }

    /// Horizontal alignment of (r, c): its own, else numbers right and
    /// text left; a header cell lines up with the column's first body
    /// value under it.
    pub fn align_at(&self, r: usize, c: usize, shown: &Shown) -> Align {
        if let Some(a) = self.format_at(r, c).align {
            return a;
        }
        if shown.tick.is_some() {
            return Align::Center;
        }
        if r < self.header_rows as usize {
            let under = self.body_rows().find(|&b| self.cell(b, c).is_some_and(|x| !x.is_empty()));
            if let Some(b) = under {
                let s = self.shown(b, c, None);
                if s.numeric || s.error.is_some() {
                    return Align::Right;
                }
            }
            return Align::Left;
        }
        if shown.numeric || shown.error.is_some() { Align::Right } else { Align::Left }
    }

    // ------------------------------------------------------------------
    // Structural edits. Formula references are rewritten by the caller
    // (see `crate::calc`); these move cells, sizes, merges and formats.

    /// Insert `n` empty rows before row `at` (`at` = the row count adds at
    /// the end). A row inserted among the header or footer rows joins them.
    pub fn insert_rows(&mut self, at: usize, n: usize, height: Option<f64>) {
        let at = at.min(self.n_rows());
        let cols = self.n_cols();
        let h = height.unwrap_or_else(|| self.rows.get(at.min(self.rows.len().saturating_sub(1))).map(|r| r.height).unwrap_or(48.0));
        let rows_before = self.n_rows();
        for _ in 0..n {
            self.cells.insert(at, CellRow::empty(cols));
            self.rows.insert(at, Row::new(h));
        }
        let hdr = self.header_rows as usize;
        if at < hdr {
            self.header_rows = (hdr + n).min(MAX_HEADERS as usize) as u8;
        }
        let foot_start = rows_before - (self.footer_rows as usize).min(rows_before);
        if self.footer_rows > 0 && at > foot_start && at < rows_before {
            self.footer_rows = (self.footer_rows as usize + n).min(MAX_HEADERS as usize) as u8;
        }
        self.shift_ranges(|r| r, |rr, axis_rows| if axis_rows { shift_insert(rr, at, n) } else { Some(rr) }, true);
    }

    /// Delete `n` rows from `at`.
    pub fn delete_rows(&mut self, at: usize, n: usize) {
        let total = self.n_rows();
        if at >= total || n == 0 {
            return;
        }
        let n = n.min(total - at);
        let hdr = self.header_rows as usize;
        let in_hdr = (at..at + n).filter(|r| *r < hdr).count();
        let foot_start = total - (self.footer_rows as usize).min(total);
        let in_foot = (at..at + n).filter(|r| *r >= foot_start).count();
        self.cells.drain(at..at + n);
        self.rows.drain(at..at + n);
        self.header_rows -= in_hdr as u8;
        self.footer_rows -= in_foot as u8;
        self.shift_ranges(|r| r, |rr, axis_rows| if axis_rows { shift_delete(rr, at, n) } else { Some(rr) }, true);
    }

    pub fn insert_columns(&mut self, at: usize, n: usize, width: Option<f64>) {
        let at = at.min(self.n_cols());
        let w = width.unwrap_or_else(|| self.columns.get(at.min(self.columns.len().saturating_sub(1))).map(|c| c.width).unwrap_or(200.0));
        for row in &mut self.cells {
            for _ in 0..n {
                row.insert(at, Cell::default());
            }
        }
        for _ in 0..n {
            self.columns.insert(at, Column::new(w));
        }
        if at < self.header_columns as usize {
            self.header_columns = (self.header_columns as usize + n).min(MAX_HEADERS as usize) as u8;
        }
        self.shift_ranges(|r| r, |rr, axis_rows| if axis_rows { Some(rr) } else { shift_insert(rr, at, n) }, false);
    }

    pub fn delete_columns(&mut self, at: usize, n: usize) {
        let total = self.n_cols();
        if at >= total || n == 0 {
            return;
        }
        let n = n.min(total - at);
        for row in &mut self.cells {
            row.drain(at..at + n);
        }
        self.columns.drain(at..at + n);
        let in_hdr = (at..at + n).filter(|c| *c < self.header_columns as usize).count();
        self.header_columns -= in_hdr as u8;
        self.shift_ranges(|r| r, |rr, axis_rows| if axis_rows { Some(rr) } else { shift_delete(rr, at, n) }, false);
    }

    /// Move merges and formats along one axis. `f(range_on_axis, is_rows)`
    /// gives the new (first, last) on that axis, or None when it's gone.
    fn shift_ranges(&mut self, _id: impl Fn(usize) -> usize, f: impl Fn((usize, usize), bool) -> Option<(usize, usize)>, rows: bool) {
        let apply = |r: &CellRange| -> Option<CellRange> {
            if rows {
                f((r.r0, r.r1), true).map(|(a, b)| CellRange { r0: a, r1: b, ..*r })
            } else {
                f((r.c0, r.c1), false).map(|(a, b)| CellRange { c0: a, c1: b, ..*r })
            }
        };
        self.merges = self.merges.iter().filter_map(apply).filter(|m| !m.is_single()).collect();
        self.cell_formats = self.cell_formats.iter().filter_map(|rf| apply(&rf.range).map(|range| RangeFormat { range, format: rf.format.clone() })).collect();
    }

    /// Rearrange rows: row `i` of the result is old row `order[i]`.
    /// `order` is a permutation of all rows. Merges must stay whole (each
    /// merged block's rows end up next to each other, in order); formats
    /// follow their cells. Formula text is the caller's.
    pub fn permute_rows(&mut self, order: &[usize]) -> Result<(), String> {
        let n = self.n_rows();
        if order.len() != n {
            return Err("order must list every row once".into());
        }
        let mut seen = vec![false; n];
        for &o in order {
            if o >= n || std::mem::replace(&mut seen[o], true) {
                return Err("order must list every row once".into());
            }
        }
        let mut new_of = vec![0usize; n];
        for (i, &o) in order.iter().enumerate() {
            new_of[o] = i;
        }
        // Merges move whole or not at all.
        let mut merges = vec![];
        for m in &self.merges {
            let first = new_of[m.r0];
            if (m.r0..=m.r1).enumerate().any(|(k, r)| new_of[r] != first + k) {
                return Err("merged cells would be split".into());
            }
            merges.push(CellRange { r0: first, r1: first + m.rows() - 1, ..*m });
        }
        // Formats: a range is split into rows that move, unless the rows it
        // covers stay where they are.
        let mut formats = vec![];
        for rf in &self.cell_formats {
            let r = rf.range;
            let rr1 = r.r1.min(n.saturating_sub(1));
            if (r.r0..=rr1).all(|x| new_of[x] == x) {
                formats.push(rf.clone());
                continue;
            }
            for x in r.r0..=rr1 {
                formats.push(RangeFormat { range: CellRange { r0: new_of[x], r1: new_of[x], ..r }, format: rf.format.clone() });
            }
        }
        let old_cells = std::mem::take(&mut self.cells);
        let old_rows = std::mem::take(&mut self.rows);
        let cols = self.columns.len();
        let mut cells: Vec<Option<CellRow>> = old_cells.into_iter().map(Some).collect();
        let mut rows: Vec<Option<Row>> = old_rows.into_iter().map(Some).collect();
        for &o in order {
            self.cells.push(cells[o].take().unwrap_or_else(|| CellRow::empty(cols)));
            self.rows.push(rows[o].take().unwrap_or(Row::new(48.0)));
        }
        self.merges = merges;
        self.cell_formats = formats;
        self.compact_formats();
        Ok(())
    }

    /// Rearrange columns: column `i` of the result is old column `order[i]`.
    pub fn permute_columns(&mut self, order: &[usize]) -> Result<(), String> {
        let n = self.n_cols();
        if order.len() != n {
            return Err("order must list every column once".into());
        }
        let mut new_of = vec![usize::MAX; n];
        for (i, &o) in order.iter().enumerate() {
            if o >= n || new_of[o] != usize::MAX {
                return Err("order must list every column once".into());
            }
            new_of[o] = i;
        }
        let mut merges = vec![];
        for m in &self.merges {
            let first = new_of[m.c0];
            if (m.c0..=m.c1).enumerate().any(|(k, c)| new_of[c] != first + k) {
                return Err("merged cells would be split".into());
            }
            merges.push(CellRange { c0: first, c1: first + m.cols() - 1, ..*m });
        }
        let mut formats = vec![];
        for rf in &self.cell_formats {
            let r = rf.range;
            let cc1 = r.c1.min(n.saturating_sub(1));
            if (r.c0..=cc1).all(|x| new_of[x] == x) {
                formats.push(rf.clone());
                continue;
            }
            for x in r.c0..=cc1 {
                formats.push(RangeFormat { range: CellRange { c0: new_of[x], c1: new_of[x], ..r }, format: rf.format.clone() });
            }
        }
        for row in &mut self.cells {
            let moved: Vec<Cell> = order.iter().map(|&o| row.get(o).cloned().unwrap_or_default()).collect();
            row.set(moved);
        }
        let mut cols: Vec<Option<Column>> = std::mem::take(&mut self.columns).into_iter().map(Some).collect();
        self.columns = order.iter().map(|&o| cols[o].take().unwrap_or_else(|| Column::new(200.0))).collect();
        self.merges = merges;
        self.cell_formats = formats;
        self.compact_formats();
        Ok(())
    }

    /// The order that moves `count` rows (or columns) from `from` to sit
    /// before `to` (indices of the original order; `to` = n puts them last).
    pub fn move_order(n: usize, from: usize, count: usize, to: usize) -> Vec<usize> {
        let from = from.min(n);
        let count = count.min(n - from);
        let block: Vec<usize> = (from..from + count).collect();
        let rest: Vec<usize> = (0..n).filter(|i| *i < from || *i >= from + count).collect();
        // Where `to` lands among the rest.
        let pos = rest.iter().position(|&i| i >= to).unwrap_or(rest.len());
        let mut out = rest[..pos].to_vec();
        out.extend(block);
        out.extend_from_slice(&rest[pos..]);
        out
    }

    /// The body order sorted by column `c` (header and footer rows stay):
    /// a stable sort by the column's type. Empty cells go last either way.
    pub fn sort_order(&self, c: usize, descending: bool) -> Vec<usize> {
        let body = self.body_rows();
        let kind = self.columns.get(c).map(|c| c.format.kind).unwrap_or_default();
        let mut keys: Vec<(usize, SortKey)> = body.clone().map(|r| (r, sort_key(self.cell(r, c).map(|x| x.typed()).unwrap_or(Typed::Empty), kind))).collect();
        keys.sort_by(|(_, a), (_, b)| {
            use std::cmp::Ordering::*;
            match (a, b) {
                (SortKey::Empty, SortKey::Empty) => Equal,
                (SortKey::Empty, _) => Greater,
                (_, SortKey::Empty) => Less,
                _ => {
                    let o = a.cmp_with(b);
                    if descending { o.reverse() } else { o }
                }
            }
        });
        let mut order: Vec<usize> = (0..body.start).collect();
        order.extend(keys.into_iter().map(|(r, _)| r));
        order.extend(body.end..self.n_rows());
        order
    }

    /// Merge `range` (grown to whole merges it touches). The content of the
    /// cells is kept: the top-left one's when it's the only one with
    /// anything, else every non-empty cell's text, a line each.
    pub fn merge(&mut self, range: CellRange) {
        let range = self.expand_to_merges(range);
        if range.is_single() {
            return;
        }
        let mut texts = vec![];
        for r in range.r0..=range.r1 {
            for c in range.c0..=range.c1 {
                if let Some(cell) = self.cell(r, c) {
                    if !cell.is_empty() {
                        texts.push((r, c));
                    }
                }
            }
        }
        if !(texts.len() <= 1 && texts.first().is_none_or(|&(r, c)| (r, c) == (range.r0, range.c0))) {
            let joined: Vec<String> = texts.iter().map(|&(r, c)| self.shown(r, c, None).text).collect();
            self.cells[range.r0][range.c0] = Cell::new(joined.join("\n"));
        }
        for r in range.r0..=range.r1 {
            for c in range.c0..=range.c1 {
                if (r, c) != (range.r0, range.c0) {
                    self.cells[r][c] = Cell::default();
                }
            }
        }
        self.merges.retain(|m| !range.covers(m));
        self.merges.push(range);
    }

    /// Unmerge every merge that touches `range`.
    pub fn unmerge(&mut self, range: CellRange) -> bool {
        let before = self.merges.len();
        self.merges.retain(|m| !m.intersects(&range));
        before != self.merges.len()
    }

    /// Add a format on `range` and tidy the list.
    pub fn add_format(&mut self, range: CellRange, format: CellFormat) {
        if format.is_empty() {
            return;
        }
        self.cell_formats.push(RangeFormat { range, format });
        self.compact_formats();
    }

    /// Drop format fields a later range sets over the whole of an earlier
    /// one, and formats left empty, so formatting the same cells again and
    /// again doesn't make the list grow.
    pub fn compact_formats(&mut self) {
        let n = self.cell_formats.len();
        for i in 0..n {
            for j in i + 1..n {
                if self.cell_formats[j].range.covers(&self.cell_formats[i].range) {
                    let later = self.cell_formats[j].format.clone();
                    self.cell_formats[i].format.without(&later);
                    if later.border.is_some() && self.cell_formats[i].format.border.as_ref().is_some_and(|b| Some(b.edges) == later.border.as_ref().map(|l| l.edges)) && self.cell_formats[j].range == self.cell_formats[i].range {
                        self.cell_formats[i].format.border = None;
                    }
                }
            }
        }
        self.cell_formats.retain(|f| !f.format.is_empty());
    }

    /// Stretch to `w` x `h` local units (either may be left as it is):
    /// columns keep their proportions, rows their smallest heights in
    /// proportion. Rows never get shorter than their text.
    pub fn resize(&mut self, body: f64, w: Option<f64>, h: Option<f64>) {
        let l = self.layout(body);
        if let Some(w) = w.filter(|w| w.is_finite() && *w > 0.0) {
            let k = w / l.width().max(1e-9);
            for c in &mut self.columns {
                c.width = (c.width * k).max(1.0);
            }
        }
        if let Some(h) = h.filter(|h| h.is_finite() && *h > 0.0) {
            let now = l.height() - l.caption;
            let k = (h - l.caption).max(1.0) / now.max(1e-9);
            for r in &mut self.rows {
                r.height = (r.h() * k).max(1.0);
                r.fit = 0.0;
            }
        }
    }

    /// Grow to at least `rows` x `cols`.
    pub fn grow_to(&mut self, rows: usize, cols: usize) {
        let rows = rows.min(MAX_ROWS);
        let cols = cols.min(MAX_COLUMNS);
        if cols > self.n_cols() {
            let n = self.n_cols();
            self.insert_columns(n, cols - n, None);
        }
        if rows > self.n_rows() {
            // New rows go after the body, before any footer, as typing on
            // in the last row does.
            let at = self.n_rows() - (self.footer_rows as usize).min(self.n_rows());
            let n = rows - self.n_rows();
            self.insert_rows(at, n, None);
        }
    }

    /// The inputs of a range as rows of text.
    pub fn inputs(&self, range: &CellRange) -> Vec<Vec<String>> {
        (range.r0..=range.r1).map(|r| (range.c0..=range.c1).map(|c| self.cell(r, c).map(|x| x.input.clone()).unwrap_or_default()).collect()).collect()
    }

    /// What a range shows, as rows of text (for copying out and for agents).
    pub fn shown_rows(&self, range: &CellRange) -> Vec<Vec<String>> {
        let looks: Vec<Option<NumLook>> = (0..self.n_cols()).map(|c| self.column_look(c)).collect();
        (range.r0..=range.r1)
            .map(|r| {
                (range.c0..=range.c1)
                    .map(|c| {
                        let s = self.shown(r, c, looks.get(c).and_then(|l| l.as_ref()));
                        match s.tick {
                            Some(t) => (if t { "TRUE" } else { "FALSE" }).to_string(),
                            None => s.text,
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// Every cell in range order (row by row).
    pub fn whole(&self) -> CellRange {
        CellRange::new(0, 0, self.n_rows().saturating_sub(1), self.n_cols().saturating_sub(1))
    }

    /// What's wrong with the table, if anything: not rectangular, counts
    /// out of range, unusable sizes, merges outside it or overlapping.
    pub fn check(&self) -> Result<(), String> {
        let (rows, cols) = (self.n_rows(), self.n_cols());
        if !fits_limits(rows, cols) {
            return Err(format!("a table has 1 to {MAX_ROWS} rows, 1 to {MAX_COLUMNS} columns and at most {MAX_CELLS} cells"));
        }
        if self.rows.len() != rows || self.cells.iter().any(|r| r.len() != cols) {
            return Err("every row needs one cell per column, and rows one height each".into());
        }
        let ok = |v: f64| v.is_finite() && v > 0.0 && v <= 1e9;
        if self.columns.iter().any(|c| !ok(c.width)) || self.rows.iter().any(|r| !ok(r.height) || !(r.fit.is_finite() && r.fit >= 0.0 && r.fit <= 1e9)) {
            return Err("column widths and row heights must be > 0".into());
        }
        if !(self.font_size.is_finite() && self.font_size > 0.01 && self.font_size <= 100.0) {
            return Err("font_size must be > 0".into());
        }
        if self.header_rows > MAX_HEADERS || self.footer_rows > MAX_HEADERS || self.header_columns > MAX_HEADERS || self.header_rows as usize + self.footer_rows as usize > rows || self.header_columns as usize > cols {
            return Err(format!("header rows, footer rows and header columns are 0 to {MAX_HEADERS} and fit in the table"));
        }
        for (i, m) in self.merges.iter().enumerate() {
            if m.r1 >= rows || m.c1 >= cols || m.is_single() {
                return Err(format!("merged cells {} are not inside the table", m.a1()));
            }
            if self.merges[..i].iter().any(|o| o.intersects(m)) {
                return Err(format!("merged cells {} overlap others", m.a1()));
            }
        }
        if self.cell_formats.iter().any(|f| f.range.r0 >= rows || f.range.c0 >= cols) {
            return Err("a cell format is outside the table".into());
        }
        Ok(())
    }

    /// The table made rectangular and every count back in range (a file
    /// from elsewhere). Returns whether anything changed.
    pub fn repair(&mut self, body: f64) -> bool {
        let before = self.clone();
        if self.cells.is_empty() {
            self.cells.push(CellRow::new(vec![]));
        }
        self.cells.truncate(MAX_ROWS);
        let cols = self.cells.iter().map(|r| r.len()).max().unwrap_or(0).max(self.columns.len()).clamp(1, MAX_COLUMNS);
        // Over the cell limit: the rows past it go.
        self.cells.truncate((MAX_CELLS / cols).max(1));
        for row in &mut self.cells {
            if row.len() != cols {
                row.resize(cols, Cell::default());
            }
            if row.iter().any(|c| input_problem(&c.input).is_some()) {
                for c in row.iter_mut() {
                    if input_problem(&c.input).is_some() {
                        let cap = if c.is_formula() { MAX_FORMULA } else { MAX_INPUT };
                        c.input = c.input.chars().take(cap).collect();
                    }
                }
            }
        }
        let col_w = (body * COLUMN_EMS).round();
        self.columns.truncate(cols);
        while self.columns.len() < cols {
            self.columns.push(Column::new(col_w));
        }
        for c in &mut self.columns {
            if !(c.width.is_finite() && c.width > 0.0 && c.width <= 1e9) {
                c.width = col_w;
            }
        }
        let rows = self.cells.len();
        let row_h = (body * ROW_EMS).round();
        self.rows.truncate(rows);
        while self.rows.len() < rows {
            self.rows.push(Row::new(row_h));
        }
        for r in &mut self.rows {
            if !(r.height.is_finite() && r.height > 0.0 && r.height <= 1e9) {
                r.height = row_h;
            }
            if !(r.fit.is_finite() && r.fit >= 0.0 && r.fit <= 1e9) {
                r.fit = 0.0;
            }
        }
        if !(self.font_size.is_finite() && self.font_size > 0.01 && self.font_size <= 100.0) {
            self.font_size = 1.0;
        }
        self.header_rows = self.header_rows.min(MAX_HEADERS).min(rows as u8);
        self.footer_rows = self.footer_rows.min(MAX_HEADERS).min((rows - self.header_rows as usize) as u8);
        self.header_columns = self.header_columns.min(MAX_HEADERS).min(cols as u8);
        // Merges inside the table, not overlapping, more than one cell.
        let mut kept: Vec<CellRange> = vec![];
        for m in std::mem::take(&mut self.merges) {
            if let Some(m) = m.clip(rows, cols) {
                if !m.is_single() && !kept.iter().any(|k| k.intersects(&m)) {
                    kept.push(m);
                }
            }
        }
        self.merges = kept;
        self.cell_formats.retain_mut(|rf| match rf.range.clip(rows, cols) {
            Some(r) => {
                rf.range = r;
                true
            }
            None => false,
        });
        *self != before
    }
}

fn shift_insert((a, b): (usize, usize), at: usize, n: usize) -> Option<(usize, usize)> {
    // A range grows when rows go in inside it (not above its first).
    let a2 = if a >= at { a + n } else { a };
    let b2 = if b >= at { b + n } else { b };
    Some((a2, b2))
}

fn shift_delete((a, b): (usize, usize), at: usize, n: usize) -> Option<(usize, usize)> {
    let end = at + n;
    let keep = |x: usize| x < at || x >= end;
    let map = |x: usize| if x < at { x } else { x - n };
    // The first and last that survive.
    let first = (a..=b).find(|x| keep(*x))?;
    let last = (a..=b).rev().find(|x| keep(*x))?;
    Some((map(first), map(last)))
}

#[derive(Debug, Clone, PartialEq)]
enum SortKey {
    Empty,
    Num(f64),
    Text(String),
    /// Text in a numeric column: after its numbers.
    Other(String),
}

impl SortKey {
    fn cmp_with(&self, o: &SortKey) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        match (self, o) {
            (SortKey::Num(a), SortKey::Num(b)) => a.total_cmp(b),
            (SortKey::Text(a), SortKey::Text(b)) | (SortKey::Other(a), SortKey::Other(b)) => natural_cmp(a, b),
            // Numbers before text, as Numbers sorts a mixed column.
            (SortKey::Num(_), _) => Less,
            (_, SortKey::Num(_)) => Greater,
            (SortKey::Text(_), SortKey::Other(_)) => Less,
            (SortKey::Other(_), SortKey::Text(_)) => Greater,
            _ => Equal,
        }
    }
}

fn sort_key(t: Typed, kind: CellType) -> SortKey {
    match (kind, t) {
        (_, Typed::Empty) => SortKey::Empty,
        (CellType::Text, t) => SortKey::Text(typed_text(&t).to_lowercase()),
        (CellType::TickBox, Typed::Bool(b)) => SortKey::Num(if b { 1.0 } else { 0.0 }),
        (CellType::TickBox, Typed::Text(t)) => SortKey::Num(if matches!(t.trim().to_ascii_lowercase().as_str(), "yes" | "y" | "x" | "✓" | "✔") { 1.0 } else { 0.0 }),
        (_, Typed::Error(e)) => SortKey::Other(e),
        (CellType::Automatic, Typed::Text(t)) => SortKey::Text(t.to_lowercase()),
        (_, Typed::Text(t)) => SortKey::Other(t.to_lowercase()),
        (_, t) => match t.number() {
            Some(v) => SortKey::Num(v),
            None => SortKey::Other(typed_text(&t).to_lowercase()),
        },
    }
}

fn typed_text(t: &Typed) -> String {
    match t {
        Typed::Empty => String::new(),
        Typed::Number(v, _) | Typed::Date(v) => general(*v),
        Typed::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).into(),
        Typed::Text(s) | Typed::Error(s) => s.clone(),
    }
}

/// Text order with runs of digits compared as numbers ("Item 2" before
/// "Item 10"), as Finder and Numbers sort.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = x.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    x.next();
                }
                let mut nb = String::new();
                while let Some(c) = y.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    y.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(p), Some(q)) => {
                let o = p.cmp(&q);
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

// ----------------------------------------------------------------------
// Tab- and comma-separated text

/// A limit that input went over. The app words each one (ftl); the API
/// says [`Limit::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Rows,
    Columns,
    Cells,
    /// A cell's input over [`MAX_INPUT`] characters.
    Input,
    /// A formula over [`MAX_FORMULA`] characters.
    Formula,
    /// Pasted or imported text over [`MAX_PASTE_BYTES`].
    Bytes,
}

impl Limit {
    pub fn message(self) -> String {
        match self {
            Limit::Rows => format!("a table has at most {MAX_ROWS} rows"),
            Limit::Columns => format!("a table has at most {MAX_COLUMNS} columns"),
            Limit::Cells => format!("a table has at most {MAX_CELLS} cells"),
            Limit::Input => format!("a cell holds at most {MAX_INPUT} characters"),
            Limit::Formula => format!("a formula is at most {MAX_FORMULA} characters"),
            Limit::Bytes => format!("text for a table is at most {} MB", MAX_PASTE_BYTES >> 20),
        }
    }
    /// The limit `rows` x `cols` goes over, if any.
    pub fn of_size(rows: usize, cols: usize) -> Option<Limit> {
        if rows > MAX_ROWS {
            Some(Limit::Rows)
        } else if cols > MAX_COLUMNS {
            Some(Limit::Columns)
        } else if rows.saturating_mul(cols) > MAX_CELLS {
            Some(Limit::Cells)
        } else {
            None
        }
    }
    /// The limit an input goes over, if any.
    pub fn of_input(s: &str) -> Option<Limit> {
        input_problem(s).map(|p| match p {
            InputProblem::TooLong => Limit::Input,
            InputProblem::FormulaTooLong => Limit::Formula,
        })
    }
}

/// Most text read as a table (a paste, `add_table`'s csv): about what
/// 100,000 short cells take, with room to spare.
pub const MAX_PASTE_BYTES: usize = 16 << 20;

/// Rows of fields from tab-separated text (what Numbers and Excel copy),
/// or comma-separated when there is no tab. Quoted fields may hold the
/// separator, quotes ("") and line breaks. Short rows are padded.
///
/// Bounded: text over [`MAX_PASTE_BYTES`] is refused before it is read,
/// and reading stops at the first row, column, cell or field over the
/// limits, so a huge paste never becomes a huge grid.
pub fn parse_delimited(text: &str) -> Result<Vec<Vec<String>>, Limit> {
    if text.len() > MAX_PASTE_BYTES {
        return Err(Limit::Bytes);
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let sep = if text.contains('\t') { '\t' } else { ',' };
    let mut rows: Vec<Vec<String>> = vec![];
    let mut row: Vec<String> = vec![];
    let mut field = String::new();
    let mut field_chars = 0usize;
    let mut cells = 0usize;
    let mut quoted = false;
    let mut at_start = true;
    let mut it = text.chars().peekable();
    // Ends a field: counted against the limits.
    let mut end_field = |row: &mut Vec<String>, field: &mut String, chars: &mut usize| -> Result<(), Limit> {
        if let Some(l) = Limit::of_input(field) {
            return Err(l);
        }
        row.push(std::mem::take(field));
        *chars = 0;
        cells += 1;
        if row.len() > MAX_COLUMNS {
            return Err(Limit::Columns);
        }
        if cells > MAX_CELLS {
            return Err(Limit::Cells);
        }
        Ok(())
    };
    while let Some(ch) = it.next() {
        if field_chars > MAX_INPUT {
            return Err(if is_formula(&field) { Limit::Formula } else { Limit::Input });
        }
        if quoted {
            if ch == '"' {
                if it.peek() == Some(&'"') {
                    field.push('"');
                    field_chars += 1;
                    it.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
                field_chars += 1;
            }
            continue;
        }
        match ch {
            '"' if at_start => {
                quoted = true;
                at_start = false;
            }
            c if c == sep => {
                end_field(&mut row, &mut field, &mut field_chars)?;
                at_start = true;
            }
            '\r' => {}
            '\n' => {
                end_field(&mut row, &mut field, &mut field_chars)?;
                rows.push(std::mem::take(&mut row));
                if rows.len() > MAX_ROWS {
                    return Err(Limit::Rows);
                }
                at_start = true;
            }
            c => {
                field.push(c);
                field_chars += 1;
                at_start = false;
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        end_field(&mut row, &mut field, &mut field_chars)?;
        rows.push(row);
        if rows.len() > MAX_ROWS {
            return Err(Limit::Rows);
        }
    }
    let w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    // Ragged rows are padded: the padded grid must fit too.
    if let Some(l) = Limit::of_size(rows.len(), w) {
        return Err(l);
    }
    for r in &mut rows {
        r.resize(w, String::new());
    }
    Ok(rows)
}

/// Rows of fields as tab-separated text, quoting fields that need it.
pub fn to_tsv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for (i, r) in rows.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for (j, f) in r.iter().enumerate() {
            if j > 0 {
                out.push('\t');
            }
            if f.contains(['\t', '\n', '"']) {
                out.push('"');
                out.push_str(&f.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(f);
            }
        }
    }
    out
}

/// Whether pasted text reads as a table: at least two fields, from text
/// with a tab in it.
pub fn looks_tabular(text: &str) -> bool {
    text.contains('\t') && {
        // Only the first lines matter: a tab and two fields.
        let head: String = text.lines().take(2).collect::<Vec<_>>().join("\n");
        let rows = parse_delimited(&head).unwrap_or_default();
        rows.len() * rows.first().map_or(0, |r| r.len()) >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a1_names_round_trip() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
        for c in [0, 1, 25, 26, 27, 51, 52, 701, 702, 1000] {
            assert_eq!(parse_column(&column_name(c)), Some(c));
        }
        assert_eq!(parse_cell("B2"), Some((1, 1)));
        assert_eq!(parse_cell("$C$10"), Some((9, 2)));
        assert_eq!(parse_cell("B0"), None);
        let r = CellRange::parse("C3:A1").unwrap();
        assert_eq!((r.r0, r.c0, r.r1, r.c1), (0, 0, 2, 2));
        assert_eq!(r.a1(), "A1:C3");
        assert_eq!(CellRange::parse("b:b").unwrap().clip(5, 4).unwrap().a1(), "B1:B5");
        assert_eq!(CellRange::parse("2:3").unwrap().clip(5, 4).unwrap().a1(), "A2:D3");
    }

    #[test]
    fn inputs_read_as_numbers_dates_and_text() {
        let n = |s: &str| match parse_input(s) {
            Typed::Number(v, _) => Some(v),
            _ => None,
        };
        assert_eq!(n("1240"), Some(1240.0));
        assert_eq!(n("1,240,000"), Some(1_240_000.0));
        assert_eq!(n("£1,240,000"), Some(1_240_000.0));
        assert_eq!(n("-£5"), Some(-5.0));
        assert_eq!(n("(5)"), Some(-5.0));
        assert_eq!(n("13.7%"), Some(0.137));
        assert_eq!(n("−7.6%").map(|v| (v * 1000.0).round()), Some(-76.0));
        assert_eq!(n("1e3"), Some(1000.0));
        assert_eq!(n(".5"), Some(0.5));
        assert_eq!(n("1,24"), None);
        assert_eq!(n("12a"), None);
        assert_eq!(parse_input("North"), Typed::Text("North".into()));
        assert_eq!(parse_input("TRUE"), Typed::Bool(true));
        assert_eq!(parse_input("20/11/2026"), Typed::Date(46346.0));
        assert_eq!(parse_input("2026-11-20"), Typed::Date(46346.0));
        assert_eq!(parse_input("20 Nov 2026"), Typed::Date(46346.0));
        assert_eq!(parse_input("31/02/2026"), Typed::Text("31/02/2026".into()));
        assert_eq!(show_date(46346.0), "20/11/2026");
        assert_eq!(show_date(46346.5), "20/11/2026 12:00");
    }

    #[test]
    fn numbers_format_by_type() {
        let f = |v: f64, kind, decimals| format_number(v, &DataFormat { kind, decimals, ..Default::default() }).text;
        assert_eq!(f(1240000.0, CellType::Currency, None), "£1,240,000");
        assert_eq!(f(1240.5, CellType::Currency, None), "£1,240.50");
        assert_eq!(f(0.137, CellType::Percentage, None), "13.7%");
        assert_eq!(f(-0.076, CellType::Percentage, Some(1)), "\u{2212}7.6%");
        assert_eq!(f(1234.5678, CellType::Number, Some(2)), "1,234.57");
        assert_eq!(f(1.0 / 3.0, CellType::Automatic, None), "0.333333333");
        let p = format_number(-5.0, &DataFormat { kind: CellType::Number, negative: Negative::RedParentheses, ..Default::default() });
        assert_eq!((p.text.as_str(), p.red), ("(5)", true));
        assert_eq!(general(2220000.0), "2220000");
        assert_eq!(general(1e20), "1E+20");
    }

    #[test]
    fn automatic_formula_results_take_the_columns_look() {
        let mut t = TableData::new(4, 2, 24.0);
        t.cells[1][1] = Cell::new("£1,240,000");
        t.cells[2][1] = Cell::new("£980,000");
        t.cells[3][1] = Cell { input: "=SUM(B2:B3)".into(), value: Some(CellValue::Number(2220000.0)), ..Default::default() };
        let look = t.column_look(1);
        assert_eq!(t.shown(3, 1, look.as_ref()).text, "£2,220,000");
        assert_eq!(t.shown(1, 1, look.as_ref()).text, "£1,240,000", "plain inputs show as typed");
        t.cells[3][0] = Cell { input: "=1/0".into(), value: Some(CellValue::Error { error: "#DIV/0!".into() }), ..Default::default() };
        let s = t.shown(3, 0, None);
        assert_eq!((s.text.as_str(), s.error.as_deref()), ("#DIV/0!", Some("#DIV/0!")));
    }

    #[test]
    fn cells_serialise_compactly() {
        let row = vec![Cell::new("North"), Cell { input: "=B2*2".into(), value: Some(CellValue::Number(4.0)), ..Default::default() }, Cell { input: "=1/0".into(), value: Some(CellValue::Error { error: "#DIV/0!".into() }), ..Default::default() }];
        let j = serde_json::to_string(&row).unwrap();
        assert_eq!(j, r##"["North",{"input":"=B2*2","value":4.0},{"input":"=1/0","value":{"error":"#DIV/0!"}}]"##);
        let back: Vec<Cell> = serde_json::from_str(&j).unwrap();
        assert_eq!(back, row);
        let nums: Vec<Cell> = serde_json::from_str("[12, true, \"x\"]").unwrap();
        assert_eq!(nums.iter().map(|c| c.input.as_str()).collect::<Vec<_>>(), ["12", "TRUE", "x"]);
    }

    #[test]
    fn rows_in_and_out_move_merges_formats_and_headers() {
        let mut t = TableData::new(5, 3, 24.0);
        t.footer_rows = 1;
        t.merges.push(CellRange::parse("A2:B3").unwrap());
        t.add_format(CellRange::parse("C2:C4").unwrap(), CellFormat { bold: Some(true), ..Default::default() });
        t.insert_rows(2, 2, None);
        assert_eq!(t.n_rows(), 7);
        assert_eq!(t.merges[0].a1(), "A2:B5", "rows inside a merge widen it");
        assert_eq!(t.cell_formats[0].range.a1(), "C2:C6");
        t.delete_rows(1, 1);
        assert_eq!(t.merges[0].a1(), "A2:B4");
        assert_eq!(t.cell_formats[0].range.a1(), "C2:C5");
        t.insert_rows(0, 1, None);
        assert_eq!(t.header_rows, 2, "a row in the header is a header row");
        let n = t.n_rows();
        t.delete_rows(n - 1, 1);
        assert_eq!(t.footer_rows, 0);
        t.delete_columns(0, 1);
        assert_eq!(t.merges[0].a1(), "A3:A5", "a merge one column wide stays (it is two rows tall)");
    }

    #[test]
    fn sort_order_follows_the_column_type() {
        let mut t = TableData::new(6, 1, 24.0);
        t.footer_rows = 1;
        for (i, v) in ["Name", "10", "9", "", "100", "Total"].iter().enumerate() {
            t.cells[i][0] = Cell::new(*v);
        }
        assert_eq!(t.sort_order(0, false), [0, 2, 1, 4, 3, 5], "numbers by value, empty last, header and footer stay");
        assert_eq!(t.sort_order(0, true), [0, 4, 1, 2, 3, 5]);
        t.columns[0].format.kind = CellType::Text;
        assert_eq!(t.sort_order(0, false), [0, 2, 1, 4, 3, 5], "as text, in natural order: 9, 10, 100");
        t.cells[1][0] = Cell::new("b");
        assert_eq!(t.sort_order(0, false), [0, 2, 4, 1, 3, 5], "numbers as text before letters");
        assert_eq!(natural_cmp("Item 2", "Item 10"), std::cmp::Ordering::Less);
    }

    #[test]
    fn permute_keeps_merges_whole() {
        let mut t = TableData::new(4, 2, 24.0);
        t.merges.push(CellRange::parse("A2:A3").unwrap());
        assert!(t.permute_rows(&[0, 2, 1, 3]).is_err());
        assert!(t.permute_rows(&[0, 3, 1, 2]).is_ok());
        assert_eq!(t.merges[0].a1(), "A3:A4");
        assert_eq!(TableData::move_order(5, 3, 1, 1), [0, 3, 1, 2, 4]);
        assert_eq!(TableData::move_order(5, 0, 2, 5), [2, 3, 4, 0, 1]);
    }

    #[test]
    fn delimited_text_round_trips() {
        let rows = parse_delimited("Region\tQ2\nNorth\t\"1,240\"\n\"a\tb\"\t\"say \"\"hi\"\"\"\n").unwrap();
        assert_eq!(rows, [vec!["Region", "Q2"], vec!["North", "1,240"], vec!["a\tb", "say \"hi\""]]);
        assert_eq!(parse_delimited(&to_tsv(&rows)).unwrap(), rows);
        assert_eq!(parse_delimited("a,b\nc").unwrap(), [vec!["a", "b"], vec!["c", ""]]);
        assert!(looks_tabular("a\tb"));
        assert!(!looks_tabular("just words"));
    }

    #[test]
    fn repair_makes_a_table_rectangular() {
        let mut t: TableData = serde_json::from_value(serde_json::json!({
            "cells": [["a", "b", "c"], ["d"]], "columns": [{"width": 100}], "rows": [], "header_rows": 9,
            "merges": ["A1:A1", "A1:B2", "B2:C2"]
        }))
        .unwrap();
        assert!(t.repair(24.0));
        assert_eq!((t.n_rows(), t.n_cols(), t.rows.len(), t.header_rows), (2, 3, 2, 2));
        assert_eq!(t.merges.len(), 1);
    }
}
