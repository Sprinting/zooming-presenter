//! Tables on the canvas and in the inspector.
//!
//! Three levels of selection, as in Keynote and Numbers: one click selects
//! the table as an object (moved, scaled and turned with the zebra
//! transform tool); a second click selects a cell, and column letters and
//! row numbers appear around the table; typing, double-clicking or Return
//! edits the cell. Esc goes back up a level at a time.
//!
//! Every change goes through zebra-core table commands, one undo step each.
use std::ops::Range;

use egui::{Align2, Color32, Key, Modifiers, Painter, Pos2, Rect as ERect, Shape, Stroke, Ui};
use zebra_core::table::{self, Border, BorderEdges, CellFormat, CellInput, CellRange, CellType, DataFormat, Gridlines, Limit, LineStyle, Negative, TableData, TableStyle, VAlign};
use zebra_core::{Align, Command, Element, ElementId, ElementKind, ElementPatch, Origin, TextStyle, Vec2};

use crate::actions::Action;
use crate::app::ZebraApp;
use crate::inspector::{Insp, Tab};
use crate::style::{self, Palette};
use crate::widgets::{self as w, Num, Seg};

const H: Origin = Origin::Human;

/// Height of the column-letter bar and width of the row-number bar (pt).
const BAR: f32 = 18.0;
const ROW_BAR: f32 = 26.0;
/// The round add handles' radius (pt).
const KNOB: f32 = 7.0;

/// Cells selected in a table (the second level), and the cell being
/// edited (the third).
#[derive(Debug, Clone)]
pub struct TableSel {
    pub id: ElementId,
    pub anchor: (usize, usize),
    pub head: (usize, usize),
    pub edit: Option<CellEdit>,
    pub drag: Option<Drag>,
}

#[derive(Debug, Clone)]
pub struct CellEdit {
    pub at: (usize, usize),
    pub text: String,
    pub focused: bool,
}

/// What a drag that began on the table does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drag {
    /// Selecting a range of cells.
    Cells,
    /// A column letter dragged to a new place.
    Column { from: usize, to: usize },
    Row { from: usize, to: usize },
    /// The handle at the right edge: columns added (or taken away).
    AddColumns { delta: i64 },
    AddRows { delta: i64 },
}

impl TableSel {
    pub fn range(&self) -> CellRange {
        let (a, b) = (self.anchor, self.head);
        CellRange::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
    }
}

fn table<'a>(app: &'a ZebraApp, id: &str) -> Option<(&'a Element, &'a TableData)> {
    let e = app.editor.doc.get(id)?;
    match &e.kind {
        ElementKind::Table(t) => Some((e, t)),
        _ => None,
    }
}

fn body_size(app: &ZebraApp) -> f64 {
    app.editor.theme().text_style(TextStyle::Body).size
}

/// Table units (from its top-left) to the screen.
fn screen(app: &ZebraApp, e: &Element, x: f64, y: f64) -> Pos2 {
    app.canvas.w2s(&app.editor, e.transform.apply(Vec2::new(x - e.size.w / 2.0, y - e.size.h / 2.0)))
}

/// The screen to table units.
fn local(app: &ZebraApp, e: &Element, p: Pos2) -> (f64, f64) {
    let l = e.transform.inverse(app.canvas.s2w(&app.editor, p));
    (l.x + e.size.w / 2.0, l.y + e.size.h / 2.0)
}

fn quad(app: &ZebraApp, e: &Element, x0: f64, y0: f64, x1: f64, y1: f64) -> [Pos2; 4] {
    [screen(app, e, x0, y0), screen(app, e, x1, y0), screen(app, e, x1, y1), screen(app, e, x0, y1)]
}

/// Screen points per table unit.
fn px_per_unit(app: &ZebraApp, e: &Element) -> f32 {
    screen(app, e, 0.0, 0.0).distance(screen(app, e, 1.0, 0.0))
}

/// The middle of cell (r, c) on screen.
pub fn cell_center(app: &ZebraApp, id: &str, r: usize, c: usize) -> Option<Pos2> {
    let (e, t) = table(app, id)?;
    let l = t.layout(body_size(app));
    if r >= t.n_rows() || c >= t.n_cols() {
        return None;
    }
    Some(screen(app, e, (l.xs[c] + l.xs[c + 1]) / 2.0, (l.ys[r] + l.ys[r + 1]) / 2.0))
}

/// The cell under a screen point, if it is on the table's cells.
pub fn cell_at(app: &ZebraApp, id: &str, p: Pos2) -> Option<(usize, usize)> {
    let (e, t) = table(app, id)?;
    let l = t.layout(body_size(app));
    let (x, y) = local(app, e, p);
    if x < 0.0 || x > l.width() || y < l.caption || y > l.height() {
        return None;
    }
    Some((l.row_at(y), l.col_at(x)))
}

/// Keep the cell selection in step with the document: gone when the
/// table is no longer the one thing selected, clamped when it shrank.
pub fn sync(app: &mut ZebraApp) {
    let Some(s) = &app.canvas.table else { return };
    let ok = app.editor.view.selection.len() == 1 && app.editor.view.selection[0] == s.id;
    let size = table(app, &s.id).map(|(_, t)| (t.n_rows(), t.n_cols()));
    match (ok, size) {
        (true, Some((n, m))) if n > 0 && m > 0 => {
            if let Some(s) = &mut app.canvas.table {
                s.anchor = (s.anchor.0.min(n - 1), s.anchor.1.min(m - 1));
                s.head = (s.head.0.min(n - 1), s.head.1.min(m - 1));
            }
        }
        _ => app.canvas.table = None,
    }
}

/// Cells selected (the second or third level) in the one selected table.
pub fn active(app: &ZebraApp) -> bool {
    app.canvas.table.is_some()
}

pub fn editing(app: &ZebraApp) -> bool {
    app.canvas.table.as_ref().is_some_and(|s| s.edit.is_some())
}

fn select_cell(app: &mut ZebraApp, id: &str, rc: (usize, usize), extend: bool) {
    let rc = table(app, id).map(|(_, t)| t.anchor(rc.0, rc.1)).unwrap_or(rc);
    match &mut app.canvas.table {
        Some(s) if s.id == id && extend => s.head = rc,
        _ => app.canvas.table = Some(TableSel { id: id.to_string(), anchor: rc, head: rc, edit: None, drag: None }),
    }
}

// ----------------------------------------------------------------------
// Inserting

/// A new table of the default size in the middle of the view.
pub fn insert_table(app: &mut ZebraApp, style: TableStyle) {
    let at = app.canvas_center_world();
    let zoom = app.editor.view.camera.zoom.max(1e-9);
    let body = body_size(app);
    let natural_w = 4.0 * (body * table::COLUMN_EMS).round();
    let vis_w = app.editor.view.viewport.x / zoom;
    let scale = (1.0 / zoom).min(0.6 * vis_w / natural_w);
    let cmd = Command::AddTable {
        rows: Some(5),
        columns: Some(4),
        cells: None,
        csv: None,
        x: at.x,
        y: at.y,
        style: Some(style),
        header_rows: None,
        // Side Header is about its first column: it gets one.
        header_columns: (style == TableStyle::SideHeader).then_some(1),
        footer_rows: None,
        scale: Some(scale),
        place: None,
        name: None,
        id: None,
    };
    if let Some(out) = app.apply(cmd) {
        app.select(out.created);
    }
}

/// The message for something over a table's limits.
pub fn limit_message(l: Limit) -> String {
    match l {
        Limit::Rows => crate::t!("table-limit-rows", count = table::MAX_ROWS),
        Limit::Columns => crate::t!("table-limit-columns", count = table::MAX_COLUMNS),
        Limit::Cells => crate::t!("table-limit-cells", count = table::MAX_CELLS),
        Limit::Input => crate::t!("table-limit-input", count = table::MAX_INPUT),
        Limit::Formula => crate::t!("table-limit-formula", count = table::MAX_FORMULA),
        Limit::Bytes => {
            let mb = table::MAX_PASTE_BYTES >> 20;
            crate::t!("table-limit-paste", mb = mb)
        }
    }
}

/// Text from Numbers or Excel pasted on the empty canvas: a new table.
/// Returns false when the text is not a table.
pub fn paste_as_table(app: &mut ZebraApp, text: &str) -> bool {
    if !table::looks_tabular(text) {
        return false;
    }
    // Bounded before it becomes a grid (the core checks again).
    if let Err(l) = table::parse_delimited(text) {
        app.note(limit_message(l));
        return true;
    }
    let at = app.canvas_center_world();
    let zoom = app.editor.view.camera.zoom.max(1e-9);
    let cmd = Command::AddTable {
        rows: None,
        columns: None,
        cells: None,
        csv: Some(text.to_string()),
        x: at.x,
        y: at.y,
        style: None,
        header_rows: None,
        header_columns: None,
        footer_rows: None,
        scale: Some(1.0 / zoom),
        place: None,
        name: None,
        id: None,
    };
    if let Some(out) = app.apply(cmd) {
        app.select(out.created);
    }
    true
}

