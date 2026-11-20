//! Charts in the app: the toolbar Chart menu and Insert › Chart, the Chart
//! Data window (typing, Tab, Return, Esc, + Row, Series in Rows), a chart
//! from a selected table (linked), and the Chart, Axis and Series tabs.
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::i18n::tr;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::chart::{ChartData, ChartLook, ChartType, SeriesIn};
use zebra_core::ElementKind;

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

fn the_chart(h: &Harness<'static, ZebraApp>) -> (String, ChartData) {
    h.state()
        .editor
        .doc
        .elements
        .values()
        .find_map(|e| match &e.kind {
            ElementKind::Chart(c) => Some((e.id.clone(), c.clone())),
            _ => None,
        })
        .expect("a chart")
}

fn input(h: &Harness<'static, ZebraApp>, r: usize, c: usize) -> String {
    the_chart(h).1.data.cells[r][c].input.clone()
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

fn last_undo(h: &Harness<'static, ZebraApp>) -> String {
    h.state().editor.history().last().unwrap().label.clone()
}

fn data_open(h: &Harness<'static, ZebraApp>) -> bool {
    h.state().chart_data.is_some()
}

fn editing(h: &Harness<'static, ZebraApp>) -> Option<(usize, usize)> {
    h.state().chart_data.as_ref().and_then(|w| w.edit.as_ref().map(|(r, c, _)| (*r, *c)))
}

#[test]
fn the_toolbar_chart_menu_adds_a_chart_and_opens_its_data() {
    let mut h = harness();
    h.get_by_label(&tr("toolbar-chart")).click();
    h.run_steps(3);
    shot(&mut h, "toolbar-chart-menu.png");
    h.get_by_label(&tr("chart-type-column")).click();
    h.run_steps(6);
    let (id, c) = the_chart(&h);
    assert_eq!(c.chart_type, ChartType::Column);
    assert_eq!(c.series_in, SeriesIn::Columns, "K12");
    assert_eq!(h.state().editor.view.selection, vec![id]);
    assert!(data_open(&h), "Chart Data opens on insert");
    assert_eq!(last_undo(&h), tr("history-add-chart"));
    shot(&mut h, "chart-data.png");
}

#[test]
fn chart_data_types_moves_and_closes_like_numbers() {
    let mut h = harness();
    h.state_mut().queued.push(Action::InsertChart(ChartType::Column));
    h.run_steps(6);
    assert!(data_open(&h));
    // A click on B4 (East, Q2) starts editing it.
    h.get_by_label("B4").click();
    h.run_steps(3);
    assert_eq!(editing(&h), Some((3, 1)));
    // Type a formula over it; Return keeps it and goes down.
    key(&mut h, Key::A, Modifiers::COMMAND);
    type_text(&mut h, "=B3*0.92");
    shot(&mut h, "chart-data-editing.png");
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(input(&h, 3, 1), "=B3*0.92");
    assert_eq!(last_undo(&h), tr("history-edit-chart-data"));
    assert_eq!(editing(&h), Some((4, 1)));
    // Tab goes across; Shift-Return back up.
    key(&mut h, Key::Tab, Modifiers::NONE);
    assert_eq!(editing(&h), Some((4, 2)));
    key(&mut h, Key::Enter, Modifiers::SHIFT);
    assert_eq!(editing(&h), Some((3, 2)));
    // Esc leaves the cell without keeping what was typed...
    key(&mut h, Key::A, Modifiers::COMMAND);
    type_text(&mut h, "999");
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(editing(&h), None);
    assert_eq!(input(&h, 3, 2), "702");
    assert!(data_open(&h));
    // ...a second Esc closes the window, and the chart stays selected.
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!data_open(&h));
    assert_eq!(h.state().editor.view.selection.len(), 1);
    // One undo step per cell.
    h.state_mut().queued.push(Action::Undo);
    h.run_steps(3);
    assert_eq!(input(&h, 3, 1), "760");
}

