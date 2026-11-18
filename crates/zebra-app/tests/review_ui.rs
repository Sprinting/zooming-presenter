//! Regression tests for the app-side bugs of the adversarial review
//! (18 Nov 2026). The core-side ones are in zebra-core/tests/review.rs.
use egui::{Event, Key, Modifiers, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::shell::Shell;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::view::Permission;
use zebra_core::{Command, Mode, Origin, StepTarget, ViewCommand};

const H: Origin = Origin::Human;

fn data_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("zebra-review-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once, before any test of this binary starts a window.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
        dir
    })
    .clone()
}

fn harness() -> Harness<'static, ZebraApp> {
    data_dir();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 1000.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn shell() -> Harness<'static, Shell> {
    data_dir();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| Shell::new(ZebraApp::new(&cc.egui_ctx, None, None, AppOptions::default())));
    h.run_steps(3);
    h
}

fn frame(w: &mut ZebraApp, x: f64, on_path: bool) -> String {
    let out = w.editor.apply(Command::AddFrame { style: Default::default(), x, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: on_path, find_space: false }, H).unwrap();
    out.created[0].to_string()
}

/// C1: removing a step that isn't the last one, with the last row of the
/// sidebar on screen, read a step that was no longer there.
#[test]
fn c1_removing_a_middle_step_with_the_last_row_on_screen() {
    for via_menu in [false, true] {
        let mut h = harness();
        for i in 0..4 {
            frame(h.state_mut(), i as f64 * 1000.0, true);
        }
        h.state_mut().show_sidebar = true;
        h.state_mut().view_cmd(ViewCommand::ClearSelection);
        h.state_mut().selected_step = Some(1);
        h.run_steps(3);
        assert_eq!(h.state().editor.doc.path.len(), 4);
        if via_menu {
            h.state_mut().editor.apply(Command::RemoveStep { index: 1 }, H).unwrap();
            // What the row's own menu does mid-loop is covered by the
            // guard at the top of the loop; here the path shrinks between
            // frames with the stale selection still set.
        } else {
            h.get_by_label("Minus").click();
        }
        h.run_steps(4);
        assert_eq!(h.state().editor.doc.path.len(), 3);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(4);
        assert_eq!(h.state().editor.doc.path.len(), 4, "one undo brings the step back");
    }
}

/// C2: the path gets shorter while presenting on its last step.
#[test]
fn c2_undoing_a_step_while_presenting_on_it() {
    let mut h = harness();
    for i in 0..3 {
        frame(h.state_mut(), i as f64 * 1000.0, true);
    }
    let cam = h.state().editor.view.camera;
    h.state_mut().apply(Command::AddStep { target: StepTarget::View { view: zebra_core::model::ViewRect { cx: cam.cx, cy: cam.cy, rotation: 0.0, width: 800.0, height: 600.0 } }, index: None, builds: vec![], notes: String::new() });
    h.state_mut().view_cmd(ViewCommand::Present { step: Some(3) });
    h.run_steps(5);
    assert_eq!(h.state().editor.view.present.step, Some(3));
    h.state_mut().queued.push(Action::Undo);
    h.run_steps(2);
    // Moving the mouse draws the step counter; the keys step.
    h.event(Event::PointerMoved(Pos2::new(400.0, 400.0)));
    h.run_steps(2);
    for k in [Key::ArrowLeft, Key::ArrowRight, Key::Backspace, Key::End] {
        h.key_press(k);
        h.run_steps(2);
    }
    let app = h.state();
    assert_eq!(app.editor.doc.path.len(), 3);
    assert!(app.editor.view.present.step.is_none_or(|s| s < 3));
    assert_eq!(app.editor.view.mode, Mode::Present);
}

/// R15: ⌘W and ⌘O while presenting are the menu's, not "white screen" and
/// "overview".
#[test]
fn r15_command_keys_are_not_presenting_keys() {
    let mut h = harness();
    frame(h.state_mut(), 0.0, true);
    frame(h.state_mut(), 1000.0, true);
    h.state_mut().view_cmd(ViewCommand::Present { step: Some(1) });
    h.run_steps(5);
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.blank, None);
    assert_eq!(h.state().editor.view.present.step, Some(1));
    h.key_press(Key::W);
    h.run_steps(2);
    assert!(h.state().editor.view.present.blank.is_some());
}