// ----------------------------------------------------------------------
// Canvas: clicks, drags, keys

/// A click on the canvas. Returns true when the table took it.
pub fn click(app: &mut ZebraApp, p: Pos2, shift: bool) -> bool {
    // A click on another cell while writing a formula adds a reference.
    if let Some(s) = app.canvas.table.clone() {
        if let Some(edit) = &s.edit {
            if edit.text.starts_with('=') {
                if let Some(rc) = cell_at(app, &s.id, p) {
                    if rc != edit.at {
                        if let Some(e) = app.canvas.table.as_mut().and_then(|s| s.edit.as_mut()) {
                            e.text.push_str(&table::cell_name(rc.0, rc.1));
                            e.focused = false;
                        }
                        return true;
                    }
                }
            }
            commit_edit(app, None);
        }
        if let Some(h) = knob_at(app, p) {
            add_from_knob(app, h, 1);
            return true;
        }
        if let Some(rc) = cell_at(app, &s.id, p) {
            select_cell(app, &s.id, rc, shift);
            return true;
        }
        app.canvas.table = None;
        return false;
    }
    // The second click on a selected table selects a cell.
    let sel = app.selection();
    if let [id] = sel.as_slice() {
        if let Some(rc) = cell_at(app, id, p) {
            select_cell(app, &id.clone(), rc, false);
            return true;
        }
    }
    false
}

/// A double click on a table edits the cell under it.
pub fn double_click(app: &mut ZebraApp, p: Pos2) -> bool {
    // Two quick clicks while writing a formula are two references (the
    // clicks added them), not an edit of the cell clicked.
    if app.canvas.table.as_ref().and_then(|s| s.edit.as_ref()).is_some_and(|e| e.text.starts_with('=')) {
        return true;
    }
    let Some(id) = crate::canvas::hit(app, p, false) else { return false };
    let Some(rc) = cell_at(app, &id, p) else { return false };
    if app.selection() != vec![id.clone()] {
        app.select(vec![id.clone()]);
    }
    select_cell(app, &id, rc, false);
    start_edit(app, None);
    true
}

