//! Settings › Keyboard records a whole key combination, not the first
//! modifier pressed. Its own test binary: it changes the shared keymap.
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp, keymap};

fn key(k: Key, modifiers: Modifiers) -> Event {
    Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn recording_waits_for_the_key_after_the_modifiers() {
    // Keep keymap.json out of the real data folder.
    let dir = std::env::temp_dir().join(format!("zebra-keys-ui-{}", std::process::id()));
    // SAFETY: set before any thread reads the environment.
    unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };
    keymap::install(keymap::Overrides::default());

    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty,keysrec".into()), ..Default::default() }));
    h.run_steps(3);
    assert!(h.state().key_capture.is_some(), "the demo starts recording Lock's keys");

    let cmd_shift = Modifiers { shift: true, ..Modifiers::COMMAND };
    // ⌘ goes down, then ⇧: egui reports both as keys of their own.
    h.event(key(Key::SuperLeft, Modifiers::COMMAND));
    h.step();
    h.event(key(Key::ShiftLeft, cmd_shift));
    h.step();
    assert!(h.state().key_capture.is_some(), "a modifier alone must not end the recording");

    h.event(key(Key::K, cmd_shift));
    h.run_steps(2);
    assert!(h.state().key_capture.is_none());
    let (m, k) = keymap::binding_for(&Action::ToggleLock).expect("Lock has keys");
    assert_eq!(k, Key::K);
    assert!(m.shift && m.command && !m.alt && !m.ctrl, "{m:?}");

    keymap::reset_all();
    let _ = std::fs::remove_dir_all(&dir);
}
