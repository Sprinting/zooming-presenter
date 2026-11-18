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
