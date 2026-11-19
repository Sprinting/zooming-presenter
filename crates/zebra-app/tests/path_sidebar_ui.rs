//! Spec 08: the path sidebar's keyboard focus, inserting a step between
//! steps, and the sidebar bugs found on 19 Nov.
use egui::{Event, Key, Modifiers, Pos2};
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Origin, ViewCommand};

const H: Origin = Origin::Human;

fn data_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("zebra-path-sidebar-ui-{}", std::process::id()));
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

fn frame(w: &mut ZebraApp, x: f64, on_path: bool) -> String {
    let out = w.editor.apply(Command::AddFrame { style: Default::default(), x, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: on_path, find_space: false }, H).unwrap();
    out.created[0].to_string()
}

/// A long path: one frame per step, added in one go (fast to set up).
fn long_path(h: &mut Harness<'static, ZebraApp>, n: usize) {
    let cmds: Vec<Command> = (0..n)
        .map(|i| Command::AddFrame { style: Default::default(), x: i as f64 * 1000.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: true, find_space: false })
        .collect();
    h.state_mut().editor.apply_batch(cmds, None, H).unwrap();
    h.run_steps(3);
}

/// 19 Nov: step numbers of 1,000 and up ran into the thumbnail, the
/// number column being a fixed 22 points.
#[test]
fn the_number_column_fits_the_widest_step_number() {
    let mut h = harness();
    long_path(&mut h, 3);
    let short = h.state().step_number_w;
    assert_eq!(short, 22.0, "short paths keep the usual column");
    long_path(&mut h, 1197);
    assert_eq!(h.state().editor.doc.path.len(), 1200);
    let wide = h.state().step_number_w;
    let text = h.ctx.fonts_mut(|f| f.layout_no_wrap("8888".into(), zebra_app::style::semibold(11.0), egui::Color32::WHITE).size().x);
    assert!(wide >= text + 8.0, "column {wide} holds four digits ({text}) with room either side");
}

/// 19 Nov: an agent's goto_step flew the camera but left the sidebar where
/// it was; only a change of selection scrolled it.
#[test]
fn goto_step_from_an_agent_scrolls_the_sidebar() {
    let mut h = harness();
    long_path(&mut h, 300);
    let on = h.state().thumbs.showing();
    assert!(!on.contains(&240), "starts at the top: {on:?}");
    // Flying the person's camera is for agents with full permission.
    h.state_mut().editor.view.agent_permission = zebra_core::view::Permission::Full;
    h.state_mut().editor.view(ViewCommand::GotoStep { step: 240 }, Origin::agent("a")).unwrap();
    h.run_steps(4);
    let on = h.state().thumbs.showing();
    assert!(on.contains(&240), "scrolled to the step: {on:?}");
    // The person's own goto_step scrolls too.
    h.state_mut().editor.view(ViewCommand::GotoStep { step: 10 }, H).unwrap();
    h.run_steps(4);
    assert!(h.state().thumbs.showing().contains(&10));
}

/// 19 Nov: the step nearest the camera is found without measuring every
/// step on each camera change, and still agrees with doing so.
#[test]
fn the_nearest_step_agrees_with_measuring_every_step() {
    let mut h = harness();
    long_path(&mut h, 400);
    h.state_mut().view_cmd(ViewCommand::ClearSelection);
    h.state_mut().selected_step = None;
    for i in [0usize, 7, 199, 398] {
        let mut cam = h.state().editor.camera_for_step(i);
        cam.cx += 120.0;
        h.state_mut().editor.view.camera = cam;
        h.run_steps(1);
        assert_eq!(h.state().current_step(), Some(i));
    }
}

#[allow(dead_code)]
fn click_at(h: &mut Harness<'static, ZebraApp>, p: Pos2, modifiers: Modifiers) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(2);
}

#[allow(dead_code)]
fn key(h: &mut Harness<'static, ZebraApp>, k: Key) {
    h.key_press(k);
    h.run_steps(2);
}
