//! Open Recent lists every document the window opens or saves under a new
//! name, whoever did it (0.1: files an agent opened or saved were missing),
//! as Keynote lists every document it opens.
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Origin};

#[test]
fn files_an_agent_opens_or_saves_go_in_open_recent() {
    let dir = std::env::temp_dir().join(format!("zebra-recent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: set before this binary's only test starts a window.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    assert!(h.state().recent.is_empty());

    // An agent opens a.zebra (as the API's `open` does: behind the window).
    let a = dir.join("a.zebra");
    let mut ed = zebra_core::Editor::new(Default::default());
    ed.apply(Command::AddFrame { style: Default::default(), x: 0.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: true, find_space: false }, Origin::Human).unwrap();
    zebra_core::io::write_file(&ed.doc, &a).unwrap();
    let app = h.state_mut();
    let doc = zebra_core::io::load(&a).unwrap();
    app.editor.load(doc, Some(a.clone()));
    h.run_steps(2);
    assert_eq!(h.state().recent, vec![a.clone()]);

    // It saves the document as b.zebra.
    let b = dir.join("b.zebra");
    let app = h.state_mut();
    app.editor.file_path = Some(b.clone());
    h.run_steps(2);
    assert_eq!(h.state().recent, vec![b.clone(), a.clone()], "newest first");

    // A new untitled document adds nothing.
    let app = h.state_mut();
    app.editor.load(zebra_core::Document::default(), None);
    h.run_steps(2);
    assert_eq!(h.state().recent, vec![b, a]);
    let _ = std::fs::remove_dir_all(&dir);
}
