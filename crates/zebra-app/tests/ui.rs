//! UI interaction tests (egui_kittest, no GPU: the canvas uses the fallback painter).
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, ElementKind, Mode, Origin, ShapeKind, Tool};

fn harness(demo: Option<&str>) -> Harness<'static, ZebraApp> {
    let demo = demo.map(|s| s.to_string());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(move |cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo, ..Default::default() }));
    h.run_steps(3);
    h
}

fn click_at(h: &mut Harness<'static, ZebraApp>, p: Pos2, count: usize) {
    h.event(Event::PointerMoved(p));
    h.step();
    for _ in 0..count {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.step();
    }
}

fn drag(h: &mut Harness<'static, ZebraApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=10 {
        let p = from + (to - from) * (i as f32 / 10.0);
        h.event(Event::PointerMoved(p));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    h.step();
}

#[test]
fn double_click_empty_canvas_writes_text() {
    let mut h = harness(Some("empty"));
    let c = h.state().canvas.rect.center();
    click_at(&mut h, c, 2);
    h.run_steps(2);
    assert!(h.state().canvas.text.is_some(), "double-click should open an in-place text box");
    h.event(Event::Text("Hello zebra".into()));
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    let app = h.state();
    assert!(app.canvas.text.is_none());
    let texts: Vec<_> = app
        .editor
        .doc
        .ordered()
        .filter_map(|e| match &e.kind {
            ElementKind::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["Hello zebra".to_string()]);
    assert_eq!(app.editor.history().len(), 1, "creating text is one undo step");
    // The new text sits where we clicked and reads at body size on screen.
    let e = app.editor.doc.ordered().next().unwrap();
    let s = app.canvas.w2s(&app.editor, e.transform.center());
    assert!(s.distance(c) < 120.0, "text near the click: {s:?} vs {c:?}");
}

#[test]
fn dragging_moves_as_one_undo_step() {
    let mut h = harness(Some("empty"));
    let id = {
        let app = h.state_mut();
        let out = app.editor.apply(Command::AddShape { shape: ShapeKind::Rectangle, x: 0.0, y: 0.0, w: 200.0, h: 120.0, rotation: 0.0, fill: None, stroke: None, id: None }, Origin::Human).unwrap();
        out.created[0].clone()
    };
    h.run_steps(2);
    let (start, zoom, hist) = {
        let app = h.state();
        (app.canvas.w2s(&app.editor, zebra_core::Vec2::new(0.0, 0.0)), app.editor.view.camera.zoom, app.editor.history().len())
    };
    drag(&mut h, start, start + egui::vec2(150.0, 60.0));
    let app = h.state();
    let e = app.editor.doc.get(&id).unwrap();
    assert!((e.transform.x - 150.0 / zoom).abs() < 2.0 / zoom, "x moved: {}", e.transform.x);
    assert!((e.transform.y - 60.0 / zoom).abs() < 2.0 / zoom, "y moved: {}", e.transform.y);
    assert_eq!(app.editor.history().len(), hist + 1, "a drag is a single undo step");
    assert_eq!(app.editor.view.selection, vec![id.clone()]);
    // ⌘Z puts it back.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    let e = h.state().editor.doc.get(&id).unwrap();
    assert!(e.transform.x.abs() < 1e-9 && e.transform.y.abs() < 1e-9);
}

#[test]
fn present_mode_keys() {
    let mut h = harness(Some("sample"));
    h.key_press(Key::F5);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.mode, Mode::Present);
    assert_eq!(h.state().editor.view.present.step, Some(0));
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.step, Some(1));
    // step 2 has three fade-in builds: Space reveals them one at a time
    h.key_press(Key::Space);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.build, 1);
    h.key_press(Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.build, 0);
    h.key_press(Key::B);
    h.run_steps(2);
    assert!(h.state().editor.view.present.blank.is_some());
    h.key_press(Key::B);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.mode, Mode::Edit);
    assert_eq!(h.state().selected_step, Some(1), "Esc returns to the editor at the current step");
}