/// The right-click menu's cell: the cell under the pointer joins the
/// selection levels as a click would.
pub fn secondary_click(app: &mut ZebraApp, p: Pos2) {
    let Some(id) = crate::canvas::hit(app, p, false) else { return };
    let Some(rc) = cell_at(app, &id, p) else { return };
    if let Some(s) = &app.canvas.table {
        if s.id == id && s.range().contains(rc.0, rc.1) {
            return;
        }
    }
    commit_edit(app, None);
    if app.selection() != vec![id.clone()] {
        app.select(vec![id.clone()]);
    }
    select_cell(app, &id, rc, false);
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Knob {
    Right,
    Bottom,
}

/// Where the round handles sit, on screen.
fn knobs(app: &ZebraApp, s: &TableSel) -> Option<[(Knob, Pos2); 2]> {
    let (e, t) = table(app, &s.id)?;
    let l = t.layout(body_size(app));
    let k = px_per_unit(app, e).max(1e-6) as f64;
    let off = (KNOB as f64 + 6.0) / k;
    let my = (l.caption + l.height()) / 2.0;
    Some([(Knob::Right, screen(app, e, l.width() + off, my)), (Knob::Bottom, screen(app, e, l.width() / 2.0, l.height() + off))])
}

fn knob_at(app: &ZebraApp, p: Pos2) -> Option<Knob> {
    let s = app.canvas.table.as_ref()?;
    knobs(app, s)?.into_iter().find(|(_, c)| c.distance(p) <= KNOB + 3.0).map(|(k, _)| k)
}

fn add_from_knob(app: &mut ZebraApp, k: Knob, n: i64) {
    let Some(s) = app.canvas.table.clone() else { return };
    let Some((_, t)) = table(app, &s.id) else { return };
    let (rows, cols) = (t.n_rows(), t.n_cols());
    let cmd = match (k, n) {
        (_, 0) => return,
        (Knob::Right, n) if n > 0 => Command::InsertColumns { id: s.id.clone(), index: cols, count: n as usize },
        (Knob::Right, n) => {
            let c = ((-n) as usize).min(cols - 1);
            if c == 0 {
                return;
            }
            Command::DeleteColumns { id: s.id.clone(), index: cols - c, count: c }
        }
        (Knob::Bottom, n) if n > 0 => Command::InsertRows { id: s.id.clone(), index: rows, count: n as usize },
        (Knob::Bottom, n) => {
            let c = ((-n) as usize).min(rows - 1);
            if c == 0 {
                return;
            }
            Command::DeleteRows { id: s.id.clone(), index: rows - c, count: c }
        }
    };
    apply_or_note(app, cmd);
    sync(app);
}

/// Apply a table command; a refusal over the limits says which, in words.
fn apply_or_note(app: &mut ZebraApp, cmd: Command) -> bool {
    match app.editor.apply(cmd, H) {
        Ok(_) => true,
        Err(e) if e.code == "too_big" => {
            app.note(crate::t!("table-too-big"));
            false
        }
        Err(e) => {
            app.report::<()>(Err(e));
            false
        }
    }
}

/// Column letter or row number bar under a screen point.
fn bar_at(app: &ZebraApp, p: Pos2) -> Option<Drag> {
    let s = app.canvas.table.as_ref()?;
    let (e, t) = table(app, &s.id)?;
    let l = t.layout(body_size(app));
    let k = px_per_unit(app, e).max(1e-6) as f64;
    let (x, y) = local(app, e, p);
    let top = l.caption;
    if y < top && y >= top - BAR as f64 / k && x >= 0.0 && x <= l.width() {
        let c = l.col_at(x);
        return Some(Drag::Column { from: c, to: c });
    }
    if x < 0.0 && x >= -(ROW_BAR as f64) / k && y >= top && y <= l.height() {
        let r = l.row_at(y);
        return Some(Drag::Row { from: r, to: r });
    }
    None
}

/// A drag starting at `p`. Returns true when the table takes it.
pub fn drag_start(app: &mut ZebraApp, p: Pos2) -> bool {
    let Some(s) = app.canvas.table.clone() else { return false };
    if let Some(k) = knob_at(app, p) {
        if let Some(st) = &mut app.canvas.table {
            st.drag = Some(if k == Knob::Right { Drag::AddColumns { delta: 0 } } else { Drag::AddRows { delta: 0 } });
        }
        return true;
    }
    if let Some(d) = bar_at(app, p) {
        if let Some(st) = &mut app.canvas.table {
            st.drag = Some(d);
            // The whole column or row is what moves: selected.
            if let Some((_, t)) = table(app, &s.id) {
                let (n, m) = (t.n_rows(), t.n_cols());
                let st = app.canvas.table.as_mut().unwrap();
                match d {
                    Drag::Column { from, .. } => {
                        st.anchor = (0, from);
                        st.head = (n - 1, from);
                    }
                    Drag::Row { from, .. } => {
                        st.anchor = (from, 0);
                        st.head = (from, m - 1);
                    }
                    _ => {}
                }
            }
        }
        return true;
    }
    if s.edit.is_some() {
        commit_edit(app, None);
    }
    if let Some(rc) = cell_at(app, &s.id, p) {
        select_cell(app, &s.id, rc, false);
        if let Some(st) = &mut app.canvas.table {
            st.drag = Some(Drag::Cells);
        }
        return true;
    }
    false
}

pub fn dragging(app: &ZebraApp) -> bool {
    app.canvas.table.as_ref().is_some_and(|s| s.drag.is_some())
}

pub fn drag_update(app: &mut ZebraApp, p: Pos2) {
    let Some(s) = app.canvas.table.clone() else { return };
    let Some(d) = s.drag else { return };
    let Some((e, t)) = table(app, &s.id) else { return };
    let l = t.layout(body_size(app));
    let (x, y) = local(app, e, p);
    let new = match d {
        Drag::Cells => {
            let rc = (l.row_at(y), l.col_at(x));
            if let Some(st) = &mut app.canvas.table {
                st.head = rc;
            }
            return;
        }
        Drag::Column { from, .. } => {
            // The gap nearest the pointer.
            let to = (0..=t.n_cols()).min_by(|a, b| (l.xs[*a] - x).abs().total_cmp(&(l.xs[*b] - x).abs())).unwrap_or(from);
            Drag::Column { from, to }
        }
        Drag::Row { from, .. } => {
            let to = (0..=t.n_rows()).min_by(|a, b| (l.ys[*a] - y).abs().total_cmp(&(l.ys[*b] - y).abs())).unwrap_or(from);
            Drag::Row { from, to }
        }
        Drag::AddColumns { .. } => {
            let unit = t.columns.last().map(|c| c.width).unwrap_or(100.0).max(1.0);
            Drag::AddColumns { delta: ((x - l.width()) / unit).round() as i64 }
        }
        Drag::AddRows { .. } => {
            let unit = t.rows.last().map(|r| r.h()).unwrap_or(40.0).max(1.0);
            Drag::AddRows { delta: ((y - l.height()) / unit).round() as i64 }
        }
    };
    if let Some(st) = &mut app.canvas.table {
        st.drag = Some(new);
    }
}

pub fn drag_end(app: &mut ZebraApp) {
    let Some(s) = app.canvas.table.clone() else { return };
    let Some(d) = s.drag else { return };
    if let Some(st) = &mut app.canvas.table {
        st.drag = None;
    }
    match d {
        Drag::Cells => {}
        Drag::Column { from, to } if to != from && to != from + 1 => {
            if apply_or_note(app, Command::MoveColumns { id: s.id.clone(), from, count: 1, to }) {
                let c = if to > from { to - 1 } else { to };
                let n = table(app, &s.id).map(|(_, t)| t.n_rows()).unwrap_or(1);
                if let Some(st) = &mut app.canvas.table {
                    st.anchor = (0, c);
                    st.head = (n - 1, c);
                }
            }
        }
        Drag::Row { from, to } if to != from && to != from + 1 => {
            if apply_or_note(app, Command::MoveRows { id: s.id.clone(), from, count: 1, to }) {
                let r = if to > from { to - 1 } else { to };
                let m = table(app, &s.id).map(|(_, t)| t.n_cols()).unwrap_or(1);
                if let Some(st) = &mut app.canvas.table {
                    st.anchor = (r, 0);
                    st.head = (r, m - 1);
                }
            }
        }
        Drag::AddColumns { delta } => add_from_knob(app, Knob::Right, delta),
        Drag::AddRows { delta } => add_from_knob(app, Knob::Bottom, delta),
        _ => {}
    }
}

/// A key while cells are selected (not while typing in one). Returns
/// true when the table used it.
pub fn key(app: &mut ZebraApp, key: Key, m: Modifiers) -> bool {
    let Some(s) = app.canvas.table.clone() else { return false };
    if s.edit.is_some() {
        return false;
    }
    let Some((_, t)) = table(app, &s.id) else { return false };
    let (n, cols) = (t.n_rows(), t.n_cols());
    let (r, c) = s.head;
    let arrow = match key {
        Key::ArrowUp => Some((-1i64, 0i64)),
        Key::ArrowDown => Some((1, 0)),
        Key::ArrowLeft => Some((0, -1)),
        Key::ArrowRight => Some((0, 1)),
        _ => None,
    };
    if let Some((dr, dc)) = arrow {
        if m.command {
            return false;
        }
        if m.alt {
            // Option-arrows add a row or column on that side.
            let range = s.range();
            let id = s.id.clone();
            let (cmd, at) = match (dr, dc) {
                (-1, _) => (Command::InsertRows { id, index: range.r0, count: 1 }, (range.r0, c)),
                (1, _) => (Command::InsertRows { id, index: range.r1 + 1, count: 1 }, (range.r1 + 1, c)),
                (_, -1) => (Command::InsertColumns { id, index: range.c0, count: 1 }, (r, range.c0)),
                _ => (Command::InsertColumns { id, index: range.c1 + 1, count: 1 }, (r, range.c1 + 1)),
            };
            if apply_or_note(app, cmd) {
                select_cell(app, &s.id, at, false);
            }
            return true;
        }
        // From a merged cell, step off its far side.
        let span = t.span(r, c);
        let nr = match dr {
            -1 => span.r0.saturating_sub(1),
            1 => (span.r1 + 1).min(n - 1),
            _ => r,
        };
        let nc = match dc {
            -1 => span.c0.saturating_sub(1),
            1 => (span.c1 + 1).min(cols - 1),
            _ => c,
        };
        select_cell(app, &s.id, (nr, nc), m.shift);
        return true;
    }
    match key {
        Key::Tab => {
            tab_move(app, !m.shift);
            true
        }
        Key::Enter if m.is_none() => {
            start_edit(app, None);
            true
        }
        // Letters and digits start typing (the Text event that follows):
        // they must not reach the tool shortcuts.
        _ if !m.command && !m.ctrl && is_typing_key(key) => true,
        _ => false,
    }
}

fn is_typing_key(k: Key) -> bool {
    let name = k.name();
    (name.len() == 1 && name.chars().all(|c| c.is_ascii_alphanumeric()))
        || matches!(k, Key::Equals | Key::Minus | Key::Plus | Key::Period | Key::Comma | Key::Space | Key::Quote | Key::Slash | Key::OpenBracket | Key::CloseBracket | Key::Semicolon | Key::Backtick)
}

/// Tab and Shift-Tab: the next or previous cell, row by row. Tab in the
/// last cell adds a row.
fn tab_move(app: &mut ZebraApp, forward: bool) {
    let Some(s) = app.canvas.table.clone() else { return };
    let Some((_, t)) = table(app, &s.id) else { return };
    let (n, m) = (t.n_rows(), t.n_cols());
    let span = t.span(s.head.0, s.head.1);
    let (r, c) = s.head;
    let next = if forward {
        if span.c1 + 1 < m {
            Some((r, span.c1 + 1))
        } else if r + 1 < n {
            Some((span.r1 + 1, 0))
        } else {
            None
        }
    } else if c > 0 {
        Some((r, span.c0 - 1))
    } else if r > 0 {
        Some((r - 1, m - 1))
    } else {
        Some((0, 0))
    };
    match next {
        Some(rc) => select_cell(app, &s.id, rc, false),
        None => {
            if apply_or_note(app, Command::InsertRows { id: s.id.clone(), index: n, count: 1 }) {
                select_cell(app, &s.id, (n, 0), false);
            }
        }
    }
}

/// Clipboard, Delete, Esc and Select All while cells are selected.
/// Returns true when the table did the action.
pub fn perform(app: &mut ZebraApp, a: &Action) -> bool {
    let Some(s) = app.canvas.table.clone() else { return false };
    match a {
        Action::Deselect => {
            if s.edit.is_some() {
                if let Some(st) = &mut app.canvas.table {
                    st.edit = None;
                }
            } else {
                app.canvas.table = None;
            }
            true
        }
        Action::Delete if s.edit.is_none() => {
            clear_cells(app, &s);
            true
        }
        Action::SelectAll if s.edit.is_none() => {
            if let Some((_, t)) = table(app, &s.id) {
                let (n, m) = (t.n_rows(), t.n_cols());
                if let Some(st) = &mut app.canvas.table {
                    st.anchor = (0, 0);
                    st.head = (n - 1, m - 1);
                }
            }
            true
        }
        Action::Copy if s.edit.is_none() => {
            copy_cells(app, &s);
            true
        }
        Action::Cut if s.edit.is_none() => {
            copy_cells(app, &s);
            clear_cells(app, &s);
            true
        }
        Action::Paste if s.edit.is_none() => {
            if let Ok(mut c) = arboard::Clipboard::new() {
                if let Ok(text) = c.get_text() {
                    paste_cells(app, &s, &text);
                }
            }
            true
        }
        _ => false,
    }
}

fn clear_cells(app: &mut ZebraApp, s: &TableSel) {
    let r = s.range();
    let values = vec![vec![CellInput::Text(String::new()); r.cols()]; r.rows()];
    apply_or_note(app, Command::SetCells { id: s.id.clone(), at: table::cell_name(r.r0, r.c0), values });
}

/// Text that a spreadsheet would read as a formula, made plain text
/// (CSV injection): what a copied cell shows is never run elsewhere.
pub fn defuse(cell: &str) -> String {
    match cell.chars().next() {
        Some('=' | '+' | '@' | '\t' | '\r') => format!("'{cell}"),
        Some('-') if cell.trim().parse::<f64>().is_err() => format!("'{cell}"),
        _ => cell.to_string(),
    }
}

/// The cells as tab-separated text: what they show, made safe to paste.
pub fn copy_text(t: &TableData, r: &CellRange) -> String {
    let rows: Vec<Vec<String>> = t.shown_rows(r).into_iter().map(|row| row.into_iter().map(|c| defuse(&c.replace('\u{2212}', "-"))).collect()).collect();
    table::to_tsv(&rows)
}

fn copy_cells(app: &mut ZebraApp, s: &TableSel) {
    let Some((_, t)) = table(app, &s.id) else { return };
    let text = copy_text(t, &s.range());
    let inputs = t.inputs(&s.range());
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text.clone());
    }
    // Pasted back into a table, the cells keep their formulas (their
    // relative references moved by how far they went).
    let r = s.range();
    app.table_clip = Some(TableClip { text: text.clone(), inputs, from: (r.r0, r.c0) });
    app.clipboard_marker = Some(text);
}

