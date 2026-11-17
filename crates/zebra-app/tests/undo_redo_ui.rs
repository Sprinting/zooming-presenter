//! Undo and redo from the keyboard after an agent's edits must never take
//! the app down. Repro for the Mac quit on 17 Nov: an agent added text, a
//! shape and grouped them, disconnected, then the person pressed ⌘Z, ⇧⌘Z.
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Origin};

fn harness(demo: &str) -> Harness<'static, ZebraApp> {
    // The forced autosave below must not touch the real Untitled autosave.
    let dir = std::env::temp_dir().join(format!("zebra-undo-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: every test here sets the same value before any app starts.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    let demo = Some(demo.to_string());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(move |cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo, ..Default::default() }));
    h.run_steps(3);
    h
}

fn cmd(v: serde_json::Value) -> Command {
    serde_json::from_value(v).unwrap()
}

/// The agent's edits from the Mac session, as one agent named "claude".
fn agent_edits(h: &mut Harness<'static, ZebraApp>) -> Vec<String> {
    let agent = Origin::agent("claude");
    let ed = &mut h.state_mut().editor;
    let t = ed.apply(cmd(serde_json::json!({"op":"add_text","text":"Mac check","x":0,"y":0})), agent.clone()).unwrap().created;
    let s = ed.apply(cmd(serde_json::json!({"op":"add_shape","shape":"rectangle","x":300,"y":0})), agent.clone()).unwrap().created;
    let ids: Vec<String> = t.iter().chain(&s).map(|i| i.to_string()).collect();
    ed.apply(cmd(serde_json::json!({"op":"group","ids":ids})), agent).unwrap();
    h.run_steps(3);
    ids
}

fn key(h: &mut Harness<'static, ZebraApp>, k: Key, m: Modifiers) {
    h.event(Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.step();
    h.event(Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.step();
}

fn undo(h: &mut Harness<'static, ZebraApp>) {
    key(h, Key::Z, Modifiers::COMMAND);
}

fn redo(h: &mut Harness<'static, ZebraApp>) {
    key(h, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
}

#[test]
fn undo_then_redo_after_agent_group() {
    for demo in ["empty", "sample"] {
        let mut h = harness(demo);
        let ids = agent_edits(&mut h);
        let n = h.state().editor.doc.elements.len();
        undo(&mut h);
        h.run_steps(10);
        redo(&mut h);
        // The quit came about 6 s after the keys, right when the 30 s
        // autosave was due: run past that and force an autosave too.
        h.run_steps(400);
        h.state_mut().last_autosave = -1e9;
        h.run_steps(5);
        zebra_core::io::save_bytes(&h.state().editor.doc).expect("saves after undo/redo");
        assert_eq!(h.state().editor.doc.elements.len(), n, "{demo}: redo restores the group");
        assert!(ids.iter().all(|i| h.state().editor.doc.elements.keys().any(|k| k.to_string() == *i)));
    }
}

#[test]
fn undo_redo_with_the_group_selected_and_all_the_way_back() {
    let mut h = harness("sample");
    let ids = agent_edits(&mut h);
    let sel = h.state().editor.doc.elements.keys().filter(|k| ids.contains(&k.to_string())).cloned().collect();
    h.state_mut().select(sel);
    h.run_steps(3);
    for _ in 0..5 {
        undo(&mut h);
        h.run_steps(5);
    }
    for _ in 0..5 {
        redo(&mut h);
        h.run_steps(5);
    }
    // Undo/redo interleaved quickly, as from a held key.
    for i in 0..20 {
        if i % 2 == 0 { undo(&mut h) } else { redo(&mut h) }
    }
    h.run_steps(400);
    h.state_mut().last_autosave = -1e9;
    h.run_steps(5);
    zebra_core::io::save_bytes(&h.state().editor.doc).expect("saves after undo/redo");
}