#[test]
fn tool_shortcuts_and_frame_cycle() {
    let mut h = harness(Some("empty"));
    h.key_press(Key::R);
    h.run_steps(1);
    assert_eq!(h.state().editor.view.tool, Tool::Rectangle);
    h.key_press(Key::F);
    h.run_steps(1);
    assert_eq!(h.state().editor.view.tool, Tool::Frame);
    let s0 = h.state().editor.view.frame_style;
    h.key_press(Key::F);
    h.run_steps(1);
    assert_eq!(h.state().editor.view.frame_style, s0.next());
    // Drawing a frame around a text wraps it.
    {
        let app = h.state_mut();
        app.editor.apply(Command::AddText { text: "inside".into(), x: 0.0, y: 0.0, style: Default::default(), scale: Some(1.0), place: None, id: None, markdown: false, on: None, color: None }, Origin::Human).unwrap();
    }
    h.run_steps(1);
    let (a, b) = {
        let app = h.state();
        (app.canvas.w2s(&app.editor, zebra_core::Vec2::new(-200.0, -120.0)), app.canvas.w2s(&app.editor, zebra_core::Vec2::new(200.0, 120.0)))
    };
    drag(&mut h, a, b);
    let app = h.state();
    let frame = app.editor.doc.ordered().find(|e| e.is_frame()).expect("frame created");
    assert_eq!(app.editor.doc.frame_contents(&frame.id).len(), 1);
    assert_eq!(app.editor.view.tool, Tool::Select, "tool returns to select after drawing");
}

#[test]
fn path_edit_click_adds_steps() {
    let mut h = harness(Some("sample"));
    let n0 = h.state().editor.doc.path.len();
    h.key_press(Key::P);
    h.run_steps(1);
    assert_eq!(h.state().editor.view.mode, Mode::PathEdit);
    // click inside the "A new angle" frame
    let p = {
        let app = h.state();
        let f = app.editor.doc.ordered().find(|e| e.name == "A new angle").unwrap();
        app.canvas.w2s(&app.editor, f.transform.center() + zebra_core::Vec2::new(0.0, 200.0))
    };
    click_at(&mut h, p, 1);
    h.run_steps(2);
    assert_eq!(h.state().editor.doc.path.len(), n0 + 1);
    h.key_press(Key::Escape);
    h.run_steps(1);
    assert_eq!(h.state().editor.view.mode, Mode::Edit);
}

#[test]
fn typing_a_new_text_stays_on_one_line() {
    let mut h = harness(Some("empty"));
    let c = h.state().canvas.rect.center();
    click_at(&mut h, c, 2);
    h.run_steps(2);
    for ch in "Why cities grow".chars() {
        h.event(Event::Text(ch.to_string()));
        h.step();
    }
    h.run_steps(2);
    let sz = h.state().canvas.text.as_ref().unwrap().size_px;
    assert!(sz.x > sz.y * 4.0, "text box wraps while typing: {sz:?}");
}

#[test]
fn the_inspector_stays_put_when_the_selection_changes() {
    // Critique item 5: the inspector is a persistent pane (⌥⌘I), so
    // selecting or deselecting never reflows the canvas.
    let mut h = harness(Some("sample"));
    h.run_steps(10);
    let before = h.state().canvas.rect;
    let id = h.state().editor.doc.ordered().find(|e| e.is_frame()).unwrap().id.clone();
    h.state_mut().editor.view(zebra_core::ViewCommand::Select { ids: vec![id] }, Origin::Human).unwrap();
    h.run_steps(5);
    assert_eq!(h.state().canvas.rect, before, "selecting moved the canvas");
    h.state_mut().editor.view(zebra_core::ViewCommand::ClearSelection, Origin::Human).unwrap();
    h.run_steps(5);
    assert_eq!(h.state().canvas.rect, before, "deselecting moved the canvas");
    // ⌥⌘I hides it; the canvas grows by the inspector's width.
    h.key_press_modifiers(Modifiers { alt: true, ..Modifiers::COMMAND }, Key::I);
    h.run_steps(5);
    assert!(!h.state().show_props);
    assert!(h.state().canvas.rect.width() > before.width() + 200.0);
}