/// Cells copied from a table: the text the system clipboard got, and the
/// inputs (formulas) with where they came from.
#[derive(Debug, Clone)]
pub struct TableClip {
    pub text: String,
    pub inputs: Vec<Vec<String>>,
    pub from: (usize, usize),
}

/// Pasted text fills cells from the selection's top-left; the table
/// grows to take it.
pub fn paste_cells(app: &mut ZebraApp, s: &TableSel, text: &str) {
    let ours = app.table_clip.as_ref().filter(|c| c.text == text).cloned();
    let mut grid = match &ours {
        Some(c) => c.inputs.clone(),
        None => match table::parse_delimited(text) {
            Ok(g) => g,
            Err(l) => {
                app.note(limit_message(l));
                return;
            }
        },
    };
    if grid.is_empty() {
        return;
    }
    let r = s.range();
    let Some((_, t)) = table(app, &s.id) else { return };
    let rows = (r.r0 + grid.len()).max(t.n_rows());
    let cols = (r.c0 + grid.iter().map(|g| g.len()).max().unwrap_or(0)).max(t.n_cols());
    if let Some(l) = Limit::of_size(rows, cols) {
        app.note(limit_message(l));
        return;
    }
    // Our own cells: relative references move with them, as in Numbers;
    // one that would leave the table becomes #REF!.
    if let Some(c) = &ours {
        let (dr, dc) = (r.r0 as i64 - c.from.0 as i64, r.c0 as i64 - c.from.1 as i64);
        for cell in grid.iter_mut().flatten() {
            *cell = table::refs::shift(cell, dr, dc, rows, cols);
        }
    }
    if let Some(l) = grid.iter().flatten().find_map(|c| Limit::of_input(c)) {
        app.note(limit_message(l));
        return;
    }
    let (h, wd) = (grid.len(), grid.iter().map(|g| g.len()).max().unwrap_or(1));
    let values = grid.into_iter().map(|row| row.into_iter().map(CellInput::Text).collect()).collect();
    if apply_or_note(app, Command::SetCells { id: s.id.clone(), at: table::cell_name(r.r0, r.c0), values }) {
        if let Some(st) = &mut app.canvas.table {
            st.anchor = (r.r0, r.c0);
            st.head = (r.r0 + h - 1, r.c0 + wd - 1);
        }
        sync(app);
    }
}

// ----------------------------------------------------------------------
// Editing a cell

/// Start editing the head cell, with `typed` replacing what is there.
pub fn start_edit(app: &mut ZebraApp, typed: Option<String>) {
    let Some(s) = app.canvas.table.clone() else { return };
    let Some((_, t)) = table(app, &s.id) else { return };
    let at = t.anchor(s.head.0, s.head.1);
    let text = typed.unwrap_or_else(|| t.cell(at.0, at.1).map(|c| c.input.clone()).unwrap_or_default());
    if let Some(st) = &mut app.canvas.table {
        st.anchor = at;
        st.head = at;
        st.edit = Some(CellEdit { at, text, focused: false });
    }
}

/// Write the cell being edited, then move by `then` (rows, columns).
pub fn commit_edit(app: &mut ZebraApp, then: Option<(i64, i64)>) {
    let Some(s) = app.canvas.table.clone() else { return };
    let Some(edit) = s.edit else { return };
    if let Some(st) = &mut app.canvas.table {
        st.edit = None;
    }
    let Some((_, t)) = table(app, &s.id) else { return };
    let before = t.cell(edit.at.0, edit.at.1).map(|c| c.input.clone()).unwrap_or_default();
    if before != edit.text {
        if let Some(l) = Limit::of_input(&edit.text) {
            app.note(limit_message(l));
            return;
        }
        apply_or_note(app, Command::SetCells { id: s.id.clone(), at: table::cell_name(edit.at.0, edit.at.1), values: vec![vec![CellInput::Text(edit.text)]] });
    }
    match then {
        Some((0, 1)) => tab_move(app, true),
        Some((0, -1)) => tab_move(app, false),
        Some((dr, 0)) => {
            let Some((_, t)) = table(app, &s.id) else { return };
            let span = t.span(edit.at.0, edit.at.1);
            let r = if dr > 0 { (span.r1 + 1).min(t.n_rows() - 1) } else { span.r0.saturating_sub(1) };
            select_cell(app, &s.id, (r, edit.at.1), false);
        }
        _ => {}
    }
}

/// Typing with cells selected starts editing with what was typed.
fn typed_text(app: &ZebraApp, ctx: &egui::Context) -> Option<String> {
    if ctx.egui_wants_keyboard_input() || app.palette.is_some() {
        return None;
    }
    let s: String = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) if !i.modifiers.command => Some(t.clone()),
                _ => None,
            })
            .collect()
    });
    (!s.is_empty()).then_some(s)
}

// ----------------------------------------------------------------------
// Overlay: letters, numbers, selection, handles, the cell field

/// Colours for formula references, in order (as Keynote tints them).
const REF_COLOURS: [Color32; 4] = [Color32::from_rgb(0x2f, 0x7d, 0xd8), Color32::from_rgb(0xe0, 0x7b, 0x2a), Color32::from_rgb(0x2e, 0x9e, 0x5b), Color32::from_rgb(0x9b, 0x51, 0xc9)];

/// The cell references in a formula ("B2", "A1:C3"), outside strings.
pub fn references(formula: &str) -> Vec<CellRange> {
    table::refs::scan(formula).iter().map(|t| t.range()).collect()
}

pub fn overlay(app: &mut ZebraApp, ui: &mut Ui, painter: &Painter) {
    sync(app);
    let ctx = ui.ctx().clone();
    if let Some(typed) = typed_text(app, &ctx) {
        if app.canvas.table.as_ref().is_some_and(|s| s.edit.is_none()) {
            start_edit(app, Some(typed));
        }
    }
    let Some(s) = app.canvas.table.clone() else { return };
    let Some((e, t)) = table(app, &s.id) else { return };
    let (e, t) = (e.clone(), t.clone());
    let p = Palette::of(&ctx);
    let accent = style::accent();
    let l = t.layout(body_size(app));
    let k = px_per_unit(app, &e).max(1e-6) as f64;
    let range = s.range();
    let clip = painter.clip_rect();
    // Rows and columns on screen.
    let corners = [clip.left_top(), clip.right_top(), clip.left_bottom(), clip.right_bottom()].map(|q| local(app, &e, q));
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, y) in corners {
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    let (r_lo, r_hi) = (l.row_at(y0), l.row_at(y1));
    let (c_lo, c_hi) = (l.col_at(x0), l.col_at(x1));
    let top = l.caption;
    let bar_h = BAR as f64 / k;
    let bar_w = ROW_BAR as f64 / k;
    let font = style::ui_font(11.0);
    let bar_fill = if p.dark { Color32::from_gray(58) } else { Color32::from_gray(232) };
    let bar_on = accent.gamma_multiply(0.25);
    let text_c = p.weak;
    // The table's outline (its transform handles show only at table level).
    let whole = quad(app, &e, 0.0, top, l.width(), l.height());
    painter.add(Shape::closed_line(whole.to_vec(), Stroke::new(1.0, accent)));
    // Column letters.
    for c in c_lo..=c_hi {
        let on = c >= range.c0 && c <= range.c1;
        let q = quad(app, &e, l.xs[c], top - bar_h, l.xs[c + 1], top);
        painter.add(Shape::convex_polygon(q.to_vec(), if on { bar_on } else { bar_fill }, Stroke::new(1.0, p.border)));
        let mid = Pos2::new((q[0].x + q[2].x) / 2.0, (q[0].y + q[2].y) / 2.0);
        painter.text(mid, Align2::CENTER_CENTER, table::column_name(c), font.clone(), if on { accent } else { text_c });
    }
    // Row numbers.
    for r in r_lo..=r_hi {
        let on = r >= range.r0 && r <= range.r1;
        let q = quad(app, &e, -bar_w, l.ys[r], 0.0, l.ys[r + 1]);
        painter.add(Shape::convex_polygon(q.to_vec(), if on { bar_on } else { bar_fill }, Stroke::new(1.0, p.border)));
        let mid = Pos2::new((q[0].x + q[2].x) / 2.0, (q[0].y + q[2].y) / 2.0);
        painter.text(mid, Align2::CENTER_CENTER, (r + 1).to_string(), font.clone(), if on { accent } else { text_c });
    }
    // Where a dragged column or row would go.
    match s.drag {
        Some(Drag::Column { from, to }) if to != from && to != from + 1 => {
            let q = quad(app, &e, l.xs[to] - 1.5 / k, top - bar_h, l.xs[to] + 1.5 / k, l.height());
            painter.add(Shape::convex_polygon(q.to_vec(), accent, Stroke::NONE));
        }
        Some(Drag::Row { from, to }) if to != from && to != from + 1 => {
            let q = quad(app, &e, -bar_w, l.ys[to] - 1.5 / k, l.width(), l.ys[to] + 1.5 / k);
            painter.add(Shape::convex_polygon(q.to_vec(), accent, Stroke::NONE));
        }
        _ => {}
    }
    // Formula references while writing one.
    if let Some(edit) = &s.edit {
        if edit.text.starts_with('=') {
            for (i, rf) in references(&edit.text).iter().enumerate() {
                let Some(rf) = rf.clip(t.n_rows(), t.n_cols()) else { continue };
                let (a, b, c, d) = l.range_box(&rf);
                let col = REF_COLOURS[i % REF_COLOURS.len()];
                let q = quad(app, &e, a, b, c, d);
                painter.add(Shape::convex_polygon(q.to_vec(), col.gamma_multiply(0.12), Stroke::new(2.0, col)));
            }
        }
    }
    // The selection.
    let sel = t.expand_to_merges(range);
    let (a, b, c, d) = l.range_box(&sel);
    let q = quad(app, &e, a, b, c, d);
    let fill = if sel.is_single() { Color32::TRANSPARENT } else { accent.gamma_multiply(0.12) };
    painter.add(Shape::convex_polygon(q.to_vec(), fill, Stroke::new(2.0, accent)));
    // The round handles.
    if let Some(ks) = knobs(app, &s) {
        for (kind, c) in ks {
            let delta = match (s.drag, kind) {
                (Some(Drag::AddColumns { delta }), Knob::Right) | (Some(Drag::AddRows { delta }), Knob::Bottom) => delta,
                _ => 0,
            };
            painter.circle(c, KNOB, p.field, Stroke::new(1.5, accent));
            let sign = if delta < 0 { "\u{2212}" } else { "+" };
            painter.text(c, Align2::CENTER_CENTER, sign, style::ui_font(12.0), accent);
            if delta != 0 {
                painter.text(c + egui::vec2(KNOB + 4.0, 0.0), Align2::LEFT_CENTER, format!("{sign}{}", delta.abs()), style::ui_font(11.0), accent);
            }
        }
    }
    if s.edit.is_some() {
        cell_field(app, &ctx, &e, &t, &l);
    }
}

