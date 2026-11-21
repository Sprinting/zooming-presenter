//! An Untitled document an agent saves: its Untitled autosave goes, as it
//! does when the person saves, so a relaunch doesn't bring back a second
//! copy (found in the 21 Nov cloud pass). Its own test binary: it points
//! Zebra's data folder at a scratch directory.
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp, files};
use zebra_core::editor::Origin;

#[test]
fn an_agents_save_tidies_the_untitled_autosave() {
    let dir = std::env::temp_dir().join(format!("zebra-autosave-agent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: set before any thread reads the environment.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };

    let doc = zebra_app::sample::build_with(Box::new(zebra_core::editor::EstimateMeasure));
    std::fs::create_dir_all(files::untitled_dir()).unwrap();
    let left = files::untitled_dir().join("Untitled-a.zebra.autosave");
    zebra_core::io::write_file(&doc, &left).unwrap();

    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(2);
    assert!(h.state_mut().reopen_autosave(&left));
    let mine = files::untitled_dir().join(format!("{}.zebra.autosave", h.state().autosave_name));
    assert!(mine.exists());

    // An agent saves it (Save As): the window's Untitled autosave goes.
    let file = dir.join("Saved by agent.zebra");
    let ed = &mut h.state_mut().editor;
    ed.start_save(file.clone(), Origin::agent("bot"), Box::new(|| {})).unwrap();
    assert!(ed.wait_save());
    let _ = ed.take_saved(|s| s.by == Origin::agent("bot"));
    h.run_steps(2);
    assert_eq!(h.state().editor.file_path.as_deref(), Some(file.as_path()));
    assert!(!mine.exists(), "the Untitled autosave is gone once an agent saved the document");
    assert!(files::leftover_untitled().is_empty());
    assert!(file.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
