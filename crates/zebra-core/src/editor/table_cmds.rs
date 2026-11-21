//! The table commands: adding tables, typing into cells, rows and columns
//! in and out, merging, formatting, sorting. Each works out the table's
//! formulas again before it returns, so the new values are part of the
//! same undo step as the edit.
use super::*;
use crate::calc::Reshape;
use crate::table::{CellFormat, CellInput, CellRange, Limit, RangeFormat, TableData, MAX_HEADERS};

/// An input over a limit, as the API reports it.
fn too_big(l: Limit) -> ZError {
    ZError::new("too_big", l.message()).hint("nothing was changed").about(crate::error::About::Table)
}

/// Refused if growing `t` to `rows` x `cols` goes over a limit. A table
/// that doesn't grow is never refused: one a file brought in bigger than
/// the limits stays editable.
fn check_growth(t: &TableData, rows: usize, cols: usize) -> ZResult<()> {
    if rows <= t.n_rows() && cols <= t.n_cols() {
        return Ok(());
    }
    match Limit::of_size(rows, cols) {
        Some(l) => Err(too_big(l)),
        None => Ok(()),
    }
}

/// The first input over a limit, if any.
fn check_inputs<'a>(inputs: impl IntoIterator<Item = &'a String>) -> ZResult<()> {
    for s in inputs {
        if let Some(l) = Limit::of_input(s) {
            return Err(too_big(l));
        }
    }
    Ok(())
}

