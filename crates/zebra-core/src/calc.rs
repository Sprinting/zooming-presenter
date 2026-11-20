//! Table formulas, worked out by IronCalc (`ironcalc_base`).
//!
//! Each table is one IronCalc sheet, built in memory the first time it is
//! worked out and kept while the app runs. What was typed is the truth
//! and is what the file saves; IronCalc only produces the values of
//! formula cells, which are written back into the table in the same undo
//! step as the edit that changed them.
//!
//! Rows and columns going in, out or moving also go through IronCalc, so
//! the references in formulas follow the cells they point at
//! ([`reshape`]). The web player has no IronCalc: it draws the values the
//! file saved, and nothing here runs there.
use std::collections::HashMap;

use crate::table::{CellValue, TableData};

/// A change to a table's shape. Indices are 0-based rows or columns.
#[derive(Debug, Clone, PartialEq)]
pub enum Reshape {
    InsertRows { at: usize, count: usize },
    DeleteRows { at: usize, count: usize },
    InsertColumns { at: usize, count: usize },
    DeleteColumns { at: usize, count: usize },
    /// `count` rows from `from` go before row `to` (an index before the move).
    MoveRows { from: usize, count: usize, to: usize },
    MoveColumns { from: usize, count: usize, to: usize },
    /// Row `i` becomes old row `order[i]`; formulas move as copies do, so
    /// `=B2*C2` sorted into row 5 reads `=B5*C5`.
    SortRows { order: Vec<usize> },
}

/// Whether any cell holds a formula.
pub fn has_formulas(t: &TableData) -> bool {
    t.cells.iter().flatten().any(|c| c.is_formula())
}

/// Functions Zebra never runs, whatever the formula engine offers: ones
/// that would reach the network, files or other programs in spreadsheet
/// apps (none of these exist in IronCalc 0.9.1, which has no network or
/// file access; listed so an upgrade can't bring them in unnoticed), and
/// INFO, which reads facts about the computer into a shared document.
pub const BLOCKED_FUNCTIONS: &[&str] = &[
    "WEBSERVICE", "FILTERXML", "IMPORTDATA", "IMPORTXML", "IMPORTHTML", "IMPORTRANGE", "IMPORTFEED", "IMAGE", "RTD", "CALL", "REGISTER.ID",
    "DDE", "EXEC", "SHELL", "INFO", "STOCKHISTORY", "GOOGLEFINANCE", "PY",
];

/// The first blocked function a formula calls (a name followed by `(`,
/// outside string literals), if any.
pub fn blocked_function(formula: &str) -> Option<&'static str> {
    let b = formula.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        let ch = b[i];
        if ch == b'"' {
            in_str = !in_str;
            i += 1;
            continue;
        }
        let starts_name = ch.is_ascii_alphabetic() || ch == b'_';
        if in_str || !starts_name || (i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'.')) {
            i += 1;
            continue;
        }
        let s = i;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
            i += 1;
        }
        let mut j = i;
        while j < b.len() && b[j] == b' ' {
            j += 1;
        }
        if b.get(j) == Some(&b'(') {
            let name = formula[s..i].to_ascii_uppercase();
            // Excel writes newer functions as _xlfn.NAME.
            let name = name.strip_prefix("_XLFN.").unwrap_or(&name);
            if let Some(f) = BLOCKED_FUNCTIONS.iter().find(|f| **f == name) {
                return Some(f);
            }
        }
    }
    None
}

/// The cached sheets, one per table element.
#[derive(Default)]
pub struct Calc {
    #[cfg(not(target_arch = "wasm32"))]
    sheets: HashMap<String, imp::Sheet>,
    #[cfg(target_arch = "wasm32")]
    _sheets: HashMap<String, ()>,
}

impl std::fmt::Debug for Calc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Calc")
    }
}

impl Calc {
    pub fn new() -> Calc {
        Calc::default()
    }

