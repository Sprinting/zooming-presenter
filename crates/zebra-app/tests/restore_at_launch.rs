//! Kartik, 19 Nov: copy Keynote. A launch brings back last run's unsaved
//! Untitled documents, also when it was for a file (command line, Finder,
//! the Dock), and that file opens beside them, in front. With the Mac's
//! "Close windows when quitting an application" on, only the file opens.
//! Its own test binary, with one test: the data folder, the open queue and
//! the setting override are process-wide.
use egui_kittest::Harness;
use std::path::{Path, PathBuf};
use zebra_app::shell::Shell;
use zebra_app::{AppOptions, ZebraApp, files, open_events};
use zebra_core::{Command, Origin};

fn deck(path: &Path, frames: usize) {
    let mut ed = zebra_core::Editor::new(Default::default());
    for i in 0..frames {
        ed.apply(Command::AddFrame { style: Default::default(), x: i as f64 * 900.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: true, find_space: false }, Origin::Human).unwrap();
    }
    zebra_core::io::write_file(&ed.doc, path).unwrap();
}

/// What a killed run leaves: two untitled autosaves nobody owns (2 and 3
/// frames, the 3-frame one newest), and one of an emptied document.
fn leftovers() {
    let dir = files::untitled_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    zebra_core::io::write_file(&zebra_core::Document::default(), &dir.join("Untitled-1-0-0.zebra.autosave")).unwrap();
    deck(&dir.join("Untitled-2-0-0.zebra.autosave"), 2);
    std::thread::sleep(std::time::Duration::from_millis(20));
    deck(&dir.join("Untitled-3-0-0.zebra.autosave"), 3);
    assert_eq!(files::leftover_untitled().len(), 3);
}

fn launch(open: Option<PathBuf>) -> Harness<'static, Shell> {
    let opts = AppOptions { open: open_events::launch_file(open), as_launch: true, ..Default::default() };
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(move |cc| Shell::new(ZebraApp::new(&cc.egui_ctx, None, None, opts)));
    h.run_steps(6);
    h
}

/// (file, element count, edited) for each window, in order.
fn windows(h: &Harness<'static, Shell>) -> Vec<(Option<PathBuf>, usize, bool)> {
    h.state().windows.iter().map(|w| (w.editor.file_path.clone(), w.editor.doc.elements.len(), w.editor.dirty())).collect()
}

#[test]
fn a_launch_brings_back_untitled_work_beside_the_file() {
    let data = std::env::temp_dir().join(format!("zebra-restore-launch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    // SAFETY: set before this binary's only test starts a window.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &data) };
    let (a, b) = (data.join("a.zebra"), data.join("b.zebra"));
    deck(&a, 1);
    deck(&b, 4);

    // Launched with a file from the command line, the Mac keeping windows
    // at quit: both untitled documents come back unsaved, the empty one
    // doesn't, and the file opens in front. No template chooser.
    zebra_app::mac::set_restores_windows(Some(true));
    leftovers();
    let h = launch(Some(a.clone()));
    assert_eq!(windows(&h), vec![(None, 2, true), (None, 3, true), (Some(a.clone()), 1, false)]);
    let s = h.state();
    assert_eq!(s.front, 2, "the file's window is in front");
    assert!(s.windows.iter().all(|w| w.welcome.is_none()), "no template chooser");
    assert!(files::leftover_untitled().is_empty(), "this run owns them now");
    assert!(!files::untitled_dir().join("Untitled-1-0-0.zebra.autosave").exists(), "the empty one is gone");
    drop(h);

    // The same from Finder, with a second file selected: each file gets a
    // window after the untitled ones, the last in front.
    leftovers();
    open_events::push([a.clone(), b.clone()]);
    let h = launch(None);
    assert_eq!(windows(&h), vec![(None, 2, true), (None, 3, true), (Some(a.clone()), 1, false), (Some(b.clone()), 4, false)]);
    assert_eq!(h.state().front, 3);
    assert!(h.state().windows.iter().all(|w| w.welcome.is_none()));
    assert!(open_events::take().is_empty());
    drop(h);

    // "Close windows when quitting an application" on: only the file
    // opens, in the one window; the untitled work stays on disk.
    zebra_app::mac::set_restores_windows(Some(false));
    leftovers();
    let h = launch(Some(a.clone()));
    assert_eq!(windows(&h), vec![(Some(a.clone()), 1, false)]);
    assert!(h.state().windows[0].welcome.is_none());
    assert_eq!(files::leftover_untitled().len(), 3, "not reopened, not deleted");
    drop(h);
    // From Finder too.
    open_events::push([a.clone()]);
    let h = launch(None);
    assert_eq!(windows(&h), vec![(Some(a.clone()), 1, false)]);
    drop(h);

    // A plain launch with that setting on: the template chooser, as
    // Keynote does, and nothing comes back.
    let h = launch(None);
    assert_eq!(windows(&h), vec![(None, 0, false)]);
    assert!(h.state().windows[0].welcome.is_some());
    drop(h);

    // A plain launch keeping windows is as before: the untitled documents
    // come back, no chooser, no blank window.
    zebra_app::mac::set_restores_windows(Some(true));
    leftovers();
    let h = launch(None);
    assert_eq!(windows(&h), vec![(None, 2, true), (None, 3, true)]);
    assert!(h.state().windows.iter().all(|w| w.welcome.is_none()));
    drop(h);

    // And with nothing left from the last run, the template chooser.
    let _ = std::fs::remove_dir_all(files::untitled_dir());
    let h = launch(None);
    assert_eq!(windows(&h), vec![(None, 0, false)]);
    assert!(h.state().windows[0].welcome.is_some());
    drop(h);

    zebra_app::mac::set_restores_windows(None);
    let _ = std::fs::remove_dir_all(&data);
}