#[test]
fn leaving_present_mode_after_a_window_resize_keeps_view_centred() {
    let mut h = harness(Some("sample"));
    h.run_steps(10);
    let centre_world = |h: &Harness<'static, ZebraApp>| {
        let app = h.state();
        app.canvas.s2w(&app.editor, app.canvas.rect.center())
    };
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(5);
    assert_eq!(h.state().editor.view.mode, Mode::Present);
    h.key_press(Key::Escape);
    h.run_steps(5);
    let before = centre_world(&h);
    // Leaving full screen shrinks the window over a few frames.
    h.set_size(egui::vec2(1200.0, 760.0));
    h.run_steps(3);
    h.set_size(egui::vec2(1100.0, 700.0));
    h.run_steps(3);
    let after = centre_world(&h);
    let zoom = h.state().editor.view.camera.zoom;
    assert!(((after.x - before.x) * zoom).abs() < 2.0 && ((after.y - before.y) * zoom).abs() < 2.0, "view drifted: {before:?} -> {after:?}");
}

#[test]
fn leaving_present_mode_fits_the_current_step_in_the_editor() {
    let mut h = harness(Some("sample"));
    h.run_steps(10);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(5);
    h.key_press(Key::ArrowRight);
    h.key_press(Key::ArrowRight);
    h.run_steps(60);
    let step = h.state().editor.view.present.step.unwrap();
    h.key_press(Key::Escape);
    h.run_steps(5);
    let app = h.state();
    assert_eq!(app.editor.view.mode, Mode::Edit);
    let want = app.editor.camera_for_step(step);
    let cam = app.editor.view.camera;
    assert!((cam.zoom - want.zoom).abs() / want.zoom < 0.01, "zoom {} vs {}", cam.zoom, want.zoom);
}

#[test]
fn present_hud_steps_with_the_mouse() {
    let mut h = harness(Some("sample"));
    h.run_steps(5);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(3);
    let before = h.state().editor.view.present.step;
    h.get_by_label("Next Step").click();
    h.run_steps(3);
    assert_ne!(h.state().editor.view.present.step, before, "› advanced");
    h.get_by_label("Previous Step").click();
    h.run_steps(3);
    assert_eq!(h.state().editor.view.present.step, before, "‹ went back");
}

#[test]
fn present_on_an_empty_document_is_refused() {
    let mut h = harness(Some("empty"));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(5);
    assert_eq!(h.state().editor.view.mode, Mode::Edit);
}

