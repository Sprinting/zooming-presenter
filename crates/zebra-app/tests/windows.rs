//! Several document windows, like Keynote: New and Open get a window of
//! their own unless the front one is an untouched new document; shared
//! settings follow from window to window; closing the first window moves
//! the next document into it.
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::shell::Shell;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Origin};

fn harness() -> Harness<'static, Shell> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| Shell::new(ZebraApp::new(&cc.egui_ctx, None, None, AppOptions::default())));
    h.run_steps(3);
    h
}

fn add_frame(w: &mut ZebraApp) {
    w.editor.apply(Command::AddFrame { style: Default::default(), x: 0.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: false, find_space: false }, Origin::Human).unwrap();
}

#[test]
fn new_reuses_an_untouched_window_and_otherwise_opens_one() {
    let mut h = harness();
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(3);
    assert_eq!(h.state().windows.len(), 1, "an untouched new document is replaced");

    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    let s = h.state();
    assert_eq!(s.windows.len(), 2);
    assert_eq!((s.windows[0].doc_id.as_str(), s.windows[1].doc_id.as_str()), ("d1", "d2"));
    assert_eq!(s.front, 1, "the new window is in front");
    assert!(s.windows[1].front && !s.windows[0].front);
    assert_eq!(s.windows[0].editor.doc.elements.len(), 1, "the first document is kept");
    assert!(s.windows[1].editor.doc.elements.is_empty());
}

#[test]
fn shared_settings_follow_from_window_to_window() {
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    assert!(h.state().windows[0].snapping);
    h.state_mut().windows[1].snapping = false;
    h.state_mut().windows[1].default_theme = "night".into();
    h.run_steps(2);
    assert!(!h.state().windows[0].snapping);
    assert_eq!(h.state().windows[0].default_theme, "night");
}

#[test]
fn closing_windows_keeps_the_others() {
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].editor.mark_saved(None);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    add_frame(&mut h.state_mut().windows[1]);
    h.state_mut().windows[1].editor.mark_saved(None);
    h.state_mut().windows[1].queued.push(Action::New);
    h.run_steps(4);
    assert_eq!(h.state().windows.len(), 3);

    // Close the middle one.
    h.state_mut().windows[1].queued.push(Action::Close);
    h.run_steps(3);
    let ids: Vec<String> = h.state().windows.iter().map(|w| w.doc_id.clone()).collect();
    assert_eq!(ids, ["d1", "d3"]);

    // Close the first: the next document moves into the first window.
    h.state_mut().windows[0].queued.push(Action::Close);
    h.run_steps(3);
    let s = h.state();
    assert_eq!(s.windows.len(), 1);
    assert_eq!(s.windows[0].doc_id, "d3");
    assert_eq!(s.front, 0);
    assert!(s.windows[0].front);
}

#[test]
fn a_window_with_unsaved_changes_asks_before_closing() {
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    add_frame(&mut h.state_mut().windows[1]);
    h.state_mut().windows[1].queued.push(Action::Close);
    h.run_steps(3);
    assert_eq!(h.state().windows.len(), 2, "still open");
    assert!(h.state().windows[1].sheet_showing(), "Save Changes is asked");
}

#[test]
fn closing_the_last_window_keeps_zebra_running() {
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].editor.mark_saved(None);
    h.state_mut().windows[0].queued.push(Action::Close);
    h.run_steps(3);
    let s = h.state();
    assert!(s.empty, "like Keynote: no window, still running");
    assert_eq!(s.windows.len(), 1);
    assert!(s.windows[0].is_pristine(), "a new document waits in the hidden window");
    assert!(!s.windows[0].force_quit);
}