/// D2: the level the person chose is not put back up when a new document
/// opens.
#[test]
fn d2_a_new_document_keeps_the_permission_the_person_chose() {
    let mut h = shell();
    let dir = data_dir();
    {
        let w = &mut h.state_mut().windows[0];
        w.api_socket = Some(dir.join("d2.sock"));
        w.editor.view.agent_permission = Permission::Full;
        w.set_ai_control(true);
        assert!(w.api.is_some(), "{:?}", w.api_error);
    }
    h.run_steps(4);
    assert_eq!(h.state().windows[0].editor.view.agent_permission, Permission::Full);
    // The person turns it down, then File > New.
    h.state_mut().windows[0].editor.view(ViewCommand::SetAgentPermission { permission: Permission::Observe }, H).unwrap();
    h.run_steps(2);
    frame(&mut h.state_mut().windows[0], 0.0, false);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(6);
    let s = h.state();
    assert_eq!(s.windows.len(), 2);
    for w in &s.windows {
        assert_eq!(w.editor.view.agent_permission, Permission::Observe, "window {}", w.doc_id);
    }
    // And when every window has been closed and a new one opens.
    h.state_mut().windows[0].set_ai_control(false);
    h.run_steps(3);
}

/// D4: Path from Frames asks before it drops steps that have notes.
#[test]
fn d4_path_from_frames_asks_before_losing_notes() {
    let mut h = harness();
    frame(h.state_mut(), 0.0, true);
    let cam = h.state().editor.view.camera;
    h.state_mut().apply(Command::AddStep { target: StepTarget::View { view: zebra_core::model::ViewRect { cx: cam.cx, cy: cam.cy, rotation: 0.0, width: 800.0, height: 600.0 } }, index: None, builds: vec![], notes: "remember this".into() });
    h.state_mut().queued.push(Action::AutoPath);
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.path.len(), 2, "nothing replaced before the answer");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.path.len(), 2, "Esc cancels");
    h.state_mut().queued.push(Action::AutoPath);
    h.run_steps(3);
    h.get_by_label("Replace Path").click();
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.path.len(), 1);
    h.state_mut().queued.push(Action::Undo);
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.path[1].notes, "remember this");
}

/// R1 and R3: a window that isn't being drawn (minimised, or simply idle)
/// still autosaves and still keeps time. `logic` is all that runs then.
#[test]
fn r1_r3_autosave_and_the_clock_run_without_a_frame() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    frame(h.state_mut(), 0.0, false);
    let app = h.state_mut();
    let file = zebra_app::files::untitled_dir().join(format!("{}.zebra.autosave", app.autosave_name));
    let _ = std::fs::remove_file(&file);
    app.last_autosave = app.now() - 1000.0;
    app.editor.view.now = 0.0;
    app.logic(&ctx);
    assert!(file.exists(), "autosave written from logic(), with no frame drawn");
    assert!(app.editor.view.now > 0.0 && (app.editor.view.now - app.now()).abs() < 1.0, "the clock moved");

    // Agent steps more than two seconds apart are separate undo entries,
    // however long ago the last frame was.
    let agent = Origin::Agent { name: "a".into() };
    let n = app.editor.history().len();
    let id: zebra_core::ElementId = app.editor.doc.z_order[0].clone();
    app.editor.apply(Command::Move { ids: vec![id.clone()], dx: 1.0, dy: 0.0, with_contents: true }, agent.clone()).unwrap();
    app.t0 -= std::time::Duration::from_secs(5);
    app.logic(&ctx);
    app.editor.apply(Command::Move { ids: vec![id], dx: 1.0, dy: 0.0, with_contents: true }, agent).unwrap();
    assert_eq!(app.editor.history().len(), n + 2);
}

/// R2: an action queued for a window that is out of sight (Quit from the
/// menu with the window minimised) gets a frame to run in.
#[test]
fn r2_a_minimised_window_still_runs_queued_actions() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().minimized = Some(true);
    h.step();
    assert!(h.state().out_of_sight(&ctx), "the test could not minimise the window");
    h.state_mut().queued.push(Action::ToggleSidebar);
    let before = h.state().show_sidebar;
    h.state_mut().logic(&ctx);
    assert!(h.state().wake_hidden, "a frame is asked for");
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().minimized = Some(true);
    h.step();
    assert_ne!(h.state().show_sidebar, before);
    assert!(!h.state().wake_hidden);
}

fn click_at(h: &mut Harness<'static, ZebraApp>, p: Pos2, count: usize) {
    h.event(Event::PointerMoved(p));
    h.step();
    for _ in 0..count {
        h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.step();
    }
}