#[test]
fn ai_control_is_switched_on_by_the_human_and_off_by_an_agent() {
    let dir = std::env::temp_dir().join(format!("zebra-ai-toggle-{}", std::process::id()));
    // SAFETY: no other test in this binary reads or writes the environment.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    let mut h = harness(Some("empty"));
    assert!(h.state().api.is_none(), "AI control must be off by default");
    assert!(zebra_api::discovery::list().is_empty());

    // Human: click the AI toolbar item, then the switch.
    h.get_by_label("AI").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Switch, "Let AI agents control Zebra").click();
    h.run_steps(3);
    assert!(h.state().ai_enabled && h.state().api.is_some(), "the switch should turn AI control on");
    let socket = h.state().api.as_ref().unwrap().socket_path();
    assert_eq!(zebra_api::discovery::list().len(), 1, "a running agent must be able to find Zebra");

    // Agent: discover, then switch it off. There is no method to switch it on.
    let t = std::thread::spawn(move || {
        let mut c = zebra_api::client::Client::discover(None, None, None, "test-agent").unwrap();
        assert!(c.call("enable", serde_json::json!({})).is_err());
        c.call("disconnect", serde_json::json!({})).unwrap()
    });
    let start = std::time::Instant::now();
    while h.state().api.is_some() && start.elapsed().as_secs() < 10 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let reply = t.join().unwrap();
    assert_eq!(reply["ok"], true);
    assert!(h.state().api.is_none() && !h.state().ai_enabled, "disconnect should switch AI control off");
    assert!(!socket.exists(), "socket should be removed");
    assert!(zebra_api::discovery::list().is_empty(), "discovery file should be removed");
    let err = zebra_api::client::Client::discover(None, None, None, "test-agent").err().unwrap().to_string();
    assert!(err.contains("AI control") && err.contains("Ask the person"), "{err}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rich_text_bold_selection_and_list_from_keyboard() {
    let mut h = harness(Some("empty"));
    let c = h.state().canvas.rect.center();
    click_at(&mut h, c, 2);
    h.run_steps(2);
    h.event(Event::Text("Hello world".into()));
    h.run_steps(2);
    // Select the last word with ⇧⌥← and make it bold.
    h.event(Event::Key { key: Key::ArrowLeft, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::SHIFT | Modifiers::ALT });
    h.run_steps(2);
    h.event(Event::Key { key: Key::B, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.run_steps(2);
    // Tab makes the paragraph a deeper indent level.
    h.event(Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    let app = h.state();
    let t = app
        .editor
        .doc
        .ordered()
        .find_map(|e| match &e.kind {
            ElementKind::Text(t) => Some(t.clone()),
            _ => None,
        })
        .expect("a text was created");
    assert_eq!(t.text, "Hello world");
    assert_eq!(t.spans.len(), 1, "{:?}", t.spans);
    assert_eq!((t.spans[0].start, t.spans[0].end), (6, 11), "only the selected word is bold");
    assert_eq!(t.spans[0].format.bold, Some(true));
    assert!(!t.bold, "the box itself is not bold");
    assert_eq!(t.paras.first().map(|p| p.indent), Some(1));
}

#[test]
fn library_inserts_shapes_and_icons_and_replaces_an_icon() {
    let mut h = harness(Some("empty"));
    h.state_mut().queued.push(zebra_app::actions::Action::ShapeLibrary);
    h.run_steps(3);
    assert!(h.state().library.open);
    h.get_by_label("Burst").click();
    h.run_steps(3);
    h.get_by_label("Icons").click();
    h.run_steps(2);
    h.event(Event::Text("rocket".into()));
    h.run_steps(3);
    h.get_by_label("rocket").click();
    h.run_steps(3);
    let app = h.state();
    let kinds: Vec<String> = app
        .editor
        .doc
        .ordered()
        .map(|e| match &e.kind {
            ElementKind::Shape(s) => format!("{:?}", s.shape),
            ElementKind::Icon(i) => i.name.clone(),
            k => k.type_name().into(),
        })
        .collect();
    assert_eq!(kinds, ["Burst", "rocket"]);
    let icon = app.editor.view.selection.first().cloned().expect("the new icon is selected");
    // Replace it from the inspector's "Replace…".
    h.state_mut().library.open_replace(icon.clone());
    h.run_steps(2);
    h.event(Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.event(Event::Text("lightbulb".into()));
    h.run_steps(3);
    h.get_by_label("lightbulb").click();
    h.run_steps(3);
    let app = h.state();
    assert!(!app.library.open, "replacing closes the picker");
    match &app.editor.doc.get(&icon).unwrap().kind {
        ElementKind::Icon(i) => assert_eq!(i.name, "lightbulb"),
        k => panic!("{k:?}"),
    }
}

#[test]
fn arrow_drawn_between_two_shapes_sticks_to_both() {
    let mut h = harness(Some("empty"));
    for (id, x) in [("a", -300.0), ("b", 300.0)] {
        h.state_mut()
            .editor
            .apply(Command::AddShape { shape: ShapeKind::Rectangle, x, y: 0.0, w: 160.0, h: 120.0, rotation: 0.0, fill: None, stroke: None, id: Some(id.into()) }, Origin::Human)
            .unwrap();
    }
    h.run_steps(2);
    let (pa, pb) = {
        let app = h.state();
        (app.canvas.w2s(&app.editor, zebra_core::Vec2::new(-300.0, 0.0)), app.canvas.w2s(&app.editor, zebra_core::Vec2::new(300.0, 0.0)))
    };
    let _ = h.state_mut().editor.view(zebra_core::ViewCommand::SetTool { tool: Tool::Arrow }, Origin::Human);
    drag(&mut h, pa, pb);
    h.run_steps(2);
    let app = h.state();
    let c = app.editor.doc.ordered().find_map(|e| match &e.kind {
        ElementKind::Connector(c) => Some(c.clone()),
        _ => None,
    });
    let c = c.expect("an arrow between two shapes becomes a connector");
    assert_eq!((c.from.element.as_deref(), c.to.element.as_deref()), (Some("a"), Some("b")));
    // Moving b keeps it attached.
    h.state_mut().editor.apply(Command::Move { ids: vec!["b".into()], dx: 0.0, dy: 300.0, with_contents: false }, Origin::Human).unwrap();
    let app = h.state();
    let e = app.editor.doc.ordered().find(|e| matches!(e.kind, ElementKind::Connector(_))).unwrap();
    assert!(e.bounds().y1 > 200.0, "the connector followed b down: {:?}", e.bounds());
}

#[test]
fn unsaved_changes_ask_with_a_sheet() {
    let mut h = harness(Some("sample"));
    h.run_steps(5);
    // Make the document dirty, then File › New.
    let id = h.state().editor.doc.ordered().next().unwrap().id.clone();
    h.state_mut().editor.apply(Command::Move { ids: vec![id], dx: 5.0, dy: 0.0, with_contents: true }, Origin::Human).unwrap();
    h.state_mut().queued.push(zebra_app::actions::Action::New);
    h.run_steps(3);
    assert!(matches!(h.state().sheet, Some(zebra_app::sheets::SheetKind::SaveChanges(_))));
    // Cancel keeps the document.
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().sheet.is_none());
    assert!(!h.state().editor.doc.elements.is_empty());
    // Again, and Don't Save discards it.
    h.state_mut().queued.push(zebra_app::actions::Action::New);
    h.run_steps(3);
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert!(h.state().sheet.is_none());
    assert!(h.state().editor.doc.elements.is_empty(), "Don't Save should go ahead with New");
}

#[test]
fn clear_path_confirms_first() {
    let mut h = harness(Some("sample"));
    h.run_steps(5);
    let n = h.state().editor.doc.path.len();
    assert!(n > 0);
    h.state_mut().queued.push(zebra_app::actions::Action::ClearPath);
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.path.len(), n, "nothing cleared before the answer");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.path.len(), n, "Esc cancels");
    h.state_mut().queued.push(zebra_app::actions::Action::ClearPath);
    h.run_steps(3);
    h.get_by_label("Clear Path").click();
    h.run_steps(3);
    assert!(h.state().editor.doc.path.is_empty());
    assert!(h.state().editor.can_undo(), "clearing the path can be undone");
}

#[test]
fn inspector_tabs_follow_the_selection() {
    use zebra_app::inspector::Tab;
    let mut h = harness(Some("sample"));
    h.run_steps(5);
    let text = h.state().editor.doc.ordered().find(|e| matches!(e.kind, ElementKind::Text(_))).unwrap().id.clone();
    let frame = h.state().editor.doc.ordered().find(|e| e.is_frame()).unwrap().id.clone();
    h.state_mut().select(vec![text]);
    h.run_steps(3);
    // Text opens on its Text tab; pick Arrange (the tab bar's button comes
    // after the menu bar's "Arrange" menu).
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Style").is_none(), "text has no Style tab");
    h.query_all_by_role_and_label(egui::accesskit::Role::Button, "Arrange").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().inspector_tab, Tab::Arrange);
    // The tab is kept for a frame (which has Arrange too)...
    h.state_mut().select(vec![frame.clone()]);
    h.run_steps(3);
    assert!(h.query_by_label("Layer").is_some(), "Arrange tab content shown");
    // ...and its Style tab's segmented control changes the frame style
    // through the command bus.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Style").click();
    h.run_steps(3);
    h.get_by_label("Circle").click();
    h.run_steps(3);
    let e = h.state().editor.doc.get(&frame).unwrap().clone();
    assert!(matches!(&e.kind, ElementKind::Frame(f) if f.style == zebra_core::FrameStyle::Circle));
    assert!(h.state().editor.can_undo());
}

