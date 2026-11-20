//! Undo steps for tables, without serialising them.
//!
//! A JSON diff of a table compares its rows position by position: a row
//! inserted at the top, or a sort, would store the whole table twice, and
//! every edit would turn the whole table into JSON to find out. Instead a
//! table's change is worked out from its rows and columns directly:
//!
//! * Rows and columns carry ids that stay with them ([`CellRow`],
//!   [`Column`]), so rows or columns inserted, deleted or moved are
//!   recorded as splices (what went in, where) or a permutation (a sort,
//!   a move: one number per row).
//! * Cells are then compared only where the row isn't the very same
//!   memory as before (rows are shared until changed): one cell edited
//!   stores that cell.
//! * Everything else (merges, formats, header counts, style) is small and
//!   is kept whole when it changes.
use std::collections::HashMap;

use crate::delta::Splice;
use crate::table::{Cell, CellRow, Column, Row, TableData, TableMeta};

/// How one axis (rows or columns) was rearranged.
#[derive(Debug, Clone, PartialEq)]
pub enum AxisChange<T> {
    /// Item `i` after is item `order[i]` before.
    Permute(Vec<u32>),
    /// Runs removed and inserted (indices before the change).
    Splices(Vec<Splice<T>>),
}

/// One direction of a change to a table's contents.
#[derive(Debug, Clone, PartialEq)]
pub struct TableChange {
    /// Rows and columns before and after (to check it fits).
    rows: (usize, usize),
    cols: (usize, usize),
    /// Applied first, to every row.
    columns: Option<AxisChange<Column>>,
    /// Then the rows (inserted rows carry their cells, in the new columns).
    row_order: Option<AxisChange<(CellRow, Row)>>,
    /// Then the cells that differ: (row, column, cell), positions after.
    cells: Vec<(u32, u32, Cell)>,
    /// Columns whose width or type changed: (index after, column).
    column_values: Vec<(u32, Column)>,
    /// Rows whose height changed.
    row_values: Vec<(u32, Row)>,
    meta: Option<Box<TableMeta>>,
}

/// Where each item after a change came from: `Some(i)` (item `i` before)
/// or `None` (inserted).
fn sources<T>(n_before: usize, ch: &Option<AxisChange<T>>) -> Vec<Option<usize>> {
    match ch {
        None => (0..n_before).map(Some).collect(),
        Some(AxisChange::Permute(order)) => order.iter().map(|&o| Some(o as usize)).collect(),
        Some(AxisChange::Splices(sp)) => {
            let mut out = Vec::with_capacity(n_before);
            let mut cur = 0;
            for s in sp {
                out.extend((cur..s.at).map(Some));
                out.extend(std::iter::repeat_n(None, s.insert.len()));
                cur = s.at + s.remove;
            }
            out.extend((cur..n_before).map(Some));
            out
        }
    }
}

/// How `b` is `a` rearranged, by the items' ids.
fn axis<T: Clone, I: Clone>(a: &[T], b: &[T], id: impl Fn(&T) -> u64, item: impl Fn(usize) -> I) -> Option<AxisChange<I>> {
    if a.len() == b.len() && a.iter().zip(b).all(|(x, y)| id(x) == id(y)) {
        return None;
    }
    if a.len() == b.len() {
        let pos: HashMap<u64, usize> = a.iter().enumerate().map(|(i, x)| (id(x), i)).collect();
        if pos.len() == a.len() {
            let order: Option<Vec<u32>> = b.iter().map(|y| pos.get(&id(y)).map(|&i| i as u32)).collect();
            if let Some(order) = order {
                let mut seen = vec![false; a.len()];
                if order.iter().all(|&o| !std::mem::replace(&mut seen[o as usize], true)) {
                    return Some(AxisChange::Permute(order));
                }
            }
        }
    }
    let ka: Vec<u64> = a.iter().map(&id).collect();
    let kb: Vec<u64> = b.iter().map(&id).collect();
    let sp = crate::delta::splices(&ka, &kb, |k| *k);
    // The splices' inserts are positions in `b`; turn them into items.
    let mut out = Vec::with_capacity(sp.len());
    let mut shift: isize = 0;
    for s in sp {
        let at_b = (s.at as isize + shift) as usize;
        let n = s.insert.len();
        out.push(Splice { at: s.at, remove: s.remove, insert: (at_b..at_b + n).map(&item).collect() });
        shift += n as isize - s.remove as isize;
    }
    Some(AxisChange::Splices(out))
}

