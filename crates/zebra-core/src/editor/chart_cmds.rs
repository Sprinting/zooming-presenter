//! The chart commands: adding charts, typing into their data, formatting,
//! linking to tables. And keeping links whole: a linked range moves when
//! its table changes shape, and a chart whose table goes (deleted, or not
//! copied with it) keeps the values it showed, in the same undo step.
use super::*;
use crate::calc::Reshape;
use crate::chart::{self, ChartData, ChartLink, ChartType};
use crate::table::{CellInput, CellRange, Limit, TableData};

fn too_big(l: Limit) -> ZError {
    ZError::new("too_big", l.message()).hint("nothing was changed")
}

fn inputs(rows: Vec<Vec<CellInput>>) -> ZResult<Vec<Vec<String>>> {
    let rows: Vec<Vec<String>> = rows.into_iter().map(|r| r.into_iter().map(CellInput::into_input).collect()).collect();
    for s in rows.iter().flatten() {
        if let Some(l) = Limit::of_input(s) {
            return Err(too_big(l));
        }
    }
    Ok(rows)
}

fn grid_size(rows: &[Vec<String>]) -> (usize, usize) {
    (rows.len(), rows.iter().map(|r| r.len()).max().unwrap_or(0))
}

fn check_fraction(name: &str, v: Option<f64>, lo: f64, hi: f64) -> ZResult<()> {
    match v {
        Some(v) if !(v.is_finite() && v >= lo && v <= hi) => Err(ZError::invalid(format!("{name} is {lo} to {hi}"))),
        _ => Ok(()),
    }
}