#[test]
fn menu_titles_follow_state() {
    use zebra_app::actions::Action;
    use zebra_app::menus::state;
    let mut h = harness(Some("sample"));
    h.run_steps(5);
    assert_eq!(state(h.state(), &Action::ToggleProperties).title, "Hide Inspector");
    assert_eq!(state(h.state(), &Action::ToggleSidebar).title, "Hide Sidebar");
    assert!(!state(h.state(), &Action::Undo).enabled);
    let id = h.state().editor.doc.ordered().next().unwrap().id.clone();
    h.state_mut().editor.apply(Command::Update { id, patch: zebra_core::ElementPatch { rotation: Some(12.0), ..Default::default() } }, Origin::Human).unwrap();
    let undo = state(h.state(), &Action::Undo);
    assert!(undo.enabled && undo.title.starts_with("Undo ") && undo.title.len() > 5, "{undo:?}");
    assert!(!state(h.state(), &Action::PauseAgents).enabled, "agent items are greyed out with no agent connected");
}

#[test]
fn grouped_shapes_select_and_drag_together() {
    let mut h = harness(Some("empty"));
    for (id, x) in [("a", -300.0), ("b", 300.0)] {
        h.state_mut()
            .editor
            .apply(Command::AddShape { shape: ShapeKind::Rectangle, x, y: 0.0, w: 160.0, h: 120.0, rotation: 0.0, fill: None, stroke: None, id: Some(id.into()) }, Origin::Human)
            .unwrap();
    }
    h.state_mut().select(vec!["a".into(), "b".into()]);
    h.state_mut().queued.push(zebra_app::actions::Action::Group);
    h.run_steps(3);
    assert!(h.state().editor.doc.get("a").unwrap().group.is_some(), "Arrange › Group groups the selection");
    let _ = h.state_mut().editor.view(zebra_core::ViewCommand::ClearSelection, Origin::Human);
    h.run_steps(2);
    let pa = { let app = h.state(); app.canvas.w2s(&app.editor, zebra_core::Vec2::new(-300.0, 0.0)) };
    click_at(&mut h, pa, 1);
    h.run_steps(2);
    let mut sel = h.state().editor.view.selection.clone();
    sel.sort();
    assert_eq!(sel, vec!["a".to_string(), "b".to_string()], "clicking one member selects the group");
    drag(&mut h, pa, pa + egui::vec2(0.0, 80.0));
    h.run_steps(2);
    let (ya, yb) = { let d = &h.state().editor.doc; (d.get("a").unwrap().transform.y, d.get("b").unwrap().transform.y) };
    assert!(ya > 1.0 && (ya - yb).abs() < 1e-6, "both moved down together: {ya} {yb}");
    h.state_mut().queued.push(zebra_app::actions::Action::Ungroup);
    h.run_steps(3);
    assert!(h.state().editor.doc.get("b").unwrap().group.is_none());
}