    /// Work out every formula of table `id` and write the values back.
    /// Returns whether a value changed. A table without formulas is not
    /// sent to IronCalc at all; any values it keeps from before are dropped.
    pub fn recalc(&mut self, id: &str, t: &mut TableData) -> bool {
        if !has_formulas(t) {
            self.forget(id);
            let mut changed = false;
            for row in t.cells.iter_mut() {
                // Only rows that hold a stale value are written (and so copied).
                if row.iter().any(|c| c.value.is_some() || c.format.is_some()) {
                    for c in row.iter_mut() {
                        c.value = None;
                        c.format = None;
                    }
                    changed = true;
                }
            }
            return changed;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            imp::recalc(&mut self.sheets, id, t)
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Drop the sheet of a table (deleted, or no formulas any more).
    pub fn forget(&mut self, id: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        self.sheets.remove(id);
        #[cfg(target_arch = "wasm32")]
        let _ = id;
    }

    /// Keep only the sheets of tables `keep` says still exist.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        #[cfg(not(target_arch = "wasm32"))]
        self.sheets.retain(|id, _| keep(id));
        #[cfg(target_arch = "wasm32")]
        let _ = keep;
    }

    /// How many sheets are held.
    pub fn len(&self) -> usize {
        #[cfg(not(target_arch = "wasm32"))]
        return self.sheets.len();
        #[cfg(target_arch = "wasm32")]
        0
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.sheets.clear();
    }
}

/// Change a table's shape and rewrite its formulas to match. Errors when a
/// move would split merged cells.
pub fn reshape(t: &mut TableData, op: &Reshape) -> Result<(), String> {
    // Formulas before, by (row, column).
    let formulas: Vec<(usize, usize, String)> = t
        .cells
        .iter()
        .enumerate()
        .flat_map(|(r, row)| row.iter().enumerate().filter(|(_, c)| c.is_formula()).map(move |(c, cell)| (r, c, cell.input.clone())))
        .collect();
    let body_end = t.body_rows().end;
    let has_footer = t.footer_rows > 0;
    apply_shape(t, op)?;
    if formulas.is_empty() {
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let rewritten = imp::rewrite(&formulas, op, body_end, has_footer)?;
        for ((r, c), f) in rewritten {
            if let Some(cell) = t.cells.get_mut(r).and_then(|row| row.get_mut(c)) {
                cell.input = f;
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (body_end, has_footer);
    Ok(())
}

fn apply_shape(t: &mut TableData, op: &Reshape) -> Result<(), String> {
    match op {
        Reshape::InsertRows { at, count } => t.insert_rows(*at, *count, None),
        Reshape::DeleteRows { at, count } => t.delete_rows(*at, *count),
        Reshape::InsertColumns { at, count } => t.insert_columns(*at, *count, None),
        Reshape::DeleteColumns { at, count } => t.delete_columns(*at, *count),
        Reshape::MoveRows { from, count, to } => t.permute_rows(&TableData::move_order(t.n_rows(), *from, *count, *to))?,
        Reshape::MoveColumns { from, count, to } => t.permute_columns(&TableData::move_order(t.n_cols(), *from, *count, *to))?,
        Reshape::SortRows { order } => t.permute_rows(order)?,
    }
    Ok(())
}

/// Where old index `i` goes along the reshaped axis, or None when deleted.
fn new_index(op: &Reshape, i: usize, n: usize) -> Option<usize> {
    match op {
        Reshape::InsertRows { at, count } | Reshape::InsertColumns { at, count } => Some(if i < *at { i } else { i + count }),
        Reshape::DeleteRows { at, count } | Reshape::DeleteColumns { at, count } => {
            if i < *at {
                Some(i)
            } else if i < at + count {
                None
            } else {
                Some(i - count)
            }
        }
        Reshape::MoveRows { from, count, to } | Reshape::MoveColumns { from, count, to } => {
            TableData::move_order(n, *from, *count, *to).iter().position(|&o| o == i)
        }
        Reshape::SortRows { order } => order.iter().position(|&o| o == i),
    }
}

/// One A1 reference inside a formula's text, 1-based.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ref {
    col_abs: bool,
    col: usize,
    row_abs: bool,
    row: usize,
}

impl Ref {
    fn write(&self) -> String {
        format!("{}{}{}{}", if self.col_abs { "$" } else { "" }, crate::table::column_name(self.col - 1), if self.row_abs { "$" } else { "" }, self.row)
    }
}

fn read_ref(b: &[u8], i: usize) -> Option<(Ref, usize)> {
    let mut j = i;
    let col_abs = b.get(j) == Some(&b'$');
    if col_abs {
        j += 1;
    }
    let ls = j;
    while j < b.len() && b[j].is_ascii_alphabetic() && j - ls < 3 {
        j += 1;
    }
    if j == ls {
        return None;
    }
    let letters = std::str::from_utf8(&b[ls..j]).ok()?;
    let row_abs = b.get(j) == Some(&b'$');
    if row_abs {
        j += 1;
    }
    let ds = j;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    if j == ds {
        return None;
    }
    let row: usize = std::str::from_utf8(&b[ds..j]).ok()?.parse().ok()?;
    let col = crate::table::parse_column(letters)? + 1;
    (row >= 1).then_some((Ref { col_abs, col, row_abs, row }, j))
}

fn ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$' || b == b'!' || b >= 0x80
}

/// Rewrite every `A1:B2` range in a formula (outside string literals).
fn map_ranges(formula: &str, mut f: impl FnMut(Ref, Ref) -> (Ref, Ref)) -> String {
    let b = formula.as_bytes();
    let mut out = String::with_capacity(formula.len());
    let mut i = 0;
    let mut last = 0;
    let mut in_str = false;
    while i < b.len() {
        let ch = b[i];
        if ch == b'"' {
            in_str = !in_str;
            i += 1;
            continue;
        }
        if in_str || (i > 0 && ident_byte(b[i - 1])) || !(ch == b'$' || ch.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        if let Some((a, j)) = read_ref(b, i) {
            if b.get(j) == Some(&b':') {
                if let Some((z, k)) = read_ref(b, j + 1) {
                    let next = b.get(k).copied();
                    if !next.is_some_and(|n| ident_byte(n) || n == b'(') {
                        let (a2, z2) = f(a, z);
                        out.push_str(&formula[last..i]);
                        out.push_str(&a2.write());
                        out.push(':');
                        out.push_str(&z2.write());
                        last = k;
                        i = k;
                        continue;
                    }
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out.push_str(&formula[last..]);
    out
}

/// Before rows `at..at+count` (0-based) go: a range with an end inside
/// them and the other outside shrinks to the rows that stay, as Excel and
/// Numbers do (IronCalc would make the whole reference `#REF!`).
fn clamp_for_delete(formula: &str, at: usize, count: usize, rows: bool) -> String {
    let (first, last) = (at + 1, at + count); // 1-based band
    let inside = |v: usize| v >= first && v <= last;
    map_ranges(formula, |mut a, mut z| {
        let (pa, pz) = if rows { (&mut a.row, &mut z.row) } else { (&mut a.col, &mut z.col) };
        let (lo, hi) = ((*pa).min(*pz), (*pa).max(*pz));
        if !(inside(lo) && inside(hi)) {
            if inside(lo) {
                if *pa == lo { *pa = last + 1 } else { *pz = last + 1 }
            } else if inside(hi) {
                if *pa == hi { *pa = first - 1 } else { *pz = first - 1 }
            }
        }
        (a, z)
    })
}

/// After rows went in at the end of the body (just above the footer):
/// ranges that ended on the last body row take the new rows in, so a
/// total in the footer still adds up the whole column (as in Numbers).
fn extend_for_append(formula: &str, last_body: usize, count: usize) -> String {
    // `last_body`: 1-based row that ended the body before.
    map_ranges(formula, |mut a, mut z| {
        if a.row <= last_body && z.row == last_body && !z.row_abs {
            z.row += count;
        } else if z.row <= last_body && a.row == last_body && !a.row_abs && a.row > z.row {
            a.row += count;
        }
        (a, z)
    })
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;
    use ironcalc_base::cell::CellValue as IcValue;
    use ironcalc_base::expressions::types::CellReferenceIndex;
    use ironcalc_base::types::CellType as IcType;
    use ironcalc_base::Model;
    use std::sync::OnceLock;

    /// A table's sheet and the rows it was last given (shared with the
    /// table, so keeping them costs a pointer per row).
    pub struct Sheet {
        model: Model<'static>,
        rows: Vec<crate::table::CellRow>,
    }

    /// The Mac's time zone (for `TODAY()` and `NOW()`): /etc/localtime
    /// links to …/zoneinfo/Europe/London. UTC when it can't be told.
    fn time_zone() -> &'static str {
        static TZ: OnceLock<&'static str> = OnceLock::new();
        TZ.get_or_init(|| {
            let from_env = std::env::var("TZ").ok().filter(|s| !s.is_empty() && !s.starts_with(':') && s.len() < 64);
            let from_link = std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                let s = p.to_string_lossy().into_owned();
                s.split_once("zoneinfo/").map(|(_, z)| z.to_string())
            });
            let tz = from_env.or(from_link).unwrap_or_else(|| "UTC".into());
            if Model::new_empty("t", "en-GB", &tz, "en").is_ok() {
                Box::leak(tz.into_boxed_str())
            } else {
                "UTC"
            }
        })
    }

    pub fn new_model() -> Model<'static> {
        // en-GB: dates read and show day first, £ is the currency.
        match Model::new_empty("table", "en-GB", time_zone(), "en") {
            Ok(m) => m,
            Err(_) => Model::new_empty("table", "en", "UTC", "en").unwrap_or_else(|e| unreachable!("IronCalc's own English locale: {e}")),
        }
    }

    /// What IronCalc is given for an input. A lone "=" is text, not a
    /// broken formula; a formula calling something Zebra doesn't allow
    /// (see [`blocked_function`]) never reaches IronCalc.
    fn input_for(s: &str) -> String {
        if s.trim() == "=" {
            return format!("'{s}");
        }
        if crate::table::is_formula(s) && blocked_function(s).is_some() {
            // Cells that use it see an error; the cell itself shows #NAME?
            // (see `read_value`).
            return "=NA()".into();
        }
        s.to_string()
    }

    pub fn recalc(sheets: &mut HashMap<String, Sheet>, id: &str, t: &mut TableData) -> bool {
        let sheet = sheets.entry(id.to_string()).or_insert_with(|| Sheet { model: new_model(), rows: vec![] });
        let m = &mut sheet.model;
        let empty = crate::table::CellRow::new(vec![]);
        let rows = t.n_rows().max(sheet.rows.len());
        for r in 0..rows {
            let now = t.cells.get(r).unwrap_or(&empty);
            let was = sheet.rows.get(r).unwrap_or(&empty);
            if now.same(was) {
                continue;
            }
            for c in 0..now.len().max(was.len()) {
                let n = now.get(c).map(|x| x.input.as_str()).unwrap_or("");
                let w = was.get(c).map(|x| x.input.as_str()).unwrap_or("");
                if n == w {
                    continue;
                }
                let (row, col) = (r as i32 + 1, c as i32 + 1);
                if n.is_empty() {
                    if let Ok(ws) = m.workbook.worksheet_mut(0) {
                        let _ = ws.cell_clear_contents(row, col);
                    }
                } else {
                    let _ = m.set_user_input(0, row, col, input_for(n));
                }
            }
        }
        m.evaluate();
        let mut changed = false;
        for (r, row) in t.cells.iter_mut().enumerate() {
            // Work out the row's values first; write (and so copy the row)
            // only when one differs.
            let new: Vec<(usize, Option<CellValue>, Option<String>)> = row
                .iter()
                .enumerate()
                .filter_map(|(c, cell)| {
                    let (value, format) = if !cell.is_formula() {
                        (None, None)
                    } else if blocked_function(&cell.input).is_some() {
                        (Some(CellValue::Error { error: "#NAME?".into() }), None)
                    } else {
                        read_value(m, r, c)
                    };
                    (cell.value != value || cell.format != format).then_some((c, value, format))
                })
                .collect();
            if !new.is_empty() {
                changed = true;
                for (c, value, format) in new {
                    row[c].value = value;
                    row[c].format = format;
                }
            }
        }
        sheet.rows = t.cells.clone();
        changed
    }

    fn read_value(m: &Model<'static>, r: usize, c: usize) -> (Option<CellValue>, Option<String>) {
        let (ri, ci) = (r as i32 + 1, c as i32 + 1);
        let v = match m.get_cell_value_by_index(0, ri, ci) {
            Ok(IcValue::Number(v)) if v.is_finite() => CellValue::Number(v),
            Ok(IcValue::Number(_)) => CellValue::Error { error: "#NUM!".into() },
            Ok(IcValue::Boolean(b)) => CellValue::Bool(b),
            Ok(IcValue::String(s)) if matches!(m.get_cell_type(0, ri, ci), Ok(IcType::ErrorValue)) => CellValue::Error { error: s },
            Ok(IcValue::String(s)) => CellValue::Text(s.chars().take(crate::table::MAX_INPUT).collect()),
            Ok(IcValue::None) | Err(_) => CellValue::Text(String::new()),
        };
        let fmt = m.get_style_for_cell(0, ri, ci).ok().map(|s| s.num_fmt).filter(|f| !f.eq_ignore_ascii_case("general") && !f.is_empty());
        let fmt = if matches!(v, CellValue::Number(_)) { fmt } else { None };
        (Some(v), fmt)
    }

    /// New formula texts, at their new (row, column), for the formulas of
    /// a table about to be reshaped by `op`.
    pub fn rewrite(formulas: &[(usize, usize, String)], op: &Reshape, body_end: usize, has_footer: bool) -> Result<Vec<((usize, usize), String)>, String> {
        let rows_axis = matches!(op, Reshape::InsertRows { .. } | Reshape::DeleteRows { .. } | Reshape::MoveRows { .. } | Reshape::SortRows { .. });
        let n_axis = formulas.iter().map(|(r, c, _)| if rows_axis { *r } else { *c }).max().unwrap_or(0) + 1;
        let n_axis = match op {
            Reshape::MoveRows { from, count, to } | Reshape::MoveColumns { from, count, to } => n_axis.max(from + count).max(*to),
            Reshape::SortRows { order } => order.len(),
            _ => n_axis,
        };
        let mut m = new_model();
        let mut out = vec![];
        if let Reshape::SortRows { order } = op {
            // Copied, not moved: references keep their offsets.
            for (r, c, f) in formulas {
                let Some(nr) = order.iter().position(|o| o == r) else { continue };
                let src = CellReferenceIndex { sheet: 0, row: *r as i32 + 1, column: *c as i32 + 1 };
                let dst = CellReferenceIndex { sheet: 0, row: nr as i32 + 1, column: *c as i32 + 1 };
                let nf = if nr == *r { f.clone() } else { m.extend_copied_value(f, &src, &dst)? };
                out.push(((nr, *c), nf));
            }
            return Ok(out);
        }
        for (r, c, f) in formulas {
            let f = match op {
                Reshape::DeleteRows { at, count } => clamp_for_delete(f, *at, *count, true),
                Reshape::DeleteColumns { at, count } => clamp_for_delete(f, *at, *count, false),
                _ => f.clone(),
            };
            m.set_user_input(0, *r as i32 + 1, *c as i32 + 1, f)?;
        }
        match op {
            Reshape::InsertRows { at, count } => m.insert_rows(0, *at as i32 + 1, *count as i32)?,
            Reshape::DeleteRows { at, count } => m.delete_rows(0, *at as i32 + 1, *count as i32)?,
            Reshape::InsertColumns { at, count } => m.insert_columns(0, *at as i32 + 1, *count as i32)?,
            Reshape::DeleteColumns { at, count } => m.delete_columns(0, *at as i32 + 1, *count as i32)?,
            Reshape::MoveRows { from, count, to } | Reshape::MoveColumns { from, count, to } => {
                let new_first = if to > from { to.saturating_sub(*count) } else { *to };
                let delta = new_first as i32 - *from as i32;
                if delta != 0 && *count > 0 {
                    if rows_axis {
                        m.move_rows_action(0, *from as i32 + 1, *count as i32, delta)?;
                    } else {
                        m.move_columns_action(0, *from as i32 + 1, *count as i32, delta)?;
                    }
                }
            }
            Reshape::SortRows { .. } => {}
        }
        for (r, c, _) in formulas {
            let (nr, nc) = if rows_axis { (new_index(op, *r, n_axis), Some(*c)) } else { (Some(*r), new_index(op, *c, n_axis)) };
            let (Some(nr), Some(nc)) = (nr, nc) else { continue };
            if let Ok(Some(mut f)) = m.get_cell_formula(0, nr as i32 + 1, nc as i32 + 1) {
                if let Reshape::InsertRows { at, count } = op {
                    if has_footer && *at == body_end && body_end > 0 {
                        f = extend_for_append(&f, body_end, *count);
                    }
                }
                out.push(((nr, nc), f));
            }
        }
        Ok(out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::table::Cell;

        fn table(rows: &[&[&str]]) -> TableData {
            let mut t = TableData::new(rows.len(), rows[0].len(), 24.0);
            for (r, row) in rows.iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    t.cells[r][c] = Cell::new(*v);
                }
            }
            t
        }

        fn inputs(t: &TableData, c: usize) -> Vec<&str> {
            t.cells.iter().map(|r| r[c].input.as_str()).collect()
        }

        #[test]
        fn formulas_work_out_and_errors_show() {
            let mut t = table(&[&["Region", "Q2"], &["North", "£1,240,000"], &["South", "980000"], &["Total", "=SUM(B2:B3)"], &["Bad", "=B4/0"], &["Odd", "=FOO(1)"]]);
            let mut calc = Calc::new();
            assert!(calc.recalc("b1", &mut t));
            assert_eq!(t.cells[3][1].value, Some(CellValue::Number(2_220_000.0)));
            assert_eq!(t.cells[4][1].value, Some(CellValue::Error { error: "#DIV/0!".into() }));
            assert!(matches!(&t.cells[5][1].value, Some(CellValue::Error { .. })));
            assert_eq!(t.cells[1][1].value, None, "plain inputs keep no value");
            // An edit works out again from the cached sheet.
            t.cells[2][1] = Cell::new("1000000");
            assert!(calc.recalc("b1", &mut t));
            assert_eq!(t.cells[3][1].value, Some(CellValue::Number(2_240_000.0)));
            assert!(!calc.recalc("b1", &mut t), "nothing changed");
            // A sheet built fresh agrees with the cached one.
            let mut fresh = t.clone();
            assert!(!Calc::new().recalc("b1", &mut fresh));
            t.cells[2][1] = Cell::default();
            calc.recalc("b1", &mut t);
            assert_eq!(t.cells[3][1].value, Some(CellValue::Number(1_240_000.0)));
        }

        #[test]
        fn dates_from_formulas_keep_their_format() {
            let mut t = table(&[&["A"], &["=TODAY()"], &["=DATE(2026,11,20)"]]);
            Calc::new().recalc("b1", &mut t);
            assert!(t.cells[1][0].format.as_deref().is_some_and(|f| f.contains("yy")), "{:?}", t.cells[1][0].format);
            let look = t.column_look(0);
            assert_eq!(t.shown(2, 0, look.as_ref()).text, "20/11/2026");
        }

        #[test]
        fn rows_in_and_out_rewrite_references() {
            let mut t = table(&[&["Q"], &["1"], &["2"], &["3"], &["4"], &["=SUM(A2:A5)"]]);
            t.footer_rows = 1;
            reshape(&mut t, &Reshape::InsertRows { at: 2, count: 1 }).unwrap();
            assert_eq!(t.cells[6][0].input, "=SUM(A2:A6)");
            // The first row of the range goes: the range shrinks (Excel), no #REF!.
            reshape(&mut t, &Reshape::DeleteRows { at: 1, count: 1 }).unwrap();
            assert_eq!(t.cells[5][0].input, "=SUM(A2:A5)");
            // Its last row goes.
            reshape(&mut t, &Reshape::DeleteRows { at: 4, count: 1 }).unwrap();
            assert_eq!(inputs(&t, 0).last().copied(), Some("=SUM(A2:A4)"));
            // A row added at the end of the body joins the total.
            let end = t.body_rows().end;
            reshape(&mut t, &Reshape::InsertRows { at: end, count: 1 }).unwrap();
            assert_eq!(inputs(&t, 0).last().copied(), Some("=SUM(A2:A5)"));
            // A single reference to a deleted row is #REF!, as in Excel.
            let mut u = table(&[&["1"], &["=A1*2"]]);
            reshape(&mut u, &Reshape::DeleteRows { at: 0, count: 1 }).unwrap();
            assert!(u.cells[0][0].input.contains("#REF!"), "{}", u.cells[0][0].input);
        }

        #[test]
        fn columns_in_and_out_rewrite_references() {
            let mut t = table(&[&["1", "2", "3", "=SUM(A1:C1)", "=C1"]]);
            reshape(&mut t, &Reshape::InsertColumns { at: 1, count: 2 }).unwrap();
            assert_eq!(t.cells[0][5].input, "=SUM(A1:E1)");
            assert_eq!(t.cells[0][6].input, "=E1");
            reshape(&mut t, &Reshape::DeleteColumns { at: 0, count: 1 }).unwrap();
            assert_eq!(t.cells[0][4].input, "=SUM(A1:D1)");
        }

        #[test]
        fn moving_rows_keeps_references_on_their_cells() {
            let mut t = table(&[&["10", "=A1"], &["20", "=A2"], &["30", "=A1+A3"]]);
            t.header_rows = 0;
            reshape(&mut t, &Reshape::MoveRows { from: 0, count: 1, to: 3 }).unwrap();
            assert_eq!(inputs(&t, 0), ["20", "30", "10"]);
            assert_eq!(inputs(&t, 1), ["=A1", "=A3+A2", "=A3"]);
        }

        #[test]
        fn sorting_copies_formulas_row_relative() {
            let mut t = table(&[&["Name", "Qty", "Price", "Total"], &["b", "2", "5", "=B2*C2"], &["a", "3", "4", "=B3*C3"], &["Sum", "", "", "=SUM(D2:D3)"]]);
            t.footer_rows = 1;
            let order = t.sort_order(0, false);
            reshape(&mut t, &Reshape::SortRows { order }).unwrap();
            assert_eq!(inputs(&t, 0), ["Name", "a", "b", "Sum"]);
            assert_eq!(inputs(&t, 3), ["Total", "=B2*C2", "=B3*C3", "=SUM(D2:D3)"]);
            let mut calc = Calc::new();
            calc.recalc("b1", &mut t);
            assert_eq!(t.cells[1][3].value, Some(CellValue::Number(12.0)));
            assert_eq!(t.cells[3][3].value, Some(CellValue::Number(22.0)));
        }

        #[test]
        fn range_text_rewrites_skip_strings_and_functions() {
            assert_eq!(clamp_for_delete("=SUM(B2:B5)&\"B2:B5\"", 1, 1, true), "=SUM(B3:B5)&\"B2:B5\"");
            assert_eq!(clamp_for_delete("=LOG10(A1)+SUM($B$2:$B$5)", 4, 1, true), "=LOG10(A1)+SUM($B$2:$B$4)");
            assert_eq!(clamp_for_delete("=SUM(A1:D1)", 0, 1, false), "=SUM(B1:D1)");
        }
    }
}