#[test]
fn chart_data_buttons_add_rows_and_read_series_in_rows() {
    let mut h = harness();
    h.state_mut().queued.push(Action::InsertChart(ChartType::Line));
    h.run_steps(6);
    h.get_by_label(&tr("chart-data-row")).click();
    h.run_steps(3);
    assert_eq!(the_chart(&h).1.data.n_rows(), 6);
    h.get_by_label(&tr("chart-data-column")).click();
    h.run_steps(3);
    assert_eq!(the_chart(&h).1.data.n_cols(), 4);
    h.get_by_label(&tr("chart-data-series-rows")).click();
    h.run_steps(3);
    assert_eq!(the_chart(&h).1.series_in, SeriesIn::Rows);
    shot(&mut h, "chart-data-series-rows.png");
}

#[test]
fn a_chart_from_a_selected_table_is_linked_and_its_data_is_read_only() {
    let mut h = harness();
    h.state_mut().queued.push(Action::InsertTable);
    h.run_steps(4);
    let table = h.state().editor.view.selection[0].clone();
    for (at, v) in [("A2", "North"), ("A3", "South"), ("B1", "Q2"), ("B2", "12"), ("B3", "18"), ("C1", "Q3"), ("C2", "15"), ("C3", "9")] {
        let cmd = serde_json::from_value(serde_json::json!({"op": "set_cells", "id": table, "at": at, "values": [[v]]})).unwrap();
        h.state_mut().apply(cmd);
    }
    h.state_mut().canvas.table = None;
    h.state_mut().select(vec![table.clone()]);
    h.run_steps(2);
    h.state_mut().queued.push(Action::InsertChart(ChartType::Bar));
    h.run_steps(6);
    let (id, c) = the_chart(&h);
    assert_eq!(c.linked_table(), Some(table.as_str()));
    assert_eq!(h.state().editor.view.selection, vec![id.clone()]);
    zebra_app::chart_ui::open_data(h.state_mut(), &id);
    h.run_steps(4);
    shot(&mut h, "chart-data-linked.png");
    // Linked cells don't open for typing; the note and Unlink are there.
    h.get_by_label("B2").click();
    h.run_steps(3);
    assert_eq!(editing(&h), None);
    h.get_all_by_label(&tr("chart-data-unlink")).last().unwrap().click();
    h.run_steps(3);
    let (_, c) = the_chart(&h);
    assert!(c.link.is_none());
    assert_eq!(c.data.cells[1][1].input, "12", "unlinking keeps what it showed");
    assert_eq!(last_undo(&h), tr("history-unlink-chart"));
}

#[test]
fn inspector_tabs_for_a_chart() {
    let mut h = harness();
    h.state_mut().queued.push(Action::InsertChart(ChartType::Column));
    h.run_steps(6);
    key(&mut h, Key::Escape, Modifiers::NONE);
    let inspector_x = h.state().canvas.rect.right();
    let tab = |h: &mut Harness<'static, ZebraApp>, label: &str| {
        h.get_all_by_label(label).find(|n| n.rect().left() > inspector_x).unwrap_or_else(|| panic!("tab {label}")).click();
        h.run_steps(4);
    };
    for (label, file) in [("inspector-tab-chart", "inspector-chart.png"), ("inspector-tab-axis", "inspector-axis.png"), ("inspector-tab-series", "inspector-series.png")] {
        tab(&mut h, &tr(label));
        shot(&mut h, file);
    }
    // Chart: a look from the swatches is one step, named.
    tab(&mut h, &tr("inspector-tab-chart"));
    h.get_by_label(&tr("chart-look-rounded")).click();
    h.run_steps(3);
    assert_eq!(the_chart(&h).1.look, ChartLook::Rounded);
    assert_eq!(last_undo(&h), tr("history-change-chart-style"));
    // The type from its menu.
    h.get_by_label(&tr("chart-type-column")).click();
    h.run_steps(3);
    h.get_by_label(&tr("chart-type-donut")).click();
    h.run_steps(3);
    assert_eq!(the_chart(&h).1.chart_type, ChartType::Donut);
    assert_eq!(last_undo(&h), tr("history-change-chart-type"));
    shot(&mut h, "inspector-chart-donut.png");
    // Edit Chart Data opens the window.
    h.get_by_label(&tr("chart-edit-data")).click();
    h.run_steps(3);
    assert!(data_open(&h));
    // Double-click on the canvas opens it too.
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!data_open(&h));
    // Axis: the gridlines box.
    h.state_mut().queued.push(Action::Undo);
    h.run_steps(3);
    tab(&mut h, &tr("inspector-tab-axis"));
    h.get_by_label(&tr("chart-gridlines")).click();
    h.run_steps(3);
    assert!(!the_chart(&h).1.gridlines);
    assert_eq!(last_undo(&h), tr("history-format-chart"));
}