impl TableChange {
    /// The change that turns `a` into `b`.
    pub fn between(a: &TableData, b: &TableData) -> TableChange {
        let columns = axis(&a.columns, &b.columns, Column::id, |j| b.columns[j].clone());
        let row_order = axis(&a.cells, &b.cells, CellRow::id, |i| (b.cells[i].clone(), b.rows.get(i).copied().unwrap_or(Row::new(1.0))));
        let col_src = sources(a.n_cols(), &columns);
        let row_src = sources(a.n_rows(), &row_order);
        let blank = Cell::default();
        let mut cells = vec![];
        for (i, src) in row_src.iter().enumerate() {
            let (Some(ai), Some(brow)) = (*src, b.cells.get(i)) else { continue };
            let arow = &a.cells[ai];
            if columns.is_none() && arow.same(brow) {
                continue;
            }
            for (j, bc) in brow.iter().enumerate() {
                let ac = col_src.get(j).copied().flatten().and_then(|aj| arow.get(aj)).unwrap_or(&blank);
                if ac != bc {
                    cells.push((i as u32, j as u32, bc.clone()));
                }
            }
        }
        let column_values = col_src
            .iter()
            .enumerate()
            .filter_map(|(j, s)| {
                let (aj, bc) = ((*s)?, b.columns.get(j)?);
                (a.columns[aj] != *bc).then(|| (j as u32, bc.clone()))
            })
            .collect();
        let row_values = row_src
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let (ai, br) = ((*s)?, b.rows.get(i)?);
                (a.rows.get(ai) != Some(br)).then_some((i as u32, *br))
            })
            .collect();
        let meta = (!a.same_meta(b)).then(|| Box::new(b.meta()));
        TableChange { rows: (a.n_rows(), b.n_rows()), cols: (a.n_cols(), b.n_cols()), columns, row_order, cells, column_values, row_values, meta }
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_none() && self.row_order.is_none() && self.cells.is_empty() && self.column_values.is_empty() && self.row_values.is_empty() && self.meta.is_none()
    }

    /// Rough heap size: what an undo step holds for this table.
    pub fn approx_bytes(&self) -> usize {
        let cell = |c: &Cell| 96 + c.input.len() + c.format.as_ref().map_or(0, |f| f.len()) + c.spans.len() * 64;
        let axis_rows = match &self.row_order {
            None => 0,
            Some(AxisChange::Permute(o)) => o.len() * 4,
            Some(AxisChange::Splices(sp)) => sp.iter().map(|s| 32 + s.insert.iter().map(|(r, _)| 64 + r.iter().map(cell).sum::<usize>()).sum::<usize>()).sum(),
        };
        let axis_cols = match &self.columns {
            None => 0,
            Some(AxisChange::Permute(o)) => o.len() * 4,
            Some(AxisChange::Splices(sp)) => sp.iter().map(|s| 32 + s.insert.len() * 64).sum(),
        };
        let meta = self.meta.as_ref().map_or(0, |m| 128 + m.merges.len() * 32 + m.cell_formats.len() * 160);
        axis_rows + axis_cols + self.cells.iter().map(|(_, _, c)| 8 + cell(c)).sum::<usize>() + self.column_values.len() * 80 + self.row_values.len() * 24 + meta
    }

    /// Whether this change was made from a table shaped like `t`.
    pub fn fits(&self, t: &TableData) -> bool {
        let (rows, cols) = (self.rows.0, self.cols.0);
        if t.n_rows() != rows || t.n_cols() != cols || t.rows.len() != rows || t.cells.iter().any(|r| r.len() != cols) {
            return false;
        }
        let axis_ok = |n: usize, after: usize, perm: Option<&Vec<u32>>, sp: Option<(usize, Vec<(usize, usize)>)>| -> bool {
            if let Some(o) = perm {
                return o.len() == n && n == after && o.iter().all(|&i| (i as usize) < n);
            }
            if let Some((ins, runs)) = sp {
                let mut cur = 0;
                let mut removed = 0;
                for (at, rm) in runs {
                    match at.checked_add(rm) {
                        Some(end) if at >= cur && end <= n => cur = end,
                        _ => return false,
                    }
                    removed += rm;
                }
                return n - removed + ins == after;
            }
            n == after
        };
        let cols_ok = match &self.columns {
            None => axis_ok(cols, self.cols.1, None, None),
            Some(AxisChange::Permute(o)) => axis_ok(cols, self.cols.1, Some(o), None),
            Some(AxisChange::Splices(sp)) => axis_ok(cols, self.cols.1, None, Some((sp.iter().map(|s| s.insert.len()).sum(), sp.iter().map(|s| (s.at, s.remove)).collect()))),
        };
        let rows_ok = match &self.row_order {
            None => axis_ok(rows, self.rows.1, None, None),
            Some(AxisChange::Permute(o)) => axis_ok(rows, self.rows.1, Some(o), None),
            Some(AxisChange::Splices(sp)) => {
                axis_ok(rows, self.rows.1, None, Some((sp.iter().map(|s| s.insert.len()).sum(), sp.iter().map(|s| (s.at, s.remove)).collect())))
                    && sp.iter().flat_map(|s| &s.insert).all(|(r, _)| r.len() == self.cols.1)
            }
        };
        let (r1, c1) = (self.rows.1 as u32, self.cols.1 as u32);
        cols_ok
            && rows_ok
            && self.cells.iter().all(|(i, j, _)| *i < r1 && *j < c1)
            && self.column_values.iter().all(|(j, _)| *j < c1)
            && self.row_values.iter().all(|(i, _)| *i < r1)
    }

    /// Apply to `t`, which must [fit](TableChange::fits).
    pub fn apply(&self, t: &mut TableData) {
        match &self.columns {
            None => {}
            Some(AxisChange::Permute(order)) => {
                for row in &mut t.cells {
                    let moved: Vec<Cell> = order.iter().map(|&o| row[o as usize].clone()).collect();
                    row.set(moved);
                }
                let cols = std::mem::take(&mut t.columns);
                t.columns = order.iter().map(|&o| cols[o as usize].clone()).collect();
            }
            Some(AxisChange::Splices(sp)) => {
                for row in &mut t.cells {
                    let mut v: Vec<Cell> = row.to_vec();
                    let blanks: Vec<Splice<Cell>> = sp.iter().map(|s| Splice { at: s.at, remove: s.remove, insert: vec![Cell::default(); s.insert.len()] }).collect();
                    crate::delta::splice_in_place(&mut v, &blanks);
                    row.set(v);
                }
                crate::delta::splice_in_place(&mut t.columns, sp);
            }
        }
        match &self.row_order {
            None => {}
            Some(AxisChange::Permute(order)) => {
                let cells = std::mem::take(&mut t.cells);
                let rows = std::mem::take(&mut t.rows);
                t.cells = order.iter().map(|&o| cells[o as usize].clone()).collect();
                t.rows = order.iter().map(|&o| rows[o as usize]).collect();
            }
            Some(AxisChange::Splices(sp)) => {
                let cells: Vec<Splice<CellRow>> = sp.iter().map(|s| Splice { at: s.at, remove: s.remove, insert: s.insert.iter().map(|(c, _)| c.clone()).collect() }).collect();
                let rows: Vec<Splice<Row>> = sp.iter().map(|s| Splice { at: s.at, remove: s.remove, insert: s.insert.iter().map(|(_, r)| *r).collect() }).collect();
                crate::delta::splice_in_place(&mut t.cells, &cells);
                crate::delta::splice_in_place(&mut t.rows, &rows);
            }
        }
        for (i, j, c) in &self.cells {
            t.cells[*i as usize][*j as usize] = c.clone();
        }
        for (j, c) in &self.column_values {
            let id = t.columns[*j as usize].id();
            t.columns[*j as usize] = Column { id, ..c.clone() };
        }
        for (i, r) in &self.row_values {
            t.rows[*i as usize] = *r;
        }
        if let Some(m) = &self.meta {
            t.set_meta((**m).clone());
        }
    }
}