#[test]
fn the_editor_stops_repainting_at_rest() {
    // F26: with nothing animating, no frame asks for another one.
    let mut h = harness(Some("sample"));
    h.run_steps(5);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    match h.try_run() {
        Ok(_) => {}
        Err(e) => panic!("still repainting at rest: {e}"),
    }
}

#[test]
fn double_click_a_shape_to_type_on_it() {
    let mut h = harness(Some("empty"));
    h.state_mut()
        .editor
        .apply(Command::AddShape { shape: ShapeKind::StickyNote, x: 0.0, y: 0.0, w: 300.0, h: 300.0, rotation: 0.0, fill: None, stroke: None, id: Some("s".into()) }, Origin::Human)
        .unwrap();
    h.run_steps(2);
    let p = { let app = h.state(); app.canvas.w2s(&app.editor, zebra_core::Vec2::new(0.0, 0.0)) };
    click_at(&mut h, p, 2);
    h.run_steps(2);
    h.event(Event::Text("Ship the beta on Friday".into()));
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    let app = h.state();
    let t = app.editor.doc.ordered().find(|e| matches!(e.kind, ElementKind::Text(_))).expect("text was added").clone();
    let ElementKind::Text(td) = &t.kind else { unreachable!() };
    assert_eq!(td.text, "Ship the beta on Friday");
    assert!(td.fixed_width && td.align == zebra_core::Align::Center, "wraps inside the note, centred");
    assert!(t.transform.center().len() < 1e-6, "centred on the note: {:?}", t.transform.center());
    let s = app.editor.doc.get("s").unwrap();
    assert!(s.group.is_some() && s.group == t.group, "grouped with the note");
    let b = t.bounds();
    assert!(b.x1 - b.x0 <= 300.0, "fits the note's width: {b:?}");
    // Moving the note carries its text; double-clicking again edits it.
    let tid = t.id.clone();
    h.run_steps(30); // past the double-click window
    click_at(&mut h, p, 2);
    h.run_steps(2);
    assert_eq!(h.state().canvas.text.as_ref().and_then(|e| e.id.clone()), Some(tid));
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().editor.doc.ordered().filter(|e| matches!(e.kind, ElementKind::Text(_))).count(), 1);
}