#[test]
fn every_type_and_look_has_a_name_and_insert_chart_lists_them() {
    for t in ChartType::ALL {
        let label = Action::InsertChart(*t).label();
        assert!(!label.is_empty() && !label.contains('{'), "{label}");
    }
    for l in ChartLook::ALL {
        assert!(zebra_app::i18n::LOADER.has(&format!("chart-look-{}", l.key())));
    }
}

#[test]
fn double_click_on_a_chart_opens_its_data() {
    let mut h = harness();
    h.state_mut().queued.push(Action::InsertChart(ChartType::Pie));
    h.run_steps(6);
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!data_open(&h));
    let id = the_chart(&h).0;
    let app = h.state();
    let e = app.editor.doc.get(&id).unwrap();
    let s = app.editor.view.camera.world_to_screen(zebra_core::Vec2::new(e.transform.x, e.transform.y), app.editor.view.viewport);
    let p = egui::pos2(app.canvas.rect.left() + s.x as f32, app.canvas.rect.top() + s.y as f32);
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
    assert!(data_open(&h));
}

/// No silent limits: past what a chart draws, the inspector says what it
/// left out; a legend over its entries and a too-fine axis step say so too.
#[test]
fn limits_are_said_in_the_inspector() {
    let mut h = harness();
    let mut header = vec![serde_json::json!("")];
    let mut row = vec![serde_json::json!("North")];
    for i in 0..70 {
        header.push(serde_json::json!(format!("S{i}")));
        row.push(serde_json::json!(i));
    }
    let cmd = serde_json::from_value(serde_json::json!({"op": "add_chart", "data": [header, row]})).unwrap();
    let out = h.state_mut().apply(cmd).unwrap();
    let id = out.created[0].clone();
    let fmt = serde_json::from_value(serde_json::json!({"op": "format_chart", "id": id, "axis_step": 0.0001})).unwrap();
    h.state_mut().apply(fmt).unwrap();
    h.state_mut().select(vec![id.clone()]);
    h.run_steps(4);
    let plot = the_chart(&h).1.plot(&h.state().editor.doc);
    assert!(plot.cut);
    let note = zebra_app::chart_ui::cut_note(&plot);
    assert!(note.contains("64 of 70"), "{note}");
    assert!(h.query_by_label(&note).is_some(), "the Chart tab says what's left out");
    assert!(h.query_by_label_contains("legend lists the first 8").is_some(), "the legend's limit is said");
    shot(&mut h, "inspector-chart-limits.png");
    let inspector_x = h.state().canvas.rect.right();
    h.get_all_by_label(&tr("inspector-tab-axis")).find(|n| n.rect().left() > inspector_x).unwrap().click();
    h.run_steps(4);
    assert!(h.query_by_label_contains("so the axis steps automatically").is_some(), "a too-fine step is said");
    shot(&mut h, "inspector-axis-limits.png");
}

/// The mini UX pass in light mode: the chart tabs and the Chart Data
/// window (screenshots with ZEBRA_SHOTS).
#[test]
fn chart_screens_in_light_mode() {
    let mut h = harness();
    h.ctx.set_visuals(egui::Visuals::light());
    h.run_steps(2);
    h.state_mut().queued.push(Action::InsertChart(ChartType::Line));
    h.run_steps(6);
    assert!(data_open(&h));
    shot(&mut h, "light-chart-data.png");
    key(&mut h, Key::Escape, Modifiers::NONE);
    let inspector_x = h.state().canvas.rect.right();
    for (label, file) in [("inspector-tab-chart", "light-inspector-chart.png"), ("inspector-tab-axis", "light-inspector-axis.png"), ("inspector-tab-series", "light-inspector-series.png")] {
        h.get_all_by_label(&tr(label)).find(|n| n.rect().left() > inspector_x).unwrap_or_else(|| panic!("tab {label}")).click();
        h.run_steps(4);
        shot(&mut h, file);
    }
    // The Chart Style swatches show the chart's own type.
    h.get_all_by_label(&tr("inspector-tab-chart")).find(|n| n.rect().left() > inspector_x).unwrap().click();
    h.run_steps(4);
    shot(&mut h, "light-inspector-chart-line.png");
}
