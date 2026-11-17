//! Untitled documents: one autosave per window, reopened unsaved after a
//! crash, and gone once the person chooses Don't Save. Its own test
//! binary: it points Zebra's data folder at a scratch directory.
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp, files};

fn app() -> Harness<'static, ZebraApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(2);
    h
}

#[test]
fn untitled_work_comes_back_unsaved_and_dont_save_removes_it() {
    let dir = std::env::temp_dir().join(format!("zebra-autosave-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: set before any thread reads the environment.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };

    let doc = zebra_app::sample::build_with(Box::new(zebra_core::editor::EstimateMeasure));
    let n = doc.elements.len();
    // One autosave from an older build, one per-window autosave.
    zebra_core::io::write_file(&doc, &dir.join("Untitled.zebra.autosave")).unwrap();
    std::fs::create_dir_all(files::untitled_dir()).unwrap();
    zebra_core::io::write_file(&doc, &files::untitled_dir().join("Untitled-a.zebra.autosave")).unwrap();

    let left = files::leftover_untitled();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(!dir.join("Untitled.zebra.autosave").exists(), "the old single autosave moves in with the rest");

    // Two windows never share an autosave.
    let (a, b) = (app(), app());
    assert_ne!(a.state().autosave_name, b.state().autosave_name);

    let mut h = app();
    assert!(h.state_mut().reopen_autosave(&files::untitled_dir().join("Untitled-a.zebra.autosave")));
    let s = h.state();
    assert_eq!(s.editor.doc.elements.len(), n);
    assert!(s.editor.dirty(), "reopened work is unsaved");
    assert!(s.editor.file_path.is_none());
    assert_eq!(s.autosave_name, "Untitled-a");

    files::discard_autosave(h.state());
    assert!(!files::untitled_dir().join("Untitled-a.zebra.autosave").exists());
    assert_eq!(files::leftover_untitled().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
