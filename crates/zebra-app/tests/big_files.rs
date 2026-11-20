//! Big documents open and save off the main thread: the window keeps
//! drawing, and what the person does meanwhile waits for the file.
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Origin};

fn data_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("zebra-big-files-{}", std::process::id()));
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
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn shape(x: f64) -> Command {
    serde_json::from_value(serde_json::json!({"op":"add_shape","shape":"rectangle","x":x,"y":0})).unwrap()
}

#[test]
fn a_big_file_opens_on_a_worker_with_progress() {
    let path = data_dir().join("big.zebra");
    let mut ed = zebra_core::Editor::default();
    ed.apply_batch((0..8000).map(|i| shape(i as f64 * 120.0)).collect(), None, Origin::Human).unwrap();
    zebra_core::io::save(&mut ed.doc, &path).unwrap();
    assert!(zebra_app::opening::is_big(&path), "big enough to open aside");

    let mut h = harness();
    h.state_mut().queued.push(Action::OpenPath(path.clone()));
    h.run_steps(1);
    assert!(h.state().opening.is_some(), "opening on a worker");
    assert_eq!(h.state().shown_file(), Some(path.as_path()));
    let t0 = std::time::Instant::now();
    while h.state().opening.is_some() && t0.elapsed().as_secs() < 120 {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let app = h.state();
    assert!(app.opening.is_none());
    assert_eq!(app.editor.file_path.as_deref(), Some(path.as_path()));
    assert_eq!(app.editor.doc.elements.len(), 8000);
    assert!(!app.editor.dirty());
}

#[test]
fn closing_waits_for_the_save_in_flight() {
    let mut h = harness();
    let app = h.state_mut();
    app.editor.apply(shape(0.0), Origin::Human).unwrap();
    let path = data_dir().join("saving.zebra");
    assert!(app.save_to(&path));
    let ctx = app.ctx.clone();
    // A new document, asked for while the file is being written, waits.
    app.perform(&ctx, Action::New);
    assert_eq!(app.after_save, vec![Action::New]);
    assert_eq!(app.editor.doc.elements.len(), 1, "the document stays until it is saved");
    // So does a second save.
    app.perform(&ctx, Action::Save);
    assert_eq!(app.after_save.len(), 2);
    app.wait_save();
    assert!(!app.editor.dirty());
    assert!(path.exists());
    // The waiting Save was for the document just written: it must not
    // run against the new empty one (that would open a Save dialog).
    assert_eq!(app.queued, vec![Action::New], "only New runs after the save");
    h.run_steps(3);
    let app = h.state();
    assert!(app.after_save.is_empty());
    assert!(app.sheet.is_none(), "nothing to ask: it was saved");
    assert!(app.editor.doc.elements.is_empty(), "then the new document");
}

#[test]
fn saves_waiting_behind_a_new_document_are_dropped() {
    use zebra_app::app::still_for_this_document as keep;
    assert_eq!(keep(vec![Action::Save, Action::New]), vec![Action::Save, Action::New], "a save before New still saves the old document");
    assert_eq!(keep(vec![Action::New, Action::Save, Action::SaveAs]), vec![Action::New]);
    assert_eq!(keep(vec![Action::Close, Action::Save, Action::Quit]), vec![Action::Close, Action::Quit]);
}

#[test]
fn dropped_files_are_told_apart_by_their_bytes_not_their_names() {
    use zebra_app::media_ui::{dropped_kind, Dropped};
    use zebra_core::MediaKind;
    let dir = std::env::temp_dir().join(format!("zebra-dropped-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A 4 GB disk image (sparse) named .png: one read of 256 KB, no more.
    let dmg = dir.join("Installer.png");
    let f = std::fs::File::create(&dmg).unwrap();
    f.set_len(4 << 30).unwrap();
    let t = std::time::Instant::now();
    assert_eq!(dropped_kind(&dmg).unwrap(), Dropped::Other);
    assert!(t.elapsed().as_millis() < 1000);
    // A picture with no extension, a video named .txt.
    let pic = dir.join("picture");
    let mut png = vec![];
    image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    std::fs::write(&pic, &png).unwrap();
    assert_eq!(dropped_kind(&pic).unwrap(), Dropped::Media(MediaKind::Image));
    let vid = dir.join("clip.txt");
    std::fs::write(&vid, zebra_core::media::fixtures::tiny_mp4(64, 64, 2, true)).unwrap();
    assert_eq!(dropped_kind(&vid).unwrap(), Dropped::Media(MediaKind::Video));
    // A PowerPoint file goes to import, like a PDF.
    let pptx = dir.join("Deck.pptx");
    std::fs::write(&pptx, b"PK\x03\x04\x14\x00\x00\x00[Content_Types].xml").unwrap();
    assert_eq!(dropped_kind(&pptx).unwrap(), Dropped::Slides);
    // A folder, a device: an error, not a read.
    assert!(dropped_kind(&dir).is_err());
    assert!(dropped_kind(std::path::Path::new("/dev/zero")).is_err());
}

#[test]
fn pasted_pictures_are_bounded() {
    use zebra_app::media_ui::{pasted_png, MAX_PASTE_SIDE};
    // Pixels that don't match the size, or no pixels: nothing.
    assert!(pasted_png(0, 0, vec![]).is_none());
    assert!(pasted_png(100, 100, vec![0; 16]).is_none());
    assert!(pasted_png(u32::MAX, u32::MAX, vec![0; 16]).is_none());
    // Wider than the limit: scaled down to it, aspect kept.
    let w = MAX_PASTE_SIDE + 1808;
    let png = pasted_png(w, 50, vec![255; w as usize * 50 * 4]).unwrap();
    let (pw, ph) = image::ImageReader::new(std::io::Cursor::new(&png)).with_guessed_format().unwrap().into_dimensions().unwrap();
    assert_eq!(pw, MAX_PASTE_SIDE);
    assert_eq!(ph, (50.0 * MAX_PASTE_SIDE as f64 / w as f64).round() as u32);
}