/// The text field over the cell being edited.
fn cell_field(app: &mut ZebraApp, ctx: &egui::Context, e: &Element, t: &TableData, l: &table::Layout) {
    let Some(s) = app.canvas.table.clone() else { return };
    let Some(mut edit) = s.edit.clone() else { return };
    let span = t.span(edit.at.0, edit.at.1);
    let (a, b, c, d) = l.range_box(&span);
    let q = quad(app, e, a, b, c, d);
    let rect = ERect::from_points(&q);
    let k = px_per_unit(app, e);
    let size = (l.font as f32 * k).clamp(9.0, 64.0);
    let font = egui::FontId::proportional(size);
    let id = app.uid("zebra-table-cell-edit");
    let p = Palette::of(ctx);
    // Keys the field must not take: they move between cells.
    let (enter, tab, back_tab, esc, alt_enter) = ctx.input_mut(|i| {
        let alt_enter = i.consume_key(Modifiers::ALT, Key::Enter);
        (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Tab), i.consume_key(Modifiers::SHIFT, Key::Tab), i.consume_key(Modifiers::NONE, Key::Escape), alt_enter)
    });
    if alt_enter {
        edit.text.push('\n');
    }
    let canvas = app.canvas.rect;
    let formula = edit.text.starts_with('=');
    // Text is typed in the cell; a formula in the formula bar below it,
    // the cell showing what is being written.
    let mut bar = None;
    if formula {
        // In the document's colours, as the cell is drawn.
        let th = app.editor.theme();
        let (paper, ink) = (style::c32(th.color(&"theme:background".to_string())), style::c32(th.color(&"theme:body".to_string())));
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, id.with("cell"))).with_clip_rect(canvas);
        painter.rect(rect, 0.0, paper, Stroke::new(2.0, style::accent()), egui::StrokeKind::Outside);
        painter.rect_filled(rect, 0.0, style::accent().gamma_multiply(0.08));
        let galley = painter.layout_no_wrap(edit.text.clone(), font.clone(), ink);
        let y = rect.center().y - galley.size().y / 2.0;
        let x = if galley.size().x + 8.0 > rect.width() { rect.right() - 4.0 - galley.size().x } else { rect.left() + 4.0 };
        painter.with_clip_rect(rect.intersect(canvas)).galley(egui::pos2(x, y), galley, ink);
        // Under the table, as in the mockup, so it never covers a cell a
        // click would add (on screen, if the table runs off the bottom).
        let whole = ERect::from_points(&quad(app, e, 0.0, l.caption, l.width(), l.height()));
        bar = formula_bar(ctx, id, &mut edit, egui::pos2(rect.left(), whole.bottom() + 30.0), canvas, &p);
    } else {
        let area = egui::Area::new(id.with("area")).fixed_pos(rect.min).order(egui::Order::Foreground).constrain(false);
        area.show(ctx, |ui| {
            ui.set_clip_rect(canvas);
            let width = rect.width().max(60.0);
            let frame = egui::Frame::NONE.fill(p.field).stroke(Stroke::new(2.0, style::accent())).inner_margin(egui::Margin::symmetric(4, 2));
            frame.show(ui, |ui| {
                ui.set_min_size(egui::vec2(width - 8.0, (rect.height() - 4.0).max(size)));
                let te = egui::TextEdit::multiline(&mut edit.text).font(font.clone()).text_color(p.text).frame(egui::Frame::NONE).desired_width(width - 8.0).desired_rows(1).lock_focus(true).id(id);
                let out = te.show(ui);
                focus_once(ui, &out, &mut edit, id);
            });
        });
    }
    if let Some(st) = &mut app.canvas.table {
        st.edit = Some(edit);
    }
    if esc || bar == Some(false) {
        if let Some(st) = &mut app.canvas.table {
            st.edit = None;
        }
    } else if bar == Some(true) {
        commit_edit(app, None);
    } else if enter {
        commit_edit(app, Some((1, 0)));
    } else if tab {
        commit_edit(app, Some((0, 1)));
    } else if back_tab {
        commit_edit(app, Some((0, -1)));
    }
}

/// Focus a field the first frame it shows, with the caret at the end.
fn focus_once(ui: &Ui, out: &egui::text_edit::TextEditOutput, edit: &mut CellEdit, id: egui::Id) {
    if !edit.focused {
        out.response.request_focus();
        let mut st = out.state.clone();
        let n = edit.text.chars().count();
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(n))));
        st.store(ui.ctx(), id);
        edit.focused = true;
    }
}

/// The formula bar: the formula with each reference a token in the
/// colour its cells are outlined in, then Cancel (×) and Accept (✓).
/// Returns Some(true) for Accept, Some(false) for Cancel.
fn formula_bar(ctx: &egui::Context, id: egui::Id, edit: &mut CellEdit, at: Pos2, canvas: ERect, p: &Palette) -> Option<bool> {
    let font = style::ui_font(15.0);
    let mut out = None;
    let (text_c, weak) = (p.text, p.weak);
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
        let text = buf.as_str();
        let mut job = egui::text::LayoutJob::default();
        let plain = egui::TextFormat { font_id: font.clone(), color: text_c, ..Default::default() };
        // Pieces: (byte range, colour, space before). A token has room
        // each side for its box; the leading "=" is dimmed.
        let mut pieces: Vec<(Range<usize>, Color32, f32)> = vec![];
        let mut pad = 0.0;
        let plain_run = |pieces: &mut Vec<(Range<usize>, Color32, f32)>, r: Range<usize>, pad: &mut f32| {
            if r.is_empty() {
                return;
            }
            if r.start == 0 && text.starts_with('=') {
                pieces.push((0..1, weak, *pad));
                *pad = 0.0;
                if r.end > 1 {
                    pieces.push((1..r.end, text_c, 0.0));
                }
            } else {
                pieces.push((r, text_c, *pad));
                *pad = 0.0;
            }
        };
        let mut at = 0;
        for (i, tok) in table::refs::scan(text).iter().enumerate() {
            plain_run(&mut pieces, at..tok.span.start, &mut pad);
            pieces.push((tok.span.clone(), REF_COLOURS[i % REF_COLOURS.len()], 5.0));
            pad = 5.0;
            at = tok.span.end;
        }
        plain_run(&mut pieces, at..text.len(), &mut pad);
        for (r, c, lead) in pieces {
            job.append(&text[r], lead, egui::TextFormat { color: c, ..plain.clone() });
        }
        ui.fonts_mut(|f| f.layout_job(job))
    };
    egui::Area::new(id.with("bar")).fixed_pos(at).order(egui::Order::Foreground).constrain_to(canvas.shrink(4.0)).show(ctx, |ui| {
        let frame = egui::Frame::NONE.fill(p.field).stroke(Stroke::new(1.0, p.control_edge)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(10, 6)).shadow(egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(40) });
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let te = egui::TextEdit::multiline(&mut edit.text).frame(egui::Frame::NONE).desired_rows(1).desired_width(260.0).lock_focus(true).layouter(&mut layouter).id(id);
                let o = te.show(ui);
                let galley = o.galley.clone();
                let origin = o.galley_pos;
                // A box round each reference, in its colour.
                let text = edit.text.clone();
                for (i, tok) in table::refs::scan(&text).iter().enumerate() {
                    let (a, b) = (text[..tok.span.start].chars().count(), text[..tok.span.end].chars().count());
                    let ra = galley.pos_from_cursor(egui::text::CCursor::new(a));
                    let rb = galley.pos_from_cursor(egui::text::CCursor::new(b));
                    let r = ERect::from_min_max(egui::pos2(ra.min.x - 4.0, ra.min.y), egui::pos2(rb.min.x + 4.0, ra.max.y)).translate(origin.to_vec2()).expand2(egui::vec2(0.0, 1.0));
                    let col = REF_COLOURS[i % REF_COLOURS.len()];
                    ui.painter().rect(r, 5.0, col.gamma_multiply(0.14), Stroke::new(1.0, col.gamma_multiply(0.7)), egui::StrokeKind::Inside);
                }
                focus_once(ui, &o, edit, id);
                ui.add_space(6.0);
                if bar_button(ui, &crate::t!("table-formula-cancel"), false).clicked() {
                    out = Some(false);
                }
                if bar_button(ui, &crate::t!("table-formula-accept"), true).clicked() {
                    out = Some(true);
                }
            });
        })
        .response
        .on_hover_text(crate::t!("table-formula-hint"));
    });
    out
}

