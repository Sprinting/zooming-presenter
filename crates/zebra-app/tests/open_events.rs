//! Files Finder, the Dock or Open With ask Zebra to open (Kartik, 18 Nov:
//! a double-clicked .zebra failed and left an Untitled window with the
//! template chooser). The AppKit half (`application:openURLs:`) only runs
//! on the Mac; this checks what it feeds: the launch takes the file instead
//! of offering the chooser, and the shell opens files that come later.
//! Its own test binary, with one test: the queue is process-wide.
use egui_kittest::Harness;
use zebra_app::shell::Shell;
use zebra_app::{AppOptions, ZebraApp, open_events};
use zebra_core::{Command, Origin};

fn data_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("zebra-open-events-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: set before this binary's only test starts a window.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    dir
}

fn deck(path: &std::path::Path, frames: usize) {
    let mut ed = zebra_core::Editor::new(Default::default());
    for i in 0..frames {
        ed.apply(Command::AddFrame { style: Default::default(), x: i as f64 * 900.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: true, find_space: false }, Origin::Human).unwrap();
    }
    zebra_core::io::write_file(&ed.doc, path).unwrap();
}

#[test]
fn files_from_finder_open_at_launch_and_later() {
    let dir = data_dir();
    let (a, b, c) = (dir.join("a.zebra"), dir.join("b.zebra"), dir.join("c.zebra"));
    deck(&a, 1);
    deck(&b, 2);
    deck(&c, 3);

    // A double-click on a.zebra launches Zebra; b.zebra was selected too.
    // Both arrive while launch finishes, before any window exists.
    open_events::push([a.clone(), b.clone()]);
    let opts = AppOptions { open: open_events::launch_file(None), ..Default::default() };
    assert_eq!(opts.open.as_deref(), Some(a.as_path()));
    assert!(!opts.offers_chooser(), "launched with a file: no template chooser");
    assert!(AppOptions::default().offers_chooser(), "a plain launch still offers it");

    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(move |cc| Shell::new(ZebraApp::new(&cc.egui_ctx, None, None, opts)));
    h.run_steps(6);
    let s = h.state();
    assert!(s.windows[0].welcome.is_none());
    assert_eq!(s.windows.len(), 2, "the second file gets a window of its own");
    let names: Vec<_> = s.windows.iter().map(|w| w.editor.file_path.clone()).collect();
    assert_eq!(names, vec![Some(a.clone()), Some(b.clone())]);
    assert_eq!(s.windows[1].editor.doc.elements.len(), 2);

    // Zebra running: a file that is already open comes to the front, no new window.
    assert_eq!(h.state().front, 1, "b.zebra's window is in front");
    open_events::push([a.clone()]);
    h.run_steps(4);
    assert_eq!(h.state().windows.len(), 2);
    assert_eq!(h.state().front, 0, "a.zebra's window comes forward");

    // A new file opens in a new window.
    open_events::push([c.clone()]);
    h.run_steps(4);
    assert_eq!(h.state().windows.len(), 3);
    assert_eq!(h.state().windows[2].editor.file_path.as_deref(), Some(c.as_path()));
    assert!(open_events::take().is_empty(), "every file was taken");

    // M10 (Mac pass 21 Nov): a file that can't be opened showed its alert
    // in a new, empty Untitled window, which stayed. As in Keynote: only the
    // alert, in the window in front; no new window.
    let bad = dir.join("garbage.zebra");
    std::fs::write(&bad, b"not a zebra file").unwrap();
    let front = h.state().front;
    open_events::push([bad.clone()]);
    h.run_steps(4);
    let s = h.state();
    assert_eq!(s.windows.len(), 3, "no window for a file that didn't open");
    assert_eq!(s.front, front);
    assert!(s.windows[front].sheet_showing(), "the alert is up in the window in front");
    assert_eq!(s.windows[front].editor.file_path.as_deref(), Some(c.as_path()), "its document is untouched");

    // A file sent after the chooser came up on an untouched window opens
    // in that window and the chooser goes.
    let mut h2 = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| Shell::new(ZebraApp::new(&cc.egui_ctx, None, None, AppOptions::default())));
    h2.run_steps(3);
    h2.state_mut().windows[0].welcome = Some(zebra_app::welcome::Welcome::new());
    open_events::push([b.clone()]);
    h2.run_steps(4);
    let s = h2.state();
    assert_eq!(s.windows.len(), 1);
    assert_eq!(s.windows[0].editor.file_path.as_deref(), Some(b.as_path()));
    assert!(s.windows[0].welcome.is_none(), "the chooser gave way to the file");
    drop(h2);

    // M14 (Mac pass, 21 Nov): Zebra's window covered by a full-screen app.
    // macOS draws no frame for a covered window, so eframe runs only
    // `logic`. A file Finder sends then (`open -g`) opens anyway, in a
    // window of its own, as Keynote opens it whether or not it's in front;
    // before, it waited for the window to be shown and an Untitled showed.
    let mut h3 = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| Shell::new(ZebraApp::new(&cc.egui_ctx, None, None, AppOptions::default())));
    h3.run_steps(3);
    h3.state_mut().windows[0].editor.apply(Command::AddFrame { style: Default::default(), x: 0.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: true, find_space: false }, Origin::Human).unwrap();
    h3.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().occluded = Some(true);
    h3.run_steps(1);
    open_events::push([c.clone()]);
    let ctx = h3.ctx.clone();
    let mut frame = eframe::Frame::_new_kittest();
    for _ in 0..4 {
        eframe::App::logic(h3.state_mut(), &ctx, &mut frame);
    }
    let s = h3.state();
    assert_eq!(s.windows.len(), 2, "c.zebra got a window with no frame drawn");
    assert_eq!(s.windows[1].editor.file_path.as_deref(), Some(c.as_path()), "and it is open, not an Untitled");
    assert_eq!(s.windows[1].editor.doc.elements.len(), 3);
    // M6: the covered window's title follows its document with no frame
    // drawn (the Window menu and Dock read it).
    h3.state_mut().windows[0].editor.file_path = Some(a.clone());
    eframe::App::logic(h3.state_mut(), &ctx, &mut frame);
    assert!(h3.state().windows[0].last_title.starts_with('a'), "the covered window's title was sent: {}", h3.state().windows[0].last_title);
}