/// D6: ⌘Z in a text box brought back what was typed in the box edited
/// before it, and finishing the edit saved that as this box's text.
#[test]
fn d6_undo_in_a_text_box_knows_nothing_of_the_last_one() {
    let mut h = harness();
    let add = |h: &mut Harness<'static, ZebraApp>, text: &str, y: f64, id: &str| {
        h.state_mut().apply(Command::AddText { text: text.into(), x: 0.0, y, style: Default::default(), scale: None, color: None, id: Some(id.into()), markdown: false, on: None, place: None });
    };
    add(&mut h, "Add a title", -200.0, "a");
    add(&mut h, "Topic 1", 200.0, "b");
    h.state_mut().queued.push(Action::FitAll);
    h.run_steps(40);
    let at = |h: &Harness<'static, ZebraApp>, id: &str| {
        let app = h.state();
        app.canvas.w2s(&app.editor, app.editor.doc.get(id).unwrap().transform.center())
    };
    // Edit the title: type, undo the typing, finish.
    let p = at(&h, "a");
    click_at(&mut h, p, 2);
    h.run_steps(3);
    assert_eq!(h.state().canvas.text.as_ref().and_then(|t| t.id.clone()).map(|i| i.to_string()), Some("a".into()));
    h.event(Event::Text("QQQ".into()));
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    h.key_press(Key::Escape);
    h.run_steps(40);
    // Edit the other text: type, then undo twice.
    let p = at(&h, "b");
    click_at(&mut h, p, 2);
    h.run_steps(3);
    assert_eq!(h.state().canvas.text.as_ref().and_then(|t| t.id.clone()).map(|i| i.to_string()), Some("b".into()));
    h.event(Event::Text("X".into()));
    h.run_steps(3);
    for _ in 0..3 {
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(3);
        if let Some(t) = &h.state().canvas.text {
            assert!(!t.buf.contains("title"), "the other box's text came back: {:?}", t.buf);
        }
    }
    h.key_press(Key::Escape);
    h.run_steps(3);
    let app = h.state();
    let text = |id: &str| match &app.editor.doc.get(id).unwrap().kind {
        zebra_core::ElementKind::Text(t) => t.text.clone(),
        _ => unreachable!(),
    };
    assert!(text("b").starts_with("Topic 1"), "{:?}", text("b"));
    assert!(text("a").starts_with("Add a title"), "{:?}", text("a"));
}

/// R23: the end of the person's drag doesn't close a batch an agent has
/// open.
#[test]
fn r23_a_mouse_up_leaves_an_agents_transaction_open() {
    let mut h = harness();
    let id = frame(h.state_mut(), 0.0, false);
    let agent = Origin::Agent { name: "a".into() };
    h.state_mut().editor.begin("agent batch", agent.clone()).unwrap();
    let n = h.state().editor.history().len();
    // The person clicks and drags on the frame; the editor refuses the
    // move (an agent's batch is open), and mouse-up must not commit it.
    let p = {
        let app = h.state();
        app.canvas.w2s(&app.editor, app.editor.doc.get(id.as_str()).unwrap().transform.center())
    };
    h.event(Event::PointerMoved(p));
    h.step();
    h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=5 {
        h.event(Event::PointerMoved(p + egui::vec2(10.0 * i as f32, 0.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: p + egui::vec2(50.0, 0.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.editor.transaction_owner(), Some(&agent), "still the agent's");
    assert_eq!(app.editor.history().len(), n);
}

/// W7: clicking into a number field and out again wrote the rounded text
/// back (123.4 became 123) with no undo step.
#[test]
fn w7_clicking_through_a_number_field_changes_nothing() {
    use zebra_app::widgets::{Num, number_field};
    let mut h = Harness::builder().with_size(egui::vec2(300.0, 120.0)).with_accessibility_check(false).build_ui_state(
        |ui, s: &mut (f64, f64, u32)| {
            zebra_app::style::install_fonts(ui.ctx());
            zebra_app::style::apply_visuals(ui.ctx());
            let a = number_field(ui, &mut s.0, Num { decimals: 0, ..Default::default() }.width(72.0));
            let b = number_field(ui, &mut s.1, Num { decimals: 0, ..Default::default() }.width(72.0));
            if a.changed() || b.changed() {
                s.2 += 1;
            }
        },
        (123.4, 7.0, 0),
    );
    h.run_steps(2);
    let fields: Vec<egui::Rect> = h.query_all_by_role(egui::accesskit::Role::TextInput).map(|n| n.rect()).collect();
    assert_eq!(fields.len(), 2);
    for r in [fields[0], fields[1], fields[0]] {
        let p = r.center();
        h.event(Event::PointerMoved(p));
        h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    assert_eq!(*h.state(), (123.4, 7.0, 0));
    // Typing a new value still commits when focus leaves.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.step();
    h.event(Event::Text("50".into()));
    h.run_steps(2);
    let p = fields[1].center();
    h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!((h.state().0, h.state().1), (50.0, 7.0));
}