impl Editor {
    pub(super) fn apply_table(&mut self, cmd: Command, out: &mut Outcome) -> ZResult<()> {
        let theme = self.theme();
        let body = theme.text_style(TextStyle::Body).size;
        let id = match cmd {
            Command::AddTable { rows, columns, cells, csv, x, y, style, header_rows, header_columns, footer_rows, scale, place, name, id } => {
                let grid: Option<Vec<Vec<String>>> = match (cells, csv) {
                    (Some(_), Some(_)) => return Err(ZError::invalid("give `cells` or `csv`, not both")),
                    (Some(c), None) => Some(c.into_iter().map(|r| r.into_iter().map(CellInput::into_input).collect()).collect()),
                    (None, Some(s)) => Some(crate::table::parse_delimited(&s).map_err(too_big)?),
                    (None, None) => None,
                };
                let grid_rows = grid.as_ref().map_or(0, |g| g.len());
                let grid_cols = grid.as_ref().map_or(0, |g| g.iter().map(|r| r.len()).max().unwrap_or(0));
                let nr = rows.unwrap_or(0).max(grid_rows);
                let nc = columns.unwrap_or(0).max(grid_cols);
                let (nr, nc) = if grid.is_none() && rows.is_none() && columns.is_none() { (4, 4) } else { (nr.max(1), nc.max(1)) };
                if let Some(l) = Limit::of_size(nr, nc) {
                    return Err(too_big(l));
                }
                if let Some(g) = &grid {
                    check_inputs(g.iter().flatten())?;
                }
                let mut t = TableData::new(nr, nc, body);
                if let Some(g) = &grid {
                    for (r, row) in g.iter().enumerate() {
                        for (c, v) in row.iter().enumerate() {
                            t.cells[r][c] = crate::table::Cell::new(v.clone());
                        }
                    }
                }
                t.style = style.unwrap_or_default();
                for (v, max) in [(header_rows, nr), (footer_rows, nr), (header_columns, nc)] {
                    if v.is_some_and(|v| v > MAX_HEADERS || v as usize > max) {
                        return Err(ZError::invalid(format!("header and footer rows and header columns are 0 to {MAX_HEADERS}, and fewer than the table has")));
                    }
                }
                t.header_rows = header_rows.unwrap_or(1).min(nr as u8);
                t.header_columns = header_columns.unwrap_or(0);
                t.footer_rows = footer_rows.unwrap_or(0).min((nr - t.header_rows as usize) as u8);
                if grid.is_some() {
                    fit_columns(&*self.measure, &theme, &mut t);
                }
                let mut e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform { x, y, scale: scale.unwrap_or(1.0), rotation: 0.0 },
                    size: Size { w: 0.0, h: 0.0 },
                    locked: false,
                    name: name.unwrap_or_default(),
                    placeholder: false,
                    group: None,
                    kind: ElementKind::Table(t),
                };
                if let Some(p) = place {
                    // Placing needs the size (measured again once in).
                    measure_text(&*self.measure, &theme, &mut e);
                    self.place(&mut e, &p, scale.is_none())?;
                }
                // Inserting works it out and measures it (`table_settled`).
                return self.insert_element(e, out).map(|_| ());
            }
            Command::SetCells { id, at, values } => {
                let (r0, c0) = crate::table::parse_cell(&at).ok_or_else(|| ZError::invalid(format!("'{at}' is not a cell like B2")))?;
                let rows = values.len();
                let cols = values.iter().map(|r| r.len()).max().unwrap_or(0);
                if rows == 0 || cols == 0 {
                    return Err(ZError::invalid("values is empty").hint("rows of values: [[\"North\", 1240]]"));
                }
                let values: Vec<Vec<String>> = values.into_iter().map(|r| r.into_iter().map(CellInput::into_input).collect()).collect();
                check_inputs(values.iter().flatten())?;
                let t = self.table_mut(&id)?;
                check_growth(t, t.n_rows().max(r0.saturating_add(rows)), t.n_cols().max(c0.saturating_add(cols)))?;
                // Grow (formulas see the new rows as typing on would).
                if c0 + cols > t.n_cols() {
                    let n = t.n_cols();
                    self.reshape_table(&id, &Reshape::InsertColumns { at: n, count: c0 + cols - n })?;
                }
                let t = self.table_mut(&id)?;
                // Writing from the body on: new rows go in above the
                // footer, which moves down out of the way. From the footer
                // on: they go at the end.
                let body_end = t.body_rows().end;
                let (at, need) = if r0 < body_end && t.footer_rows > 0 { (body_end, (r0 + rows).saturating_sub(body_end)) } else { (t.n_rows(), (r0 + rows).saturating_sub(t.n_rows())) };
                check_growth(t, t.n_rows() + need, t.n_cols())?;
                if need > 0 {
                    self.reshape_table(&id, &Reshape::InsertRows { at, count: need })?;
                }
                let t = self.table_mut(&id)?;
                for (dr, row) in values.into_iter().enumerate() {
                    for (dc, input) in row.into_iter().enumerate() {
                        let (r, c) = (r0 + dr, c0 + dc);
                        if t.anchor(r, c) != (r, c) {
                            continue; // under a merged cell
                        }
                        // Compared first: writing copies the row.
                        if t.cells[r][c].input != input {
                            t.cells[r][c] = crate::table::Cell::new(input);
                        }
                    }
                }
                id
            }
            Command::InsertRows { id, index, count } => {
                let t = self.table_mut(&id)?;
                if index > t.n_rows() {
                    return Err(ZError::invalid(format!("index {index} is past the last row ({} rows)", t.n_rows())));
                }
                if count == 0 {
                    return Err(ZError::invalid("count must be at least 1"));
                }
                if let Some(l) = Limit::of_size(t.n_rows().saturating_add(count), t.n_cols()) {
                    return Err(too_big(l));
                }
                self.reshape_table(&id, &Reshape::InsertRows { at: index, count })?;
                id
            }
            Command::InsertColumns { id, index, count } => {
                let t = self.table_mut(&id)?;
                if index > t.n_cols() {
                    return Err(ZError::invalid(format!("index {index} is past the last column ({} columns)", t.n_cols())));
                }
                if count == 0 {
                    return Err(ZError::invalid("count must be at least 1"));
                }
                if let Some(l) = Limit::of_size(t.n_rows(), t.n_cols().saturating_add(count)) {
                    return Err(too_big(l));
                }
                self.reshape_table(&id, &Reshape::InsertColumns { at: index, count })?;
                id
            }
            Command::DeleteRows { id, index, count } => {
                let t = self.table_mut(&id)?;
                if count == 0 || index.saturating_add(count) > t.n_rows() {
                    return Err(ZError::invalid(format!("rows {index} to {} are not all in the table ({} rows)", index.saturating_add(count.max(1) - 1), t.n_rows())));
                }
                if count == t.n_rows() {
                    return Err(ZError::invalid("a table keeps at least one row").hint("delete the table instead"));
                }
                self.reshape_table(&id, &Reshape::DeleteRows { at: index, count })?;
                id
            }
            Command::DeleteColumns { id, index, count } => {
                let t = self.table_mut(&id)?;
                if count == 0 || index.saturating_add(count) > t.n_cols() {
                    return Err(ZError::invalid(format!("columns {index} to {} are not all in the table ({} columns)", index.saturating_add(count.max(1) - 1), t.n_cols())));
                }
                if count == t.n_cols() {
                    return Err(ZError::invalid("a table keeps at least one column").hint("delete the table instead"));
                }
                self.reshape_table(&id, &Reshape::DeleteColumns { at: index, count })?;
                id
            }
            Command::MoveRows { id, from, count, to } => {
                let t = self.table_mut(&id)?;
                let n = t.n_rows();
                if count == 0 || from.saturating_add(count) > n || to > n {
                    return Err(ZError::invalid(format!("rows out of range ({n} rows, 0-based)")));
                }
                self.reshape_table(&id, &Reshape::MoveRows { from, count, to })?;
                id
            }
            Command::MoveColumns { id, from, count, to } => {
                let t = self.table_mut(&id)?;
                let n = t.n_cols();
                if count == 0 || from.saturating_add(count) > n || to > n {
                    return Err(ZError::invalid(format!("columns out of range ({n} columns, 0-based)")));
                }
                self.reshape_table(&id, &Reshape::MoveColumns { from, count, to })?;
                id
            }
            Command::MergeCells { id, range } => {
                let t = self.table_mut(&id)?;
                let r = clip(t, &range)?;
                if r.is_single() {
                    return Err(ZError::invalid("merge needs more than one cell"));
                }
                t.merge(r);
                id
            }
            Command::UnmergeCells { id, range } => {
                let t = self.table_mut(&id)?;
                let r = clip(t, &range)?;
                t.unmerge(r);
                id
            }
            Command::FormatCells { id, range, format, clear } => {
                check_format(&format)?;
                let t = self.table_mut(&id)?;
                let r = clip(t, &range)?;
                if clear {
                    clear_formats(t, &r);
                }
                if let Some(n) = &format.number {
                    check_data_format(n)?;
                }
                t.add_format(r, format);
                id
            }
            Command::SetColumn { id, column, width, format } => {
                if width.is_some_and(|w| !(w.is_finite() && w > 0.0 && w <= 1e6)) {
                    return Err(ZError::invalid("width must be > 0"));
                }
                if let Some(f) = &format {
                    check_data_format(f)?;
                }
                let t = self.table_mut(&id)?;
                let Some(col) = t.columns.get_mut(column) else {
                    return Err(ZError::invalid(format!("column {column} is not in the table ({} columns, 0-based)", t.n_cols())));
                };
                if let Some(w) = width {
                    col.width = w;
                }
                if let Some(f) = format {
                    col.format = f;
                }
                id
            }
            Command::SetRow { id, row, height } => {
                if !(height.is_finite() && height > 0.0 && height <= 1e6) {
                    return Err(ZError::invalid("height must be > 0"));
                }
                let t = self.table_mut(&id)?;
                let n = t.n_rows();
                let Some(r) = t.rows.get_mut(row) else {
                    return Err(ZError::invalid(format!("row {row} is not in the table ({n} rows, 0-based)")));
                };
                r.height = height;
                id
            }
            Command::SortTable { id, column, descending } => {
                let t = self.table_mut(&id)?;
                if column >= t.n_cols() {
                    return Err(ZError::invalid(format!("column {column} is not in the table ({} columns, 0-based)", t.n_cols())));
                }
                let body = t.body_rows();
                if t.merges.iter().any(|m| m.rows() > 1 && m.r1 >= body.start && m.r0 < body.end) {
                    return Err(ZError::invalid("can't sort rows that have cells merged across them").hint("unmerge them first"));
                }
                let order = t.sort_order(column, descending);
                self.reshape_table(&id, &Reshape::SortRows { order })?;
                id
            }
            _ => unreachable!("apply_table takes table commands"),
        };
        self.table_settled(&id, &theme);
        // Charts that follow it draw it afresh.
        out.changed.extend(self.charts_on(&id));
        if !out.created.contains(&id) {
            out.changed.push(id);
        }
        Ok(())
    }

    /// Change the shape of table `id` (noted for undo already) and rewrite
    /// its formulas to match.
    ///
    /// The table's formula sheet is not reshaped with it: IronCalc moves
    /// every cell of a sheet one by one (75 ms to insert a row at the top
    /// of 100,000 cells), while sending the moved cells again at the next
    /// recalculation takes 15 ms.
    ///
    /// Undo size: every row whose formulas are rewritten is a changed row
    /// in the undo step. A table with a formula in every row, at the
    /// 100,000-cell cap (4,999 x 20), makes a step of about 1.2 MB for an
    /// insert or delete at the top. That is bounded by the cell cap and
    /// accepted (review of 19a); plain tables make steps of a few KB.
    ///
    /// Ranges of charts linked to it move with their cells; a chart whose
    /// cells all go keeps the values it showed.
    fn reshape_table(&mut self, id: &str, op: &Reshape) -> ZResult<()> {
        if !matches!(self.doc.elements.get(id).map(|e| &e.kind), Some(ElementKind::Table(_))) {
            return Err(ZError::invalid(format!("'{id}' is not a table")));
        }
        // (A refused change is rolled back with the batch, charts too.)
        let moves = self.links_before_reshape(id, op);
        match self.doc.elements.get_mut(id).map(|e| &mut e.kind) {
            Some(ElementKind::Table(t)) => crate::calc::reshape(t, op).map_err(ZError::invalid)?,
            _ => unreachable!("checked above"),
        }
        self.links_moved(moves, &mut Outcome::default());
        Ok(())
    }

    /// The table to change (noted for undo), unlocked.
    fn table_mut(&mut self, id: &str) -> ZResult<&mut TableData> {
        self.check_unlocked(&[id.to_string()])?;
        match &mut self.el_mut(id)?.kind {
            ElementKind::Table(t) => Ok(t),
            k => Err(ZError::invalid(format!("'{id}' is a {}, not a table", k.type_name()))),
        }
    }

    /// After a table changed: formulas worked out again, rows fitted to
    /// their text, the element's size from its rows and columns.
    pub(super) fn table_settled(&mut self, id: &str, theme: &Theme) {
        self.note(id);
        let calc = self.calc.get_mut().unwrap_or_else(|p| p.into_inner());
        let Some(e) = self.doc.elements.get_mut(id) else { return };
        if let ElementKind::Table(t) = &mut e.kind {
            calc.recalc(id, t);
        }
        // Rows that are the very rows they were before the edit keep their
        // measured heights: a cell edit measures one row, not the table.
        let before = self.journal.as_ref().and_then(|j| j.before(id)).and_then(|b| match &b.kind {
            ElementKind::Table(t) => Some(t),
            _ => None,
        });
        measure_table(&*self.measure, theme, e, before);
    }
}

