//! UI interaction tests (egui_kittest, no GPU: the canvas uses the fallback painter).
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
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
        app.editor.apply(Command::AddText { text: "inside".into(), x: 0.0, y: 0.0, style: Default::default(), scale: Some(1.0), place: None, id: None }, Origin::Human).unwrap();
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