/// × (red) or ✓ (green), drawn.
fn bar_button(ui: &mut Ui, label: &str, accept: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let col = if accept { Color32::from_rgb(0x2e, 0x9e, 0x5b) } else { Color32::from_rgb(0xd9, 0x3b, 0x30) };
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, col.gamma_multiply(0.15));
    }
    let c = r.center();
    let st = Stroke::new(1.8, col);
    if accept {
        ui.painter().line(vec![c + egui::vec2(-5.0, 0.5), c + egui::vec2(-1.5, 4.0), c + egui::vec2(5.0, -4.0)], st);
    } else {
        ui.painter().line_segment([c + egui::vec2(-4.0, -4.0), c + egui::vec2(4.0, 4.0)], st);
        ui.painter().line_segment([c + egui::vec2(4.0, -4.0), c + egui::vec2(-4.0, 4.0)], st);
    }
    resp.on_hover_text(label)
}

// ----------------------------------------------------------------------
// Right-click menu

pub fn menu(ui: &mut Ui, app: &mut ZebraApp) {
    let Some(s) = app.canvas.table.clone() else { return };
    let Some((_, t)) = table(app, &s.id) else { return };
    let r = s.range();
    let id = s.id.clone();
    let merged = t.merges.iter().any(|m| m.intersects(&r));
    let items: Vec<(String, Option<Command>)> = vec![
        (crate::t!("table-add-row-above"), Some(Command::InsertRows { id: id.clone(), index: r.r0, count: 1 })),
        (crate::t!("table-add-row-below"), Some(Command::InsertRows { id: id.clone(), index: r.r1 + 1, count: 1 })),
        (crate::t!("table-add-column-before"), Some(Command::InsertColumns { id: id.clone(), index: r.c0, count: 1 })),
        (crate::t!("table-add-column-after"), Some(Command::InsertColumns { id: id.clone(), index: r.c1 + 1, count: 1 })),
        (String::new(), None),
        (crate::t!("table-delete-row"), (r.rows() < t.n_rows()).then(|| Command::DeleteRows { id: id.clone(), index: r.r0, count: r.rows() })),
        (crate::t!("table-delete-column"), (r.cols() < t.n_cols()).then(|| Command::DeleteColumns { id: id.clone(), index: r.c0, count: r.cols() })),
        (String::new(), None),
        (crate::t!("table-merge-cells"), (!r.is_single()).then(|| Command::MergeCells { id: id.clone(), range: r })),
        (crate::t!("table-unmerge-cells"), merged.then(|| Command::UnmergeCells { id: id.clone(), range: r })),
        (String::new(), None),
        (crate::t!("table-sort-ascending"), Some(Command::SortTable { id: id.clone(), column: r.c0, descending: false })),
        (crate::t!("table-sort-descending"), Some(Command::SortTable { id: id.clone(), column: r.c0, descending: true })),
    ];
    for (label, cmd) in items {
        if label.is_empty() {
            w::menu_separator(ui);
            continue;
        }
        let enabled = cmd.is_some();
        if w::menu_row(ui, None, &label, None, enabled).clicked() {
            if let Some(cmd) = cmd {
                apply_or_note(app, cmd);
                sync(app);
            }
            ui.close();
        }
    }
    w::menu_separator(ui);
}

// ----------------------------------------------------------------------
// Toolbar: the Table item's menu of styles

/// A small picture of a style in the current theme.
pub fn style_thumb(ui: &mut Ui, app: &ZebraApp, s: TableStyle, size: egui::Vec2, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let name = style_name(s);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &name));
    let th = app.editor.theme();
    let mut t = TableData::new(6, 3, 10.0);
    t.style = s;
    t.footer_rows = 1;
    t.header_columns = u8::from(s == TableStyle::SideHeader);
    let look = zebra_core::table_style::look(&t, &th);
    let c32 = |c: [u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    let painter = ui.painter_at(rect);
    let inner = rect.shrink(3.0);
    painter.rect_filled(inner, 2.0, c32(look.canvas));
    let n = 6;
    let rh = inner.height() / n as f32;
    let cw = inner.width() / 3.0;
    for r in 0..n {
        for c in 0..3 {
            let paint = zebra_core::table_style::cell_paint(&t, &th, &look, r, c);
            let cell = ERect::from_min_size(inner.min + egui::vec2(c as f32 * cw, r as f32 * rh), egui::vec2(cw, rh));
            if let Some(f) = paint.fill {
                painter.rect_filled(cell, 0.0, c32(f));
            }
            // A text bar in each cell.
            let wbar = if c == 0 { cw * 0.55 } else { cw * 0.4 };
            let x = if c == 0 { cell.left() + 3.0 } else { cell.right() - 3.0 - wbar };
            painter.rect_filled(ERect::from_min_size(Pos2::new(x, cell.center().y - 1.0), egui::vec2(wbar, 2.0)), 0.0, c32(paint.text));
        }
        let y = inner.top() + (r + 1) as f32 * rh;
        if r == 0 {
            if let Some((c, _)) = look.header_rule {
                painter.line_segment([Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)], Stroke::new(1.5, c32(c)));
            }
        } else if r == n - 2 {
            match look.footer_rule {
                zebra_core::table_style::FootRule::Line(c, _) => {
                    painter.line_segment([Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)], Stroke::new(1.5, c32(c)));
                }
                zebra_core::table_style::FootRule::Double(c) => {
                    painter.line_segment([Pos2::new(inner.left(), y - 1.0), Pos2::new(inner.right(), y - 1.0)], Stroke::new(0.7, c32(c)));
                    painter.line_segment([Pos2::new(inner.left(), y + 1.0), Pos2::new(inner.right(), y + 1.0)], Stroke::new(0.7, c32(c)));
                }
                _ => {}
            }
        } else if r < n - 1 {
            if let Some(c) = look.row_rule {
                painter.line_segment([Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)], Stroke::new(0.6, c32(c)));
            }
        }
    }
    if let Some(c) = look.column_rule {
        for i in 1..3 {
            let x = inner.left() + i as f32 * cw;
            painter.line_segment([Pos2::new(x, inner.top()), Pos2::new(x, inner.bottom())], Stroke::new(0.6, c32(c)));
        }
    }
    if let Some((c, _)) = look.outline {
        painter.rect_stroke(inner, 0.0, Stroke::new(1.2, c32(c)), egui::StrokeKind::Inside);
    }
    let p = Palette::of(ui.ctx());
    let border = if on { Stroke::new(2.0, style::accent()) } else if resp.hovered() { Stroke::new(1.0, p.weak) } else { Stroke::new(1.0, p.border) };
    painter.rect_stroke(rect.shrink(1.0), 4.0, border, egui::StrokeKind::Inside);
    resp.on_hover_text(style_name(s))
}

/// The grid of styles with their names: the toolbar's Table menu and the
/// inspector's Table Styles. Returns the style clicked.
pub fn style_grid(ui: &mut Ui, app: &ZebraApp, current: Option<TableStyle>, width: f32) -> Option<TableStyle> {
    let mut out = None;
    let cols = 3;
    let gap = 6.0;
    let cw = ((width - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let size = egui::vec2(cw, (cw * 0.68).round());
    let p = Palette::of(ui.ctx());
    for chunk in TableStyle::ALL.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for s in chunk {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if style_thumb(ui, app, *s, size, current == Some(*s)).clicked() {
                        out = Some(*s);
                    }
                    // The name under it (painted: the picture is the button
                    // and already carries the name).
                    let (r, _) = ui.allocate_exact_size(egui::vec2(cw, 14.0), egui::Sense::hover());
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, style_name(*s), style::ui_font(11.0), p.weak);
                });
            }
        });
        ui.add_space(4.0);
    }
    out
}