#[test]
fn right_click_menu_selects_and_acts() {
    let mut h = harness(Some("empty"));
    for (id, x) in [("a", -300.0), ("b", 300.0)] {
        h.state_mut()
            .editor
            .apply(Command::AddShape { shape: ShapeKind::Rectangle, x, y: 0.0, w: 160.0, h: 120.0, rotation: 0.0, fill: None, stroke: None, id: Some(id.into()) }, Origin::Human)
            .unwrap();
    }
    h.run_steps(2);
    let right_click = |h: &mut Harness<'static, ZebraApp>, p: Pos2| {
        h.event(Event::PointerMoved(p));
        h.step();
        h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
    };
    let pb = { let app = h.state(); app.canvas.w2s(&app.editor, zebra_core::Vec2::new(300.0, 0.0)) };
    right_click(&mut h, pb);
    assert_eq!(h.state().editor.view.selection, vec!["b".to_string()], "right-click selects what's under the pointer");
    h.get_by_label("Add Text").click();
    h.run_steps(3);
    assert!(h.state().canvas.text.as_ref().is_some_and(|t| t.host.as_deref() == Some("b")));
    h.key_press(Key::Escape);
    h.run_steps(2);
    // With both selected, the menu groups them.
    h.state_mut().select(vec!["a".into(), "b".into()]);
    h.run_steps(2);
    right_click(&mut h, pb);
    assert_eq!(h.state().editor.view.selection.len(), 2, "right-clicking inside the selection keeps it");
    // The inspector has a Group button too: click the one in the menu,
    // which opens at the pointer.
    h.get_all_by_label("Group").min_by(|a, b| (a.rect().center() - pb).length().total_cmp(&(b.rect().center() - pb).length())).unwrap().click();
    h.run_steps(3);
    assert!(h.state().editor.doc.get("a").unwrap().group.is_some());
}

#[test]
fn web_export_sheet_offers_the_browser() {
    use zebra_app::sheets::SheetKind;
    let mut h = harness(Some("sample"));
    h.run_steps(5);
    // The toolbar's Export menu lists the web page.
    h.get_by_label("Export").click();
    h.run_steps(3);
    h.get_by_label("Web Page (HTML)…");
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        h.render().unwrap().save(std::path::Path::new(&dir).join("export-menu.png")).unwrap();
    }
    h.key_press(Key::Escape);
    h.run_steps(3);
    for folder in [false, true] {
        let page = if folder { "/tmp/Talk/index.html" } else { "/tmp/Talk.html" };
        zebra_app::sheets::request(h.state_mut(), SheetKind::WebExported { page: page.into(), folder });
        h.run_steps(3);
        h.get_by_label("Open in Browser");
        let title = if folder { "Exported “Talk”" } else { "Exported “Talk.html”" };
        h.get_by_label(title);
        if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
            h.render().unwrap().save(std::path::Path::new(&dir).join(format!("export-sheet{}.png", if folder { "-folder" } else { "" }))).unwrap();
        }
        h.get_by_label("Done").click();
        h.run_steps(3);
        assert!(h.state().sheet.is_none());
    }
}