fn clip(t: &TableData, r: &CellRange) -> ZResult<CellRange> {
    r.clip(t.n_rows(), t.n_cols()).ok_or_else(|| ZError::invalid(format!("{} is outside the table ({} rows, {} columns)", r.a1(), t.n_rows(), t.n_cols())))
}

fn check_color(c: &Option<ColorRef>) -> ZResult<()> {
    match c {
        Some(c) if !c.is_empty() && !valid_color(c) => Err(ZError::invalid(format!("bad colour '{c}'")).hint("use #rrggbb, #rrggbbaa, a theme slot, or \"\" for the style's").values(Theme::COLOR_SLOTS.iter().map(|s| s.to_string()).collect())),
        _ => Ok(()),
    }
}

fn check_format(f: &CellFormat) -> ZResult<()> {
    check_color(&f.fill)?;
    check_color(&f.color)?;
    if let Some(b) = &f.border {
        check_color(&b.color)?;
        if b.width.is_some_and(|w| !(w.is_finite() && w >= 0.0 && w <= 1000.0)) {
            return Err(ZError::invalid("border width must be 0 to 1000"));
        }
    }
    Ok(())
}

fn check_data_format(f: &crate::table::DataFormat) -> ZResult<()> {
    if f.decimals.is_some_and(|d| d > 10) {
        return Err(ZError::invalid("decimals must be 0 to 10"));
    }
    if f.currency.chars().count() > 4 {
        return Err(ZError::invalid("currency is a symbol such as £, $ or €"));
    }
    Ok(())
}