impl Editor {
    pub(super) fn apply_chart(&mut self, cmd: Command, out: &mut Outcome) -> ZResult<()> {
        let theme = self.theme();
        let body = theme.text_style(TextStyle::Body).size;
        let id = match cmd {
            Command::AddChart { chart_type, look, data, csv, from_table, range, link, series_in, title, x, y, width, height, scale, place, name, id } => {
                let ty = chart_type.unwrap_or_default();
                let given = match (data, csv) {
                    (Some(_), Some(_)) => return Err(ZError::invalid("give `data` or `csv`, not both")),
                    (Some(d), None) => Some(inputs(d)?),
                    (None, Some(s)) => Some(inputs(crate::table::parse_delimited(&s).map_err(too_big)?.into_iter().map(|r| r.into_iter().map(CellInput::Text).collect()).collect())?),
                    (None, None) => None,
                };
                if given.is_some() && from_table.is_some() {
                    return Err(ZError::invalid("give `data`/`csv` or `from_table`, not both"));
                }
                if from_table.is_none() && (link || range.is_some()) {
                    return Err(ZError::invalid("`link` and `range` go with `from_table`").hint("from_table: the id of a table (b1...)"));
                }
                for (n, v) in [("width", width), ("height", height)] {
                    if v.is_some_and(|v| !(v.is_finite() && v > 0.0 && v <= 1e6)) {
                        return Err(ZError::invalid(format!("{n} must be > 0")));
                    }
                }
                let mut c = match (&given, &from_table) {
                    (Some(g), _) => {
                        let (nr, nc) = grid_size(g);
                        if nr == 0 || nc == 0 {
                            return Err(ZError::invalid("data is empty").hint("rows of values: [[\"\", \"Q2\"], [\"North\", 1240]]"));
                        }
                        if let Some(l) = chart::over_limit(nr, nc) {
                            return Err(too_big(l));
                        }
                        ChartData::new(ty, g, body)
                    }
                    (None, Some(t)) => {
                        let table = self.linkable(t, range.as_ref())?;
                        let mut c = ChartData::new(ty, &[vec![String::new()]], body);
                        c.link = Some(ChartLink { table: t.clone(), range: range.map(|r| r.clip(table.n_rows(), table.n_cols()).unwrap_or(r)) });
                        if !link {
                            if let Some(st) = &series_in {
                                c.series_in = *st;
                            }
                            c.bake(&self.doc, body);
                        }
                        c
                    }
                    (None, None) => ChartData::sample(ty, body),
                };
                c.chart_type = ty;
                if let Some(l) = look {
                    c.look = l;
                }
                if let Some(s) = series_in {
                    c.series_in = s;
                }
                if let Some(t) = title {
                    c.title = t;
                }
                if ty == ChartType::Waterfall && c.totals.is_empty() {
                    c.totals = default_totals(&c, &self.doc);
                }
                let mut e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform { x, y, scale: scale.unwrap_or(1.0), rotation: 0.0 },
                    size: Size { w: width.unwrap_or(chart::DEFAULT_W), h: height.unwrap_or(chart::DEFAULT_H) },
                    locked: false,
                    name: name.unwrap_or_default(),
                    placeholder: false,
                    group: None,
                    kind: ElementKind::Chart(c),
                };
                if let Some(p) = place {
                    self.place(&mut e, &p, scale.is_none())?;
                }
                return self.insert_element(e, out).map(|_| ());
            }
            Command::SetChartData { id, at, values, data, rows, columns } => {
                let given = match (values, data) {
                    (Some(_), Some(_)) => return Err(ZError::invalid("give `values` or `data`, not both")),
                    (Some(v), None) => Some((false, inputs(v)?)),
                    (None, Some(d)) => Some((true, inputs(d)?)),
                    (None, None) => None,
                };
                if given.is_none() && rows.is_none() && columns.is_none() {
                    return Err(ZError::invalid("nothing to change").hint("give `values` (from `at`), `data`, or `rows`/`columns`"));
                }
                let (r0, c0) = crate::table::parse_cell(&at).ok_or_else(|| ZError::invalid(format!("'{at}' is not a cell like B2")))?;
                let c = self.chart_mut(&id)?;
                if c.link.is_some() {
                    return Err(ZError::new("linked", format!("'{id}' draws a table's cells")).hint("change the table, or unlink_chart first"));
                }
                let t = &mut c.data;
                let (mut nr, mut nc) = (t.n_rows(), t.n_cols());
                match &given {
                    Some((true, g)) => (nr, nc) = grid_size(g),
                    Some((false, g)) => {
                        let (gr, gc) = grid_size(g);
                        if gr == 0 || gc == 0 {
                            return Err(ZError::invalid("values is empty").hint("rows of values: [[\"North\", 1240]]"));
                        }
                        nr = nr.max(r0.saturating_add(gr));
                        nc = nc.max(c0.saturating_add(gc));
                    }
                    None => {}
                }
                let nr = rows.unwrap_or(nr);
                let nc = columns.unwrap_or(nc);
                if nr == 0 || nc == 0 {
                    return Err(ZError::invalid("a chart's data keeps at least one row and one column"));
                }
                // Only growing past the limits is refused: data a file
                // brought in bigger stays editable (B2).
                if nr > t.n_rows() || nc > t.n_cols() {
                    if let Some(l) = chart::over_limit(nr, nc) {
                        return Err(too_big(l));
                    }
                }
                if let Some((true, g)) = &given {
                    let mut fresh = TableData::new(nr, nc, t.columns.first().map_or(24.0, |c| c.width / crate::table::COLUMN_EMS));
                    fresh.header_columns = t.header_columns.min(nc as u8);
                    fresh.header_rows = t.header_rows.min(nr as u8);
                    for (r, row) in g.iter().enumerate() {
                        for (cc, v) in row.iter().enumerate() {
                            fresh.cells[r][cc] = crate::table::Cell::new(v.clone());
                        }
                    }
                    *t = fresh;
                } else {
                    resize(t, nr, nc)?;
                    if let Some((false, g)) = given {
                        for (dr, row) in g.into_iter().enumerate() {
                            for (dc, input) in row.into_iter().enumerate() {
                                let (r, cc) = (r0 + dr, c0 + dc);
                                if r < t.n_rows() && cc < t.n_cols() && t.cells[r][cc].input != input {
                                    t.cells[r][cc] = crate::table::Cell::new(input);
                                }
                            }
                        }
                    }
                }
                let n = c.data.n_rows();
                c.totals.retain(|i| *i + 1 < n);
                id
            }
            Command::FormatChart { id, format: f } => {
                check_fraction("font_size", f.font_size, 0.25, 4.0)?;
                check_fraction("gap", f.gap, 0.0, 1.0)?;
                check_fraction("gap_sets", f.gap_sets, 0.0, 5.0)?;
                check_fraction("hole", f.hole, 0.1, 0.9)?;
                for (n, v) in [("axis_min", f.axis_min), ("axis_max", f.axis_max)] {
                    if v.and_then(AxisValue::get).is_some_and(|v| !v.is_finite()) {
                        return Err(ZError::invalid(format!("{n} must be a finite number or \"auto\"")));
                    }
                }
                if f.axis_step.and_then(AxisValue::get).is_some_and(|v| !(v.is_finite() && v > 0.0)) {
                    return Err(ZError::invalid("axis_step must be > 0 or \"auto\""));
                }
                if let Some(n) = &f.number {
                    if n.decimals.is_some_and(|d| d > 10) {
                        return Err(ZError::invalid("decimals is 0 to 10"));
                    }
                    if n.currency.chars().count() > 4 {
                        return Err(ZError::invalid("currency is a symbol such as £, $ or €"));
                    }
                }
                for s in &f.series {
                    if s.index >= chart::MAX_SERIES {
                        return Err(ZError::invalid(format!("series index is 0 to {}", chart::MAX_SERIES - 1)));
                    }
                    if let Some(c) = s.color.as_deref().filter(|c| !c.is_empty()) {
                        if !valid_color(c) {
                            return Err(ZError::invalid(format!("bad colour '{c}'")).hint("use #rrggbb, a theme slot, or \"\" for the theme's").values(Theme::COLOR_SLOTS.iter().map(|s| s.to_string()).collect()));
                        }
                    }
                    if s.line_width.is_some_and(|w| !(w.is_finite() && (0.0..=100.0).contains(&w))) {
                        return Err(ZError::invalid("line_width is 0 (the look's) to 100"));
                    }
                }
                if let Some(t) = &f.totals {
                    if t.iter().any(|i| *i >= chart::MAX_CATEGORIES) {
                        return Err(ZError::invalid(format!("totals are category indexes below {}", chart::MAX_CATEGORIES)));
                    }
                }
                for t in [&f.title, &f.x_title, &f.y_title].into_iter().flatten() {
                    if t.chars().count() > crate::table::MAX_INPUT {
                        return Err(too_big(Limit::Input));
                    }
                }
                let to_waterfall = f.chart_type == Some(ChartType::Waterfall);
                let totals_given = f.totals.is_some();
                let c = self.chart_mut(&id)?;
                let was_waterfall = c.chart_type == ChartType::Waterfall;
                macro_rules! set {
                    ($($f:ident),*) => { $(if let Some(v) = f.$f { c.$f = v; })* };
                }
                set!(chart_type, look, series_in, title, legend, border, font_size, value_labels, gridlines, gap, gap_sets, hole, number, x_title, y_title, totals);
                if let Some(v) = f.axis_min {
                    c.axis_min = v.get();
                }
                if let Some(v) = f.axis_max {
                    c.axis_max = v.get();
                }
                if let Some(v) = f.axis_step {
                    c.axis_step = v.get();
                }
                if let (Some(a), Some(b)) = (c.axis_min, c.axis_max) {
                    if a >= b {
                        return Err(ZError::invalid(format!("axis_min ({a}) must be below axis_max ({b})")));
                    }
                }
                for s in f.series {
                    if c.series.len() <= s.index {
                        c.series.resize(s.index + 1, Default::default());
                    }
                    let to = &mut c.series[s.index];
                    if let Some(col) = s.color {
                        to.color = (!col.is_empty()).then_some(col);
                    }
                    if let Some(w) = s.line_width {
                        to.line_width = (w > 0.0).then_some(w);
                    }
                    if s.labels.is_some() {
                        to.labels = s.labels;
                    }
                    if s.markers.is_some() {
                        to.markers = s.markers;
                    }
                    if let Some(v) = s.smooth {
                        to.smooth = v;
                    }
                    if s.as_line.is_some() {
                        to.as_line = s.as_line;
                    }
                }
                while c.series.last().is_some_and(|s| s.is_empty()) {
                    c.series.pop();
                }
                c.totals.sort_unstable();
                c.totals.dedup();
                let need_totals = to_waterfall && !was_waterfall && !totals_given && c.totals.is_empty();
                if need_totals {
                    if let Some(ElementKind::Chart(c)) = self.doc.elements.get(&id).map(|e| &e.kind) {
                        let t = default_totals(c, &self.doc);
                        if let ElementKind::Chart(c) = &mut self.el_mut(&id)?.kind {
                            c.totals = t;
                        }
                    }
                }
                id
            }
            Command::LinkChart { id, table, range } => {
                if table == id {
                    return Err(ZError::invalid("a chart links to a table, not to itself"));
                }
                let range = {
                    let t = self.linkable(&table, range.as_ref())?;
                    range.map(|r| r.clip(t.n_rows(), t.n_cols()).unwrap_or(r))
                };
                let c = self.chart_mut(&id)?;
                c.link = Some(ChartLink { table, range });
                id
            }
            Command::UnlinkChart { id } => {
                self.check_unlocked(std::slice::from_ref(&id))?;
                let doc = &self.doc;
                let Some(ElementKind::Chart(c)) = doc.elements.get(&id).map(|e| &e.kind) else {
                    return Err(self.not_a_chart(&id));
                };
                if c.link.is_none() {
                    return Err(ZError::invalid(format!("'{id}' is not linked to a table")));
                }
                let mut c = c.clone();
                c.bake(doc, body);
                self.note(&id);
                if let Some(ElementKind::Chart(to)) = self.doc.elements.get_mut(&id).map(|e| &mut e.kind) {
                    *to = c;
                }
                id
            }
            _ => unreachable!("apply_chart takes chart commands"),
        };
        self.chart_settled(&id);
        if !out.created.contains(&id) {
            out.changed.push(id);
        }
        Ok(())
    }

    fn not_a_chart(&self, id: &str) -> ZError {
        match self.doc.elements.get(id) {
            Some(e) => ZError::invalid(format!("'{id}' is a {}, not a chart", e.kind.type_name())),
            None => ZError::not_found(id, self.doc.elements.keys().cloned()),
        }
    }

    /// The chart to change (noted for undo), unlocked.
    fn chart_mut(&mut self, id: &str) -> ZResult<&mut ChartData> {
        if !matches!(self.doc.elements.get(id).map(|e| &e.kind), Some(ElementKind::Chart(_))) {
            return Err(self.not_a_chart(id));
        }
        self.check_unlocked(&[id.to_string()])?;
        match &mut self.el_mut(id)?.kind {
            ElementKind::Chart(c) => Ok(c),
            _ => unreachable!("checked above"),
        }
    }

    /// Table `id`, which a chart can draw `range` of.
    fn linkable(&self, id: &str, range: Option<&CellRange>) -> ZResult<&TableData> {
        let t = match self.doc.elements.get(id).map(|e| &e.kind) {
            Some(ElementKind::Table(t)) => t,
            Some(k) => return Err(ZError::invalid(format!("'{id}' is a {}, not a table", k.type_name())).hint("charts draw from tables (b1...)")),
            None => return Err(ZError::not_found(id, self.doc.elements.keys().filter(|k| k.starts_with('b')).cloned())),
        };
        if let Some(r) = range {
            if r.clip(t.n_rows(), t.n_cols()).is_none() {
                return Err(ZError::invalid(format!("{} is outside the table ({} rows, {} columns)", r.a1(), t.n_rows(), t.n_cols())));
            }
        }
        Ok(t)
    }

    /// After a chart changed: the formulas of its own data worked out.
    pub(super) fn chart_settled(&mut self, id: &str) {
        let calc = self.calc.get_mut().unwrap_or_else(|p| p.into_inner());
        if let Some(ElementKind::Chart(c)) = self.doc.elements.get_mut(id).map(|e| &mut e.kind) {
            if c.link.is_none() || crate::calc::has_formulas(&c.data) {
                calc.recalc(id, &mut c.data);
            }
        }
    }

    /// The charts that follow table `table`.
    pub(super) fn charts_on(&self, table: &str) -> Vec<ElementId> {
        self.doc
            .elements
            .values()
            .filter(|e| matches!(&e.kind, ElementKind::Chart(c) if c.linked_table() == Some(table)))
            .map(|e| e.id.clone())
            .collect()
    }

    /// Before table `table` changes shape by `op`: where each linked range
    /// goes, and the charts whose cells all go (they keep their values now,
    /// before the cells are gone). Apply with [`Editor::links_moved`].
    pub(super) fn links_before_reshape(&mut self, table: &str, op: &Reshape) -> Vec<(ElementId, CellRange)> {
        let Some(ElementKind::Table(t)) = self.doc.elements.get(table).map(|e| &e.kind) else { return vec![] };
        let (rows, cols) = (t.n_rows(), t.n_cols());
        let mut moves = vec![];
        let mut lost = vec![];
        for id in self.charts_on(table) {
            let Some(ElementKind::Chart(c)) = self.doc.elements.get(&id).map(|e| &e.kind) else { continue };
            let Some(r) = c.link.as_ref().and_then(|l| l.range) else { continue };
            match chart::move_range(r, op, rows, cols) {
                Some(n) if n != r => moves.push((id, n)),
                Some(_) => {}
                None => lost.push(id),
            }
        }
        for id in lost {
            self.bake_chart(&id);
        }
        moves
    }

    pub(super) fn links_moved(&mut self, moves: Vec<(ElementId, CellRange)>, out: &mut Outcome) {
        for (id, r) in moves {
            self.note(&id);
            if let Some(ElementKind::Chart(c)) = self.doc.elements.get_mut(&id).map(|e| &mut e.kind) {
                if let Some(l) = &mut c.link {
                    l.range = Some(r);
                }
            }
            out.changed.push(id);
        }
    }

    /// Unlink chart `id`, keeping the values it shows (noted for undo).
    fn bake_chart(&mut self, id: &str) {
        let body = self.theme().text_style(TextStyle::Body).size;
        let Some(ElementKind::Chart(c)) = self.doc.elements.get(id).map(|e| &e.kind) else { return };
        let mut c = c.clone();
        c.bake(&self.doc, body);
        self.note(id);
        if let Some(ElementKind::Chart(to)) = self.doc.elements.get_mut(id).map(|e| &mut e.kind) {
            *to = c;
        }
        self.chart_settled(id);
    }

    /// Before tables `gone` are deleted: the charts that follow them (and
    /// stay) keep the values they show. Returns them.
    pub(super) fn unlink_from_deleted(&mut self, gone: &HashSet<&ElementId>) -> Vec<ElementId> {
        let hit: Vec<ElementId> = self
            .doc
            .elements
            .values()
            .filter(|e| !gone.contains(&e.id))
            .filter(|e| matches!(&e.kind, ElementKind::Chart(c) if c.link.as_ref().is_some_and(|l| gone.contains(&l.table))))
            .map(|e| e.id.clone())
            .collect();
        for id in &hit {
            self.bake_chart(id);
        }
        hit
    }
}

