//! Hostile-input audit H4: an untitled document that crashed the app while
//! it was being reopened is not reopened by itself at the next launch,
//! which would crash again; a sheet asks instead. Its own test binary,
//! with one test: the data folder and the setting override are
//! process-wide.
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::path::{Path, PathBuf};
use zebra_app::sheets::SheetKind;
use zebra_app::shell::Shell;
use zebra_app::{AppOptions, ZebraApp, files};
use zebra_core::{Command, Origin};

fn deck(path: &Path, frames: usize) {
    let mut ed = zebra_core::Editor::new(Default::default());
    for i in 0..frames {
        ed.apply(Command::AddFrame { style: Default::default(), x: i as f64 * 900.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: true, find_space: false }, Origin::Human).unwrap();
    }
    zebra_core::io::write_file(&ed.doc, path).unwrap();
}

fn launch() -> Harness<'static, Shell> {
    let opts = AppOptions { as_launch: true, ..Default::default() };
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(move |cc| Shell::new(ZebraApp::new(&cc.egui_ctx, None, None, opts)));
    h.run_steps(6);
    h
}

fn sheet(h: &Harness<'static, Shell>) -> Option<SheetKind> {
    h.state().windows.iter().find_map(|w| w.sheet.clone())
}

/// What a launch that died while opening `name` leaves: its autosave
/// (owned by a dead process) with the "opening" marker beside it.
fn crashed(name: &str, frames: usize) -> PathBuf {
    let p = files::untitled_dir().join(format!("{name}.zebra.autosave"));
    deck(&p, frames);
    std::fs::write(files::opening_marker(&p), b"").unwrap();
    p
}

#[test]
fn a_document_that_crashed_while_reopening_is_asked_about_not_reopened() {
    let data = std::env::temp_dir().join(format!("zebra-crash-reopen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    // SAFETY: set before this binary's only test starts a window.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &data) };
    zebra_app::mac::set_restores_windows(Some(true));
    let dir = files::untitled_dir();
    std::fs::create_dir_all(&dir).unwrap();

    // The marker logic on its own: reopening marks the autosave; a run
    // that dies before the marker goes leaves both behind, and the next
    // launch sorts that autosave out of the ones to reopen.
    let fine = dir.join("Untitled-1-2147483000-0.zebra.autosave");
    deck(&fine, 2);
    {
        let mut a = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
        assert!(a.state_mut().reopen_autosave(&fine));
        let (marker, _) = a.state().opening_marker.clone().expect("marked while opening");
        assert!(marker.exists());
        let adopted = files::untitled_dir().join(format!("{}.zebra.autosave", a.state().autosave_name));
        assert_eq!(marker, files::opening_marker(&adopted));
        // The app "dies" here: rename it to a dead owner, marker and all.
        let dead = dir.join("Untitled-2-2147483001-0.zebra.autosave");
        std::fs::rename(&adopted, &dead).unwrap();
        std::fs::rename(&marker, files::opening_marker(&dead)).unwrap();
        let (reopen, asked) = files::split_crashed(files::leftover_untitled());
        assert!(reopen.is_empty(), "{reopen:?}");
        assert_eq!(asked, vec![dead.clone()]);
        // Shown long enough (or closed cleanly): the marker goes.
        a.state_mut().opening_marker = Some((files::opening_marker(&dead), -100.0));
        a.state_mut().clear_opening_marker(false);
        assert!(!files::opening_marker(&dead).exists());
        std::fs::remove_file(&dead).unwrap();
    }

    // A launch: the sound leftover comes back, the crashed one is asked
    // about in a sheet and not opened.
    deck(&dir.join("Untitled-3-2147483002-0.zebra.autosave"), 2);
    let bad = crashed("Untitled-4-2147483003-0", 3);
    let mut h = launch();
    assert_eq!(h.state().windows.len(), 1);
    assert_eq!(h.state().windows[0].editor.doc.elements.len(), 2);
    let Some(SheetKind::CrashedOpening { path, name }) = sheet(&h) else { panic!("{:?}", sheet(&h)) };
    assert_eq!(path, bad);
    assert_eq!(name, "Untitled 2", "numbered after the open window, \"Untitled\"");
    h.run_steps(2);
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        h.render().unwrap().save(Path::new(&dir).join("crash-reopen-sheet.png")).unwrap();
    }
    // Keep for Later: nothing changes; the next launch asks again.
    h.get_by_label(&zebra_app::i18n::tr("sheet-keep-for-later")).click();
    h.run_steps(3);
    assert!(sheet(&h).is_none());
    assert!(bad.exists() && files::opening_marker(&bad).exists());
    files::discard_autosave(&h.state().windows[0]);
    drop(h);

    // Open Anyway: it opens, in a window of its own, marked again.
    let mut h = launch();
    assert!(matches!(sheet(&h), Some(SheetKind::CrashedOpening { .. })));
    // Nothing else was reopened, so the template chooser is due; it waits
    // behind the sheet instead of covering it.
    assert!(h.state().windows[0].welcome.is_some());
    h.get_by_label(&zebra_app::i18n::tr("sheet-open-anyway")).click();
    h.run_steps(6);
    let opened: Vec<usize> = h.state().windows.iter().map(|w| w.editor.doc.elements.len()).collect();
    assert!(opened.contains(&3), "{opened:?}");
    assert!(!files::opening_marker(&bad).exists(), "the old marker went with the old name");
    assert!(h.state().windows.iter().any(|w| w.opening_marker.as_ref().is_some_and(|(m, _)| m.exists())));
    for w in &mut h.state_mut().windows {
        w.clear_opening_marker(true);
        files::discard_autosave(w);
    }
    drop(h);

    // Move to Bin: it goes.
    let bad = crashed("Untitled-5-2147483004-0", 3);
    let mut h = launch();
    h.get_by_label(&zebra_app::i18n::tr("sheet-move-to-bin")).click();
    h.run_steps(3);
    assert!(!bad.exists() && !files::opening_marker(&bad).exists());
    h.get_by_label(&zebra_app::i18n::tr("welcome-title"));
    assert!(h.state().windows.iter().all(|w| w.editor.doc.elements.len() != 3));
    drop(h);
    zebra_app::mac::set_restores_windows(None);
    let _ = std::fs::remove_dir_all(&data);
}
