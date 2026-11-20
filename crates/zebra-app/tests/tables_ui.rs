//! Spec 19a: tables in the app. Insert from the toolbar or Insert ›
//! Table; one click selects the table, a second a cell (column letters and
//! row numbers appear); typing edits; Tab, Return and Option-arrows; the
//! right-click menu; the Table, Cell and Text inspector tabs; pasting
//! tab-separated cells.
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::i18n::tr;
use zebra_app::{table_ui, AppOptions, ZebraApp};
use zebra_core::table::TableData;
use zebra_core::{ElementKind, Origin};

const H: Origin = Origin::Human;

fn harness() -> Harness<'static, ZebraApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn shot(h: &mut Harness<'static, ZebraApp>, name: &str) {
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        let _ = std::fs::create_dir_all(&dir);
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

fn the_table(h: &Harness<'static, ZebraApp>) -> (String, TableData) {
    let app = h.state();
    app.editor
        .doc
        .elements
        .values()
        .find_map(|e| match &e.kind {
            ElementKind::Table(t) => Some((e.id.clone(), t.clone())),
            _ => None,
        })
        .expect("a table")
}

fn input(h: &Harness<'static, ZebraApp>, r: usize, c: usize) -> String {
    the_table(h).1.cells[r][c].input.clone()
}

fn click_at(h: &mut Harness<'static, ZebraApp>, p: Pos2, button: PointerButton) {
    h.event(Event::PointerMoved(p));
    h.step();
    h.event(Event::PointerButton { pos: p, button, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: p, button, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn cell(h: &Harness<'static, ZebraApp>, r: usize, c: usize) -> Pos2 {
    let id = the_table(h).0;
    table_ui::cell_center(h.state(), &id, r, c).expect("cell on screen")
}

fn key(h: &mut Harness<'static, ZebraApp>, k: Key, m: Modifiers) {
    h.event(Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.event(Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.run_steps(3);
}

fn type_text(h: &mut Harness<'static, ZebraApp>, s: &str) {
    h.event(Event::Text(s.into()));
    h.run_steps(3);
}

fn head(h: &Harness<'static, ZebraApp>) -> Option<(usize, usize)> {
    h.state().canvas.table.as_ref().map(|s| s.head)
}

/// A table from Insert › Table, selected, with a cell selected in it.
fn table_with_cell(h: &mut Harness<'static, ZebraApp>) {
    h.state_mut().queued.push(Action::InsertTable);
    h.run_steps(4);
    let (id, t) = the_table(h);
    assert_eq!((t.n_rows(), t.n_cols()), (5, 4));
    assert_eq!(h.state().editor.view.selection, vec![id]);
    // The second click selects a cell.
    let p = cell(h, 1, 0);
    click_at(h, p, PointerButton::Primary);
    assert_eq!(head(h), Some((1, 0)));
}

#[test]
fn the_toolbar_table_menu_inserts_a_table_in_a_style() {
    let mut h = harness();
    h.get_by_label(&tr("toolbar-table")).click();
    h.run_steps(3);
    shot(&mut h, "toolbar-table-menu.png");
    h.get_by_label("Banded").click();
    h.run_steps(4);
    let (_, t) = the_table(&h);
    assert_eq!(t.style.id(), "ts2");
    shot(&mut h, "table-selected.png");
}

#[test]
fn cells_select_type_and_move_with_tab_return_and_arrows() {
    let mut h = harness();
    table_with_cell(&mut h);
    shot(&mut h, "cell-selected.png");
    // Typing starts editing; Return accepts and moves down.
    type_text(&mut h, "North");
    assert!(table_ui::editing(h.state()));
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(input(&h, 1, 0), "North");
    assert_eq!(head(&h), Some((2, 0)));
    // Tab: the next cell; Shift-Tab back.
    key(&mut h, Key::Tab, Modifiers::NONE);
    assert_eq!(head(&h), Some((2, 1)));
    key(&mut h, Key::Tab, Modifiers::SHIFT);
    assert_eq!(head(&h), Some((2, 0)));
    // Arrows move; Shift-arrows select a range.
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    key(&mut h, Key::ArrowDown, Modifiers::SHIFT);
    let s = h.state().canvas.table.clone().unwrap();
    assert_eq!(s.range().a1(), "B3:B4");
    // Option-arrows add a row below and a column after.
    let n = the_table(&h).1.n_rows();
    key(&mut h, Key::ArrowDown, Modifiers::ALT);
    assert_eq!(the_table(&h).1.n_rows(), n + 1);
    let m = the_table(&h).1.n_cols();
    key(&mut h, Key::ArrowRight, Modifiers::ALT);
    assert_eq!(the_table(&h).1.n_cols(), m + 1);
    // Tab in the last cell adds a row.
    let (_, t) = the_table(&h);
    let last = (t.n_rows() - 1, t.n_cols() - 1);
    let p = cell(&h, last.0, last.1);
    click_at(&mut h, p, PointerButton::Primary);
    key(&mut h, Key::Tab, Modifiers::NONE);
    assert_eq!(the_table(&h).1.n_rows(), t.n_rows() + 1);
    assert_eq!(head(&h), Some((t.n_rows(), 0)));
    // Esc: the cell, then the table, then nothing.
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().canvas.table.is_none());
    assert_eq!(h.state().editor.view.selection.len(), 1);
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().editor.view.selection.is_empty());
}

#[test]
fn formulas_work_out_and_errors_show() {
    let mut h = harness();
    table_with_cell(&mut h);
    for v in ["10", "20"] {
        type_text(&mut h, v);
        key(&mut h, Key::Enter, Modifiers::NONE);
    }
    type_text(&mut h, "=SUM(A2:A3");
    // A click on a cell while writing a formula adds a reference.
    shot(&mut h, "formula-editing.png");
    type_text(&mut h, ")");
    key(&mut h, Key::Enter, Modifiers::NONE);
    let (_, t) = the_table(&h);
    assert_eq!(t.cells[3][0].value, Some(zebra_core::table::CellValue::Number(30.0)));
    type_text(&mut h, "=1/0");
    key(&mut h, Key::Enter, Modifiers::NONE);
    let (_, t) = the_table(&h);
    assert!(matches!(&t.cells[4][0].value, Some(zebra_core::table::CellValue::Error { error }) if error == "#DIV/0!"));
    shot(&mut h, "error-cell.png");
    // One undo step per cell.
    h.state_mut().editor.undo(H).unwrap();
    assert_eq!(input(&h, 4, 0), "");
}

#[test]
fn a_reference_is_added_by_clicking_a_cell() {
    let mut h = harness();
    table_with_cell(&mut h);
    type_text(&mut h, "=");
    let p = cell(&h, 2, 1);
    click_at(&mut h, p, PointerButton::Primary);
    assert_eq!(h.state().canvas.table.as_ref().unwrap().edit.as_ref().unwrap().text, "=B3");
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!table_ui::editing(h.state()));
    assert_eq!(input(&h, 1, 0), "", "Esc cancels");
}

#[test]
fn the_right_click_menu_adds_rows() {
    let mut h = harness();
    table_with_cell(&mut h);
    let n = the_table(&h).1.n_rows();
    let p = cell(&h, 2, 1);
    click_at(&mut h, p, PointerButton::Secondary);
    assert_eq!(head(&h), Some((2, 1)));
    shot(&mut h, "table-context-menu.png");
    h.get_by_label(&tr("table-add-row-above")).click();
    h.run_steps(3);
    assert_eq!(the_table(&h).1.n_rows(), n + 1);
    let id = the_table(&h).0;
    h.state_mut().editor.apply(serde_json::from_value(serde_json::json!({"op": "set_cells", "id": id, "at": "B2", "values": [[1], [3], [2]]})).unwrap(), H).unwrap();
    h.run_steps(2);
    let p = cell(&h, 2, 1);
    click_at(&mut h, p, PointerButton::Secondary);
    h.get_by_label(&tr("table-sort-descending")).click();
    h.run_steps(3);
    assert_eq!(h.state().editor.history().last().unwrap().label, tr("history-sort-table"));
}

#[test]
fn delete_clears_cells_and_the_whole_table_goes_when_selected_as_one() {
    let mut h = harness();
    table_with_cell(&mut h);
    type_text(&mut h, "x");
    key(&mut h, Key::Enter, Modifiers::NONE);
    key(&mut h, Key::ArrowUp, Modifiers::NONE);
    h.state_mut().queued.push(Action::Delete);
    h.run_steps(2);
    assert_eq!(input(&h, 1, 0), "");
    assert_eq!(h.state().editor.doc.elements.len(), 1, "the table stays");
    key(&mut h, Key::Escape, Modifiers::NONE);
    h.state_mut().queued.push(Action::Delete);
    h.run_steps(2);
    assert!(h.state().editor.doc.elements.is_empty(), "the table went");
}

#[test]
fn inspector_tabs_for_a_table() {
    let mut h = harness();
    table_with_cell(&mut h);
    let inspector_x = h.state().canvas.rect.right();
    let tab = |h: &mut Harness<'static, ZebraApp>, label: &str| {
        h.get_all_by_label(label).find(|n| n.rect().left() > inspector_x).unwrap_or_else(|| panic!("tab {label}")).click();
        h.run_steps(4);
    };
    for (label, file) in [("inspector-tab-table", "inspector-table.png"), ("inspector-tab-cell", "inspector-cell.png"), ("inspector-tab-text", "inspector-text.png"), ("inspector-tab-arrange", "inspector-arrange.png"), ("inspector-tab-path", "inspector-path.png")] {
        tab(&mut h, &tr(label));
        shot(&mut h, file);
    }
    // Table: a style from the grid, header rows.
    tab(&mut h, &tr("inspector-tab-table"));
    h.get_all_by_label("Dark").find(|n| n.rect().left() > inspector_x).unwrap().click();
    h.run_steps(3);
    assert_eq!(the_table(&h).1.style.id(), "ts10");
    // Cell: the Data Format of the selected cell.
    tab(&mut h, &tr("inspector-tab-cell"));
    h.get_by_label(&tr("table-format-automatic")).click();
    h.run_steps(3);
    h.get_by_label(&tr("table-format-percentage")).click();
    h.run_steps(3);
    let (_, t) = the_table(&h);
    assert_eq!(t.data_format(1, 0).kind, zebra_core::table::CellType::Percentage);
    // Text: bold on the selected cell.
    tab(&mut h, &tr("inspector-tab-text"));
    h.get_all_by_label(&tr("table-bold")).find(|n| n.rect().left() > inspector_x).unwrap().click();
    h.run_steps(3);
    assert!(the_table(&h).1.bold_at(1, 0));
}

#[test]
fn pasting_tab_separated_cells() {
    let mut h = harness();
    // On the empty canvas: a new table.
    assert!(table_ui::paste_as_table(h.state_mut(), "Region\tQ2\nNorth\t1240\nSouth\t980\n"));
    h.run_steps(3);
    let (_, t) = the_table(&h);
    assert_eq!((t.n_rows(), t.n_cols()), (3, 2));
    assert!(!table_ui::paste_as_table(h.state_mut(), "just words"), "plain text is not a table");
    // Into cells: from the selected cell, the table grows to take them.
    let p = cell(&h, 1, 1);
    click_at(&mut h, p, PointerButton::Primary);
    click_at(&mut h, p, PointerButton::Primary);
    let s = h.state().canvas.table.clone().expect("a cell selected");
    table_ui::paste_cells(h.state_mut(), &s, "1\t2\n3\t4\n5\t6");
    h.run_steps(3);
    let (_, t) = the_table(&h);
    assert_eq!((t.n_rows(), t.n_cols()), (4, 3));
    assert_eq!(t.cells[3][2].input, "6");
    // Over the limits: refused with a message, nothing changed.
    let huge = "1\t".repeat(1_001) + "\n";
    table_ui::paste_cells(h.state_mut(), &s, &huge);
    h.run_steps(2);
    assert_eq!(the_table(&h).1.n_cols(), 3);
    assert!(h.state().note.as_ref().is_some_and(|n| n.text.contains("1000") || n.text.contains("1,000")), "a note says why");
    // Copying out: what cells show, formulas made plain text.
    assert_eq!(table_ui::defuse("=cmd|' /C calc'!A0"), "'=cmd|' /C calc'!A0");
    assert_eq!(table_ui::defuse("-12"), "-12");
    assert_eq!(table_ui::defuse("-x"), "'-x");
}

#[test]
fn pasted_formulas_move_their_references() {
    let mut h = harness();
    table_with_cell(&mut h);
    let id = the_table(&h).0;
    h.state_mut()
        .editor
        .apply(serde_json::from_value(serde_json::json!({"op": "set_cells", "id": id, "at": "A2", "values": [[1], [2], [3], ["=SUM(A2:A4)", "=$A$2+A2"]]})).unwrap(), H)
        .unwrap();
    h.run_steps(2);
    // Copy A5:B5.
    let p = cell(&h, 4, 0);
    click_at(&mut h, p, PointerButton::Primary);
    key(&mut h, Key::ArrowRight, Modifiers::SHIFT);
    h.state_mut().queued.push(Action::Copy);
    h.run_steps(2);
    let text = h.state().table_clip.clone().expect("copied").text;
    // Two columns right: relative parts move, $ parts stay.
    let p = cell(&h, 4, 2);
    click_at(&mut h, p, PointerButton::Primary);
    let s = h.state().canvas.table.clone().unwrap();
    table_ui::paste_cells(h.state_mut(), &s, &text);
    h.run_steps(2);
    assert_eq!(input(&h, 4, 2), "=SUM(C2:C4)");
    assert_eq!(input(&h, 4, 3), "=$A$2+C2");
    // Four rows up: the range would leave the table.
    let p = cell(&h, 0, 0);
    click_at(&mut h, p, PointerButton::Primary);
    let s = h.state().canvas.table.clone().unwrap();
    table_ui::paste_cells(h.state_mut(), &s, &text);
    h.run_steps(2);
    assert_eq!(input(&h, 0, 0), "=SUM(#REF!)");
    assert_eq!(input(&h, 0, 1), "=$A$2+#REF!");
    assert!(matches!(&the_table(&h).1.cells[0][0].value, Some(zebra_core::table::CellValue::Error { .. })));
}

#[test]
fn the_formula_bar_colours_references_and_accepts_or_cancels() {
    let mut h = harness();
    table_with_cell(&mut h);
    type_text(&mut h, "=");
    let p = cell(&h, 2, 1);
    click_at(&mut h, p, PointerButton::Primary);
    type_text(&mut h, "*2+");
    // Far enough in time from the last click not to be a double click.
    h.run_steps(20);
    let p = cell(&h, 3, 2);
    click_at(&mut h, p, PointerButton::Primary);
    assert_eq!(h.state().canvas.table.as_ref().unwrap().edit.as_ref().unwrap().text, "=B3*2+C4");
    shot(&mut h, "formula-bar.png");
    // No transform handles while a cell is selected.
    assert!(table_ui::active(h.state()));
    // ✓ accepts, the cell stays selected.
    h.get_by_label(&tr("table-formula-accept")).click();
    h.run_steps(3);
    assert_eq!(input(&h, 1, 0), "=B3*2+C4");
    assert!(!table_ui::editing(h.state()));
    // × cancels.
    type_text(&mut h, "=1");
    h.get_by_label(&tr("table-formula-cancel")).click();
    h.run_steps(3);
    assert_eq!(input(&h, 1, 0), "=B3*2+C4");
    assert!(!table_ui::editing(h.state()));
}

#[test]
fn gridlines_are_four_independent_toggles() {
    let mut h = harness();
    table_with_cell(&mut h);
    let inspector_x = h.state().canvas.rect.right();
    h.get_all_by_label(&tr("inspector-tab-table")).find(|n| n.rect().left() > inspector_x).unwrap().click();
    h.run_steps(4);
    let g = |h: &Harness<'static, ZebraApp>| the_table(h).1.gridlines;
    let before = g(&h);
    h.get_by_label(&tr("table-gridlines-vertical")).click();
    h.run_steps(3);
    h.get_by_label(&tr("table-gridlines-outline")).click();
    h.run_steps(3);
    let after = g(&h).expect("gridlines set");
    assert!(after.vertical && after.outline, "both on together: {after:?} (was {before:?})");
    if after.horizontal {
        h.get_by_label(&tr("table-gridlines-horizontal")).click();
        h.run_steps(3);
        assert!(!g(&h).unwrap().horizontal && g(&h).unwrap().vertical, "row lines off alone");
    }
    // All Gridlines turns both kinds of inner line on.
    h.get_by_label(&tr("table-gridlines-all")).click();
    h.run_steps(3);
    let all = g(&h).unwrap();
    assert!(all.horizontal && all.vertical && all.outline, "{all:?}");
    shot(&mut h, "inspector-table.png");
}

/// D6 (cloud pass): Esc while typing in a cell selected the whole table,
/// so the next arrow moved the table and Delete deleted it. Like Numbers:
/// Esc cancels the typing and keeps that cell selected; a second Esc goes
/// to the table, a third to nothing.
#[test]
fn esc_while_typing_keeps_the_cell() {
    let mut h = harness();
    table_with_cell(&mut h);
    type_text(&mut h, "Draft");
    assert!(table_ui::editing(h.state()));
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!table_ui::editing(h.state()), "typing cancelled");
    assert_eq!(head(&h), Some((1, 0)), "the cell is still selected");
    assert_eq!(input(&h, 1, 0), "", "nothing typed was kept");
    // Arrows now move between cells, and Delete clears a cell, not the table.
    key(&mut h, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(head(&h), Some((2, 0)));
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().canvas.table.is_none(), "then the table");
    assert_eq!(h.state().editor.view.selection.len(), 1);
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().editor.view.selection.is_empty(), "then nothing");
}