/// A style's name in the app.
pub fn style_name(s: TableStyle) -> String {
    match s.id() {
        "ts1" => crate::t!("table-style-banner"),
        "ts2" => crate::t!("table-style-banded"),
        "ts3" => crate::t!("table-style-grid"),
        "ts4" => crate::t!("table-style-ruled"),
        "ts6" => crate::t!("table-style-accent"),
        "ts7" => crate::t!("table-style-side-header"),
        "ts8" => crate::t!("table-style-boxed"),
        "ts9" => crate::t!("table-style-soft"),
        "ts10" => crate::t!("table-style-dark"),
        _ => crate::t!("table-style-ledger"),
    }
}

// ----------------------------------------------------------------------
// Inspector: Table, Cell and Text tabs

/// The tabs a table has.
pub fn tabs() -> Vec<Tab> {
    vec![Tab::Table, Tab::Cell, Tab::Text, Tab::Arrange, Tab::Path]
}

/// The cells the Cell and Text tabs change: the selected ones, else all.
fn target(insp: &Insp, t: &TableData) -> CellRange {
    match &insp.app.canvas.table {
        Some(s) if s.id == insp.el.id => t.expand_to_merges(s.range()),
        _ => t.whole(),
    }
}

pub fn table_tab(insp: &mut Insp, ui: &mut Ui) {
    let el = insp.el.clone();
    let ElementKind::Table(t) = &el.kind else { return };
    w::section(ui, &crate::t!("table-section-styles"), |ui| {
        let wdt = ui.available_width();
        if let Some(s) = style_grid(ui, insp.app, Some(t.style), wdt) {
            // Side Header shows on a header column: one if there's none.
            let header_columns = (s == TableStyle::SideHeader && t.header_columns == 0).then_some(1);
            insp.set(ElementPatch { table_style: Some(s), header_columns, ..Default::default() });
        }
    });
    w::section(ui, &crate::t!("table-section-headers"), |ui| {
        let n = t.n_rows() as f64;
        for (label, now, which) in [(crate::t!("table-header-rows"), t.header_rows, 0), (crate::t!("table-header-columns"), t.header_columns, 1), (crate::t!("table-footer-rows"), t.footer_rows, 2)] {
            w::row_w(ui, &label, 96.0, |ui| {
                let mut v = now as f64;
                let max = if which == 1 { t.n_cols() as f64 } else { n }.min(table::MAX_HEADERS as f64);
                let r = w::number_field(ui, &mut v, Num::default().range(0.0, max).step(1.0).decimals(0).width(48.0));
                if r.changed() {
                    let v = v.round().clamp(0.0, max) as u8;
                    let patch = match which {
                        0 => ElementPatch { header_rows: Some(v), ..Default::default() },
                        1 => ElementPatch { header_columns: Some(v), ..Default::default() },
                        _ => ElementPatch { footer_rows: Some(v), ..Default::default() },
                    };
                    insp.set(patch);
                }
            });
        }
    });
    w::section(ui, "", |ui| {
        let mut named = t.show_name;
        if w::checkbox(ui, &mut named, &crate::t!("table-name")).changed() {
            insp.set(ElementPatch { show_name: Some(named), ..Default::default() });
        }
        if named {
            let mut name = el.name.clone();
            let r = w::text_field(ui, &mut name, ui.available_width(), &crate::t!("table-name-hint"));
            insp.track(&r);
            if r.changed() {
                insp.set(ElementPatch { name: Some(name), ..Default::default() });
            }
        }
        let th = insp.theme.clone();
        let mut banded = zebra_core::table_style::look(t, &th).band_fill.is_some();
        if w::checkbox(ui, &mut banded, &crate::t!("table-alternating-rows")).changed() {
            insp.set(ElementPatch { banded: Some(banded), ..Default::default() });
        }
    });
    w::section(ui, &crate::t!("table-section-gridlines"), |ui| {
        let th = insp.theme.clone();
        let look = zebra_core::table_style::look(t, &th);
        let now = Gridlines { horizontal: look.row_rule.is_some(), vertical: look.column_rule.is_some(), outline: look.outline.is_some() };
        // Four independent toggles, as in the mockup: all inner lines,
        // rows, columns, outline.
        let mut g = now;
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let both = now.horizontal && now.vertical;
            if w::icon_toggle(ui, crate::icons::Icon::GridAll, both, &crate::t!("table-gridlines-all")).clicked() {
                g.horizontal = !both;
                g.vertical = !both;
                changed = true;
            }
            if w::icon_toggle(ui, crate::icons::Icon::GridRows, now.horizontal, &crate::t!("table-gridlines-horizontal")).clicked() {
                g.horizontal = !now.horizontal;
                changed = true;
            }
            if w::icon_toggle(ui, crate::icons::Icon::GridColumns, now.vertical, &crate::t!("table-gridlines-vertical")).clicked() {
                g.vertical = !now.vertical;
                changed = true;
            }
            if w::icon_toggle(ui, crate::icons::Icon::GridOutline, now.outline, &crate::t!("table-gridlines-outline")).clicked() {
                g.outline = !now.outline;
                changed = true;
            }
        });
        if changed {
            insp.set(ElementPatch { gridlines: Some(g), ..Default::default() });
        }
    });
    w::section(ui, "", |ui| {
        w::row_w(ui, &crate::t!("table-font-size"), 96.0, |ui| {
            let label = |f: f64| if (f - 1.0).abs() < 1e-9 { crate::t!("table-font-size-theme") } else { format!("{}%", (f * 100.0).round()) };
            popup(ui, "table-font-size", &label(t.font_size), 110.0, |ui| {
                for f in [0.6, 0.75, 0.9, 1.0, 1.15, 1.3, 1.5, 2.0] {
                    if w::menu_check(ui, (t.font_size - f).abs() < 1e-9, &label(f), None).clicked() {
                        insp.set(ElementPatch { font_size: Some(f), ..Default::default() });
                    }
                }
            });
        });
    });
}

fn popup(ui: &mut Ui, id: &str, label: &str, width: f32, add: impl FnOnce(&mut Ui)) {
    w::popup_button(ui, id, label, width, add);
}

fn type_name(k: CellType) -> String {
    match k {
        CellType::Automatic => crate::t!("table-format-automatic"),
        CellType::Number => crate::t!("table-format-number"),
        CellType::Currency => crate::t!("table-format-currency"),
        CellType::Percentage => crate::t!("table-format-percentage"),
        CellType::DateTime => crate::t!("table-format-date-time"),
        CellType::Text => crate::t!("table-format-text"),
        CellType::TickBox => crate::t!("table-format-tick-box"),
    }
}

fn negative_name(n: Negative, f: &DataFormat) -> String {
    let unit = |v: &str| match f.kind {
        CellType::Currency => format!("{}{v}", f.symbol()),
        CellType::Percentage => format!("{v}%"),
        _ => v.to_string(),
    };
    match n {
        Negative::Minus => format!("\u{2212}{}", unit("100")),
        Negative::Red => crate::t!("table-negative-red", example = format!("\u{2212}{}", unit("100"))),
        Negative::Parentheses => format!("({})", unit("100")),
        Negative::RedParentheses => crate::t!("table-negative-red", example = format!("({})", unit("100"))),
    }
}

fn format_cells(insp: &mut Insp, range: CellRange, format: CellFormat) {
    let id = insp.el.id.clone();
    apply_or_note(insp.app, Command::FormatCells { id, range, format, clear: false });
}