/// Take every format off `r`: formats inside it go, formats across its
/// edge keep the part outside.
fn clear_formats(t: &mut TableData, r: &CellRange) {
    let mut out = vec![];
    for rf in std::mem::take(&mut t.cell_formats) {
        let f = rf.range;
        if !f.intersects(r) {
            out.push(rf);
            continue;
        }
        let mut parts = vec![];
        if f.r0 < r.r0 {
            parts.push(CellRange { r1: r.r0 - 1, ..f });
        }
        if f.r1 > r.r1 {
            parts.push(CellRange { r0: r.r1 + 1, ..f });
        }
        let (mr0, mr1) = (f.r0.max(r.r0), f.r1.min(r.r1));
        if f.c0 < r.c0 {
            parts.push(CellRange { r0: mr0, r1: mr1, c1: r.c0 - 1, ..f });
        }
        if f.c1 > r.c1 {
            parts.push(CellRange { r0: mr0, r1: mr1, c0: r.c1 + 1, ..f });
        }
        out.extend(parts.into_iter().map(|range| RangeFormat { range, format: rf.format.clone() }));
    }
    t.cell_formats = out;
}

/// Columns of a table made from data, wide enough for what they hold
/// (between 60% and 250% of the usual width), as pasting into Numbers does.
fn fit_columns(m: &dyn TextMeasure, theme: &Theme, t: &mut TableData) {
    let body = theme.text_style(TextStyle::Body).size;
    let l = t.layout(body);
    let usual = (body * crate::table::COLUMN_EMS).round();
    for c in 0..t.n_cols() {
        let mut widest: f64 = 0.0;
        for r in 0..t.n_rows().min(200) {
            if t.merge_at(r, c).is_some() {
                continue;
            }
            let s = t.shown(r, c, None);
            let longest = s.text.split('\n').max_by_key(|l| l.chars().count()).unwrap_or("");
            if longest.is_empty() {
                continue;
            }
            let td = TextData { text: crate::table::drawn_text(longest).into_owned(), bold: t.bold_at(r, c), ..Default::default() };
            widest = widest.max(m.measure(&td, theme, None).0 * t.font_size);
        }
        let w = (widest + 2.0 * l.pad_x + 2.0).ceil();
        t.columns[c].width = w.clamp((usual * 0.6).round(), (usual * 2.5).round());
    }
}

