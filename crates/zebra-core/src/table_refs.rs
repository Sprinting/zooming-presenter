//! Cell references inside a formula: finding them (to colour them in the
//! formula bar and outline them on the table) and moving them (a formula
//! pasted somewhere else, as Numbers and Excel do).
use std::ops::Range;

use super::{column_name, parse_column, CellRange};

/// One `A1`, `$A1`, `A$1` or `$A$1` inside a formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRef {
    pub row: usize,
    pub col: usize,
    /// `$` before the column: it stays when pasted.
    pub col_abs: bool,
    /// `$` before the row.
    pub row_abs: bool,
}

impl CellRef {
    fn parse(s: &str) -> Option<CellRef> {
        let b = s.as_bytes();
        let mut i = 0;
        let col_abs = b.first() == Some(&b'$');
        if col_abs {
            i += 1;
        }
        let l0 = i;
        while i < b.len() && b[i].is_ascii_alphabetic() {
            i += 1;
        }
        let letters = &s[l0..i];
        let row_abs = b.get(i) == Some(&b'$');
        if row_abs {
            i += 1;
        }
        let digits = &s[i..];
        if letters.is_empty() || digits.is_empty() || !digits.bytes().all(|d| d.is_ascii_digit()) || digits.len() > 7 {
            return None;
        }
        let col = parse_column(letters)?;
        let row: usize = digits.parse().ok()?;
        (row >= 1).then(|| CellRef { row: row - 1, col, col_abs, row_abs })
    }

    fn write(&self) -> String {
        format!("{}{}{}{}", if self.col_abs { "$" } else { "" }, column_name(self.col), if self.row_abs { "$" } else { "" }, self.row + 1)
    }

    /// Moved by (rows, columns), absolute parts staying; None when it
    /// leaves a table of `rows` x `cols`.
    fn shifted(&self, dr: i64, dc: i64, rows: usize, cols: usize) -> Option<CellRef> {
        let r = if self.row_abs { self.row as i64 } else { self.row as i64 + dr };
        let c = if self.col_abs { self.col as i64 } else { self.col as i64 + dc };
        let moved = r != self.row as i64 || c != self.col as i64;
        let inside = (0..rows as i64).contains(&r) && (0..cols as i64).contains(&c);
        // A reference that didn't move stays as written, inside or not.
        (inside || !moved).then_some(CellRef { row: r.max(0) as usize, col: c.max(0) as usize, ..*self })
    }
}

/// A reference in a formula: one cell or a range, and where it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefToken {
    /// Byte range in the formula.
    pub span: Range<usize>,
    pub start: CellRef,
    pub end: Option<CellRef>,
}

impl RefToken {
    /// The cells it covers.
    pub fn range(&self) -> CellRange {
        let e = self.end.unwrap_or(self.start);
        CellRange::new(self.start.row.min(e.row), self.start.col.min(e.col), self.start.row.max(e.row), self.start.col.max(e.col))
    }
}

/// Every cell reference in `formula`, in order. Text in quotes, function
/// names (`LOG10(`) and names that aren't cells are skipped.
pub fn scan(formula: &str) -> Vec<RefToken> {
    let b = formula.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    let mut in_str = false;
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'$' || c == b':' || c == b'_' || c == b'.';
    while i < b.len() {
        let ch = b[i];
        if ch == b'"' {
            in_str = !in_str;
            i += 1;
            continue;
        }
        let starts = !in_str && (ch.is_ascii_alphabetic() || ch == b'$') && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'.' || b[i - 1] == b'$' || b[i - 1] == b'!'));
        if !starts {
            i += 1;
            continue;
        }
        let j = (i..b.len()).find(|&k| !word(b[k])).unwrap_or(b.len());
        // A function or a sheet name, not a reference.
        let next = b.get(j).copied();
        if next == Some(b'(') || next == Some(b'!') {
            i = j;
            continue;
        }
        let text = &formula[i..j];
        let mut parts = text.split(':');
        let a = parts.next().and_then(CellRef::parse);
        let e = parts.next().map(CellRef::parse);
        let extra = parts.next().is_some();
        match (a, e, extra) {
            (Some(start), None, false) => out.push(RefToken { span: i..j, start, end: None }),
            (Some(start), Some(Some(end)), false) => out.push(RefToken { span: i..j, start, end: Some(end) }),
            _ => {}
        }
        i = j;
    }
    out
}

/// `formula` pasted `dr` rows and `dc` columns away from where it was
/// copied, into a table of `rows` x `cols`: relative parts move, `$` parts
/// stay, and a reference that would leave the table becomes `#REF!`.
pub fn shift(formula: &str, dr: i64, dc: i64, rows: usize, cols: usize) -> String {
    if !formula.starts_with('=') || (dr == 0 && dc == 0) {
        return formula.to_string();
    }
    let mut out = String::with_capacity(formula.len());
    let mut at = 0;
    for t in scan(formula) {
        out.push_str(&formula[at..t.span.start]);
        let a = t.start.shifted(dr, dc, rows, cols);
        let e = t.end.map(|e| e.shifted(dr, dc, rows, cols));
        match (a, e) {
            (Some(a), None) => out.push_str(&a.write()),
            (Some(a), Some(Some(e))) => {
                out.push_str(&a.write());
                out.push(':');
                out.push_str(&e.write());
            }
            _ => out.push_str("#REF!"),
        }
        at = t.span.end;
    }
    out.push_str(&formula[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_references_not_functions_or_text() {
        let f = "=SUM(A2:B$3)+LOG10(4)+\"C5\"+$D$9*e1+Sheet!A1+my_A1";
        let got: Vec<&str> = scan(f).iter().map(|t| &f[t.span.clone()]).collect();
        assert_eq!(got, ["A2:B$3", "$D$9", "e1"]);
        assert_eq!(scan("=A1:B2")[0].range(), CellRange::new(0, 0, 1, 1));
        assert!(scan("=TRUE+A0+A:A+A1:B2:C3").is_empty());
    }

    #[test]
    fn pasting_moves_relative_parts_only() {
        assert_eq!(shift("=A1+$B$2+C$3+$D4", 2, 1, 20, 10), "=B3+$B$2+D$3+$D6");
        assert_eq!(shift("=SUM(A2:A5)*2", 0, 2, 10, 10), "=SUM(C2:C5)*2");
        assert_eq!(shift("=\"A1\"&A1", 1, 0, 10, 10), "=\"A1\"&A2");
        assert_eq!(shift("plain A1", 1, 1, 10, 10), "plain A1");
        // Off the table: #REF!, the whole range.
        assert_eq!(shift("=A1+B2", -1, 0, 10, 10), "=#REF!+B1");
        assert_eq!(shift("=SUM(A1:A9)", 0, 9, 10, 10), "=SUM(J1:J9)");
        assert_eq!(shift("=SUM(A1:A9)", 0, 10, 10, 10), "=SUM(#REF!)");
        assert_eq!(shift("=$A$1+A1", 0, -1, 10, 10), "=$A$1+#REF!");
        // An absolute reference outside the table stays as written.
        assert_eq!(shift("=$Z$99+A1", 1, 0, 10, 10), "=$Z$99+A2");
    }
}