pub fn cell_tab(insp: &mut Insp, ui: &mut Ui) {
    let el = insp.el.clone();
    let ElementKind::Table(t) = &el.kind else { return };
    let range = target(insp, t);
    let (r, c) = (range.r0, range.c0);
    let f = t.data_format(r, c);
    let own = t.format_at(r, c);
    w::section(ui, &crate::t!("table-section-data-format"), |ui| {
        let wdt = ui.available_width();
        popup(ui, "table-data-format", &type_name(f.kind), wdt, |ui| {
            for k in CellType::ALL {
                if w::menu_check(ui, f.kind == k, &type_name(k), None).clicked() {
                    format_cells(insp, range, CellFormat { number: Some(DataFormat { kind: k, ..f.clone() }), ..Default::default() });
                }
            }
        });
        if matches!(f.kind, CellType::Number | CellType::Currency | CellType::Percentage) {
            w::row(ui, &crate::t!("table-decimals"), |ui| {
                let mut d = f.decimals.map(|d| d as f64).unwrap_or(0.0);
                if w::number_field(ui, &mut d, Num::default().range(0.0, 10.0).step(1.0).decimals(0).width(48.0)).changed() {
                    format_cells(insp, range, CellFormat { number: Some(DataFormat { decimals: Some(d.round().clamp(0.0, 10.0) as u8), ..f.clone() }), ..Default::default() });
                }
            });
            w::row(ui, &crate::t!("table-negative-numbers"), |ui| {
                popup(ui, "table-negative", &negative_name(f.negative, &f), 120.0, |ui| {
                    for n in Negative::ALL {
                        if w::menu_check(ui, f.negative == n, &negative_name(n, &f), None).clicked() {
                            format_cells(insp, range, CellFormat { number: Some(DataFormat { negative: n, ..f.clone() }), ..Default::default() });
                        }
                    }
                });
            });
            let mut th = f.thousands;
            if w::checkbox(ui, &mut th, &crate::t!("table-thousands")).changed() {
                format_cells(insp, range, CellFormat { number: Some(DataFormat { thousands: th, ..f.clone() }), ..Default::default() });
            }
        }
        if f.kind == CellType::Currency {
            w::row(ui, &crate::t!("table-currency"), |ui| {
                popup(ui, "table-currency", f.symbol(), 90.0, |ui| {
                    for sym in ["£", "$", "€", "¥", "₹", "CHF"] {
                        if w::menu_check(ui, f.symbol() == sym, sym, None).clicked() {
                            format_cells(insp, range, CellFormat { number: Some(DataFormat { currency: sym.to_string(), ..f.clone() }), ..Default::default() });
                        }
                    }
                });
            });
        }
    });
    w::section(ui, &crate::t!("table-section-column-type"), |ui| {
        let col = t.columns.get(c).map(|x| x.format.clone()).unwrap_or_default();
        let wdt = ui.available_width();
        popup(ui, "table-column-type", &crate::t!("table-column-type-whole", kind = type_name(col.kind)), wdt, |ui| {
            for k in CellType::ALL {
                if w::menu_check(ui, col.kind == k, &type_name(k), None).clicked() {
                    let id = insp.el.id.clone();
                    for column in range.c0..=range.c1 {
                        apply_or_note(insp.app, Command::SetColumn { id: id.clone(), column, width: None, format: Some(DataFormat { kind: k, ..col.clone() }) });
                    }
                }
            }
        });
        w::caption(ui, crate::t!("table-column-type-note"));
    });
    w::section(ui, &crate::t!("table-section-fill"), |ui| {
        let current = own.fill.clone().unwrap_or_default();
        let wdt = ui.available_width();
        let label = if current.is_empty() { crate::t!("table-fill-style") } else { crate::t!("table-fill-colour") };
        popup(ui, "table-fill", &label, wdt, |ui| {
            if w::menu_check(ui, current.is_empty(), &crate::t!("table-fill-style"), None).clicked() {
                format_cells(insp, range, CellFormat { fill: Some(String::new()), ..Default::default() });
            }
        });
        let slots: Vec<(&str, String)> = zebra_core::Theme::COLOR_SLOTS.iter().map(|s| (*s, format!("theme:{s}"))).collect();
        let sw = crate::inspector::theme_swatches(insp, &slots);
        if let Some(k) = crate::inspector::colour_picker(ui, "table-fill-colour", &current, Color32::TRANSPARENT, &sw) {
            format_cells(insp, range, CellFormat { fill: Some(k), ..Default::default() });
        }
    });
    w::section(ui, &crate::t!("table-section-border"), |ui| {
        let names = [
            (BorderEdges::All, crate::t!("table-border-all")),
            (BorderEdges::Outer, crate::t!("table-border-outer")),
            (BorderEdges::Horizontal, crate::t!("table-border-horizontal")),
            (BorderEdges::Vertical, crate::t!("table-border-vertical")),
            (BorderEdges::Top, crate::t!("table-border-top")),
            (BorderEdges::Bottom, crate::t!("table-border-bottom")),
        ];
        let labels: Vec<(&str, &str, bool)> = names.iter().map(|(_, n)| (n.as_str(), n.as_str(), true)).collect();
        let b = own.border.clone();
        let width = b.as_ref().and_then(|b| b.width).unwrap_or(1.0);
        let style_now = b.as_ref().map(|b| b.style).unwrap_or_default();
        if let Some(i) = w::button_grid(ui, &labels, 3) {
            let border = Border { edges: names[i].0, style: style_now, color: b.as_ref().and_then(|b| b.color.clone()), width: Some(width) };
            format_cells(insp, range, CellFormat { border: Some(border), ..Default::default() });
        }
        let line_name = |s: LineStyle| match s {
            LineStyle::Solid => crate::t!("table-line-solid"),
            LineStyle::Dashed => crate::t!("table-line-dashed"),
            LineStyle::Dotted => crate::t!("table-line-dotted"),
            LineStyle::None => crate::t!("table-line-none"),
        };
        ui.horizontal(|ui| {
            popup(ui, "table-border-style", &line_name(style_now), 110.0, |ui| {
                for s in [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted, LineStyle::None] {
                    if w::menu_check(ui, style_now == s, &line_name(s), None).clicked() {
                        let edges = b.as_ref().map(|b| b.edges).unwrap_or(BorderEdges::All);
                        format_cells(insp, range, CellFormat { border: Some(Border { edges, style: s, color: b.as_ref().and_then(|b| b.color.clone()), width: Some(width) }), ..Default::default() });
                    }
                }
            });
            let mut wv = width;
            if w::number_field(ui, &mut wv, Num::pt().range(0.25, 20.0).step(0.5).decimals(1).width(64.0)).changed() {
                let edges = b.as_ref().map(|b| b.edges).unwrap_or(BorderEdges::All);
                format_cells(insp, range, CellFormat { border: Some(Border { edges, style: style_now, color: b.as_ref().and_then(|b| b.color.clone()), width: Some(wv) }), ..Default::default() });
            }
        });
    });
}

/// The Text tab, applied to the selected cells.
pub fn text_tab(insp: &mut Insp, ui: &mut Ui) {
    let el = insp.el.clone();
    let ElementKind::Table(t) = &el.kind else { return };
    let range = target(insp, t);
    let f = t.format_at(range.r0, range.c0);
    let bold = t.bold_at(range.r0, range.c0);
    let italic = f.italic.unwrap_or(false);
    w::section(ui, &crate::t!("table-section-font"), |ui| {
        let b_label = crate::t!("table-bold");
        let i_label = crate::t!("table-italic");
        if let Some(i) = w::segmented_toggles(ui, &[bold, italic], &[Seg::Text(&b_label), Seg::Text(&i_label)], Some(ui.available_width())) {
            let fmt = if i == 0 { CellFormat { bold: Some(!bold), ..Default::default() } } else { CellFormat { italic: Some(!italic), ..Default::default() } };
            format_cells(insp, range, fmt);
        }
    });
    w::section(ui, &crate::t!("table-section-alignment"), |ui| {
        let shown = t.shown(range.r0, range.c0, None);
        let mut a = t.align_at(range.r0, range.c0, &shown);
        let (l, c, r) = (crate::t!("table-align-left"), crate::t!("table-align-centre"), crate::t!("table-align-right"));
        let items = [(Align::Left, Seg::Text(&l)), (Align::Center, Seg::Text(&c)), (Align::Right, Seg::Text(&r))];
        if w::segmented(ui, &mut a, &items, Some(ui.available_width())).changed() {
            format_cells(insp, range, CellFormat { align: Some(a), ..Default::default() });
        }
        ui.add_space(4.0);
        let mut v = f.valign.unwrap_or_default();
        let (tp, md, bt) = (crate::t!("table-valign-top"), crate::t!("table-valign-middle"), crate::t!("table-valign-bottom"));
        let items = [(VAlign::Top, Seg::Text(&tp)), (VAlign::Middle, Seg::Text(&md)), (VAlign::Bottom, Seg::Text(&bt))];
        if w::segmented(ui, &mut v, &items, Some(ui.available_width())).changed() {
            format_cells(insp, range, CellFormat { valign: Some(v), ..Default::default() });
        }
    });
    w::section(ui, &crate::t!("table-section-text-colour"), |ui| {
        let current = f.color.clone().unwrap_or_default();
        if w::menu_check(ui, current.is_empty(), &crate::t!("table-text-colour-style"), None).clicked() && !current.is_empty() {
            format_cells(insp, range, CellFormat { color: Some(String::new()), ..Default::default() });
        }
        let slots: Vec<(&str, String)> = zebra_core::Theme::COLOR_SLOTS.iter().map(|s| (*s, format!("theme:{s}"))).collect();
        let sw = crate::inspector::theme_swatches(insp, &slots);
        if let Some(k) = crate::inspector::colour_picker(ui, "table-text-colour", &current, Color32::BLACK, &sw) {
            format_cells(insp, range, CellFormat { color: Some(k), ..Default::default() });
        }
    });
}