/// Fit a table's rows to their text and its box to its rows and columns.
///
/// With `prev` (the table before an edit, same theme): rows that are the
/// same rows in memory as there keep the height measured then, when the
/// columns and everything that changes how text sets (merges, header
/// rows, formats, text size) are the same.
pub(super) fn measure_table(m: &dyn TextMeasure, theme: &Theme, e: &mut Element, prev: Option<&TableData>) {
    let ElementKind::Table(t) = &mut e.kind else { return };
    let reuse: std::collections::HashMap<u64, (&crate::table::CellRow, f64, f64)> = match prev {
        Some(p) if p.same_meta(t) && p.columns.len() == t.columns.len() && p.columns.iter().zip(&t.columns).all(|(a, b)| a.width == b.width) => {
            p.cells.iter().zip(&p.rows).map(|(c, r)| (c.id(), (c, r.fit, r.height))).collect()
        }
        _ => Default::default(),
    };
    let body = theme.text_style(TextStyle::Body).size;
    let l = t.layout(body);
    let fs = t.font_size;
    let line = |bold: bool| m.measure(&TextData { text: "Ag".into(), bold, ..Default::default() }, theme, None).1 * fs;
    let (line_r, line_b) = (line(false), line(true));
    // A character is rarely wider than this many ems: text shorter than
    // the column by that measure fits on one line without measuring it.
    let em_max = 0.62 * l.font;
    let n = t.n_rows();
    let mut fits = vec![0.0f64; n];
    for (r, fit) in fits.iter_mut().enumerate() {
        if let Some((row, f, h)) = reuse.get(&t.cells[r].id()) {
            // The same row, or one whose formulas were only rewritten
            // (references moved by an insert above): it shows the same.
            let shows_same = || {
                row.len() == t.cells[r].len()
                    && row.iter().zip(t.cells[r].iter()).all(|(a, b)| {
                        if a.is_formula() && b.is_formula() { a.value == b.value && a.format == b.format } else { a == b }
                    })
            };
            if *h == t.rows[r].height && (row.same(&t.cells[r]) || shows_same()) {
                *fit = *f;
                continue;
            }
        }
        let mut need: f64 = 0.0;
        for c in 0..t.n_cols() {
            let span = t.span(r, c);
            if (span.r0, span.c0) != (r, c) || span.rows() > 1 {
                continue; // drawn by its anchor; tall merges don't grow rows
            }
            let cell = &t.cells[r][c];
            if cell.is_empty() {
                continue;
            }
            // A column's look only changes how numbers show; numbers
            // take one line whatever their look.
            let s = t.shown(r, c, None);
            let bold = t.bold_at(r, c);
            let lh = if bold { line_b } else { line_r };
            if s.numeric || s.tick.is_some() || s.error.is_some() {
                need = need.max(lh);
                continue;
            }
            let avail = (l.xs[span.c1 + 1] - l.xs[span.c0] - 2.0 * l.pad_x).max(1.0);
            let h = if !s.text.contains('\n') && s.text.chars().count() as f64 * em_max <= avail {
                lh
            } else {
                let italic = t.format_at(r, c).italic.unwrap_or(false);
                let td = TextData { text: crate::table::drawn_text(&s.text).into_owned(), bold, italic, fixed_width: true, ..Default::default() };
                m.measure(&td, theme, Some(avail / fs)).1 * fs
            };
            need = need.max(h);
        }
        let need = if need > 0.0 { (need + 2.0 * l.pad_y).ceil() } else { 0.0 };
        *fit = if need > t.rows[r].height { need } else { 0.0 };
    }
    for (row, fit) in t.rows.iter_mut().zip(fits) {
        if row.fit != fit {
            row.fit = fit;
        }
    }
    let (w, h) = t.size(body);
    e.size = Size { w, h };
}