/// A new waterfall's totals: its last point.
fn default_totals(c: &ChartData, doc: &Document) -> Vec<usize> {
    let n = c.plot(doc).categories.len();
    if n >= 2 { vec![n - 1] } else { vec![] }
}

/// Grow or cut a chart's own data to `nr` x `nc`, rewriting formulas.
fn resize(t: &mut TableData, nr: usize, nc: usize) -> ZResult<()> {
    let (r, c) = (t.n_rows(), t.n_cols());
    let ops = [
        (nc > c).then(|| Reshape::InsertColumns { at: c, count: nc - c }),
        (nc < c).then(|| Reshape::DeleteColumns { at: nc, count: c - nc }),
        (nr > r).then(|| Reshape::InsertRows { at: r, count: nr - r }),
        (nr < r).then(|| Reshape::DeleteRows { at: nr, count: r - nr }),
    ];
    for op in ops.into_iter().flatten() {
        crate::calc::reshape(t, &op).map_err(ZError::invalid)?;
    }
    // Rows always need at least one header for the names; footers don't
    // apply to a chart's own data.
    t.footer_rows = 0;
    t.header_rows = t.header_rows.min(t.n_rows() as u8);
    t.header_columns = t.header_columns.min(t.n_cols() as u8);
    Ok(())
}
