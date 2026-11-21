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
    assert_eq!(dropped_kind(&dmg).unwrap(), Dropped::Damaged, "named as a picture, but isn't one");
    assert!(t.elapsed().as_millis() < 1000);
    // Something named as nothing Zebra takes: "use a picture, ...".
    let log = dir.join("notes.log");
    std::fs::write(&log, "just text\n").unwrap();
    assert_eq!(dropped_kind(&log).unwrap(), Dropped::Other);
    // An empty PowerPoint file is damaged, not "use a PowerPoint file".
    let empty = dir.join("empty.pptx");
    std::fs::write(&empty, b"").unwrap();
    assert_eq!(dropped_kind(&empty).unwrap(), Dropped::Damaged);
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
    // A folder, a device, a pipe: an error, not a read, and the person
    // reads a plain line with the name, not the path and "not a file".
    assert!(dropped_kind(&dir).is_err());
    assert!(dropped_kind(std::path::Path::new("/dev/zero")).is_err());
    let pipe = dir.join("pipe.png");
    assert!(std::process::Command::new("mkfifo").arg(&pipe).status().unwrap().success());
    let e = dropped_kind(&pipe).unwrap_err();
    let said = zebra_app::media_ui::unreadable(&pipe, &e);
    assert_eq!(said, zebra_app::t!("app-toast-not-a-file", name = "pipe.png"));
    assert!(!said.contains(dir.to_str().unwrap()) && !said.contains("not a file"), "{said}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A pasted picture comes in at its full size: nothing is scaled down
/// (a 10,000-pixel-wide one stays 10,000 wide). Pixels that don't add up
/// are no picture.
#[test]
fn pasted_pictures_keep_their_size() {
    use zebra_app::media_ui::pasted_png;
    assert!(pasted_png(0, 0, vec![]).is_none());
    assert!(pasted_png(100, 100, vec![0; 16]).is_none());
    assert!(pasted_png(u32::MAX, u32::MAX, vec![0; 16]).is_none());
    let w = 10_000;
    let png = pasted_png(w, 50, vec![255; w as usize * 50 * 4]).unwrap();
    let (pw, ph) = image::ImageReader::new(std::io::Cursor::new(&png)).with_guessed_format().unwrap().into_dimensions().unwrap();
    assert_eq!((pw, ph), (w, 50));
}

/// Cloud pass: pasting a 12000 x 9000 screenshot held the window for 2 s
/// while the clipboard's picture was read and decoded. Now that happens
/// on a worker; the window goes on drawing, and the picture lands after.
/// With no picture there after all, Zebra's own copy is pasted instead.
#[test]
fn a_pasted_picture_is_read_off_the_window_thread() {
    use zebra_app::media_ui::paste_picture_with;
    let mut h = harness();
    let t = std::time::Instant::now();
    paste_picture_with(h.state_mut(), || {
        std::thread::sleep(std::time::Duration::from_millis(600));
        Some((64, 32, vec![200; 64 * 32 * 4]))
    });
    assert!(t.elapsed() < std::time::Duration::from_millis(200), "the window didn't wait: {:?}", t.elapsed());
    h.run_steps(2);
    assert!(h.state().editor.doc.elements.is_empty(), "not there yet");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while h.state().editor.doc.elements.is_empty() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.run_steps(1);
    }
    let app = h.state();
    assert_eq!(app.editor.doc.elements.len(), 1, "the picture landed");
    assert!(matches!(app.editor.doc.elements.values().next().unwrap().kind, zebra_core::ElementKind::Image(_)));

    // No picture on the clipboard: Zebra's own copied objects are pasted.
    h.state_mut().queued.push(Action::Copy);
    h.run_steps(2);
    paste_picture_with(h.state_mut(), || None);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while h.state().editor.doc.elements.len() < 2 && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.run_steps(1);
    }
    assert_eq!(h.state().editor.doc.elements.len(), 2, "own copy pasted");
}

/// M22 (Mac pass, 21 Nov): a slow paste showed nothing for 13 s. Like
/// Keynote, an insert or paste that takes time shows "Inserting Media…"
/// with Cancel (log row 64); a quick one shows nothing. Cancel drops the
/// picture when it arrives, and its scratch file with it.
#[test]
fn a_slow_paste_shows_inserting_media_and_cancels() {
    use egui_kittest::kittest::Queryable;
    use zebra_app::media_ui::{SHEET_AFTER, inserting_sheet_up, paste_picture_with};
    let mut h = harness();
    // Quick: no sheet.
    paste_picture_with(h.state_mut(), || Some((8, 8, vec![9; 8 * 8 * 4])));
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while h.state().editor.doc.elements.is_empty() && std::time::Instant::now() < until {
        assert!(!inserting_sheet_up(h.state()), "no sheet for a quick paste");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    assert_eq!(h.state().editor.doc.elements.len(), 1);
    // Slow: the sheet comes up after SHEET_AFTER; Cancel drops it.
    let (go, wait) = std::sync::mpsc::channel::<()>();
    paste_picture_with(h.state_mut(), move || {
        let _ = wait.recv();
        Some((64, 32, vec![200; 64 * 32 * 4]))
    });
    h.run_steps(1);
    assert!(!inserting_sheet_up(h.state()));
    std::thread::sleep(SHEET_AFTER + std::time::Duration::from_millis(100));
    h.run_steps(2);
    assert!(inserting_sheet_up(h.state()), "the sheet is up");
    h.get_by_label("Inserting Media…");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(!inserting_sheet_up(h.state()), "Cancel took the sheet down");
    go.send(()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.elements.len(), 1, "the cancelled picture never came in");
}

/// A3 (cloud pass): every non-table "too big" read "That would make the
/// table too big." Each error the person sees says what it is about.
#[test]
fn errors_say_what_they_are_about() {
    use zebra_core::error::About;
    use zebra_core::ZError;
    let say = |e: ZError| zebra_app::words::error(&e);
    let picture = say(ZError::new("too_big", "x").about(About::Picture));
    assert_eq!(picture, zebra_app::t!("app-error-picture-too-big"));
    assert!(picture.contains("picture") && !picture.contains("MB"), "{picture}");
    assert!(say(ZError::new("too_big", "x").about(About::Table)).contains("table"));
    for e in [ZError::new("too_big", "x"), ZError::new("too_large", "x"), ZError::new("too_big", "x").about(About::Poster), ZError::new("bad_file", "x").about(About::Document), ZError::new("busy", "x").about(About::Saving)] {
        assert!(!say(e.clone()).contains("table"), "{e:?}: {}", say(e.clone()));
    }
    assert!(!say(ZError::new("busy", "x").about(About::Saving)).contains("agent"));

    // A picture whose pixels can't be scaled down here (a progressive
    // JPEG claiming 60,000 x 60,000), dropped: the toast says so. Any file
    // size comes in (Kartik, 21 Nov); only this kind is refused. (On the
    // Mac, ImageIO scales these, as Keynote does.)
    #[cfg(not(target_os = "macos"))]
    {
        let mut h = harness();
        let dir = data_dir().join("over-limit");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("picture-unscalable.jpg");
        std::fs::write(&p, lying_jpeg(60_000, 60_000)).unwrap();
        let at = h.state().canvas_center_world();
        zebra_app::media_ui::insert_file(h.state_mut(), &p, at);
        h.run_steps(1);
        assert_eq!(h.state().toast.as_ref().map(|t| t.text.clone()), Some(picture.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A progressive JPEG header claiming `w` x `h`, and no pixels.
#[cfg(not(target_os = "macos"))]
fn lying_jpeg(w: u16, h: u16) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0];
    v.extend([0xFF, 0xC2, 0, 17, 8]);
    v.extend(h.to_be_bytes());
    v.extend(w.to_be_bytes());
    v.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    v.extend([0xFF, 0xD9]);
    v
}

/// A15 (cloud pass): a .zebra over the document size limit said "isn't a
/// Zebra presentation, or it's damaged". It says it's too large.
#[test]
fn a_document_over_the_limit_is_too_large_not_damaged() {
    use zebra_core::error::About;
    let mut h = harness();
    let p = data_dir().join("bomb-document.zebra");
    let e = zebra_core::ZError::new("bad_file", "the document is too large").about(About::Document);
    h.state_mut().finish_open(&p, Err(e));
    match h.state().sheet.clone() {
        Some(zebra_app::sheets::SheetKind::Alert { message, .. }) => assert_eq!(message, zebra_app::t!("app-open-failed-too-large")),
        other => panic!("{other:?}"),
    }
}

/// Mac pass 21 Nov. M9: an empty picture chosen with Insert › Image…
/// said "Couldn't do that."; it says the file is empty or damaged, as a
/// drop of it does. M20: an iPhone photo (HEIC) dropped or chosen goes
/// the picture way, not the video way.
#[test]
fn an_empty_picture_says_so_and_a_heic_is_a_picture() {
    use zebra_app::media_ui::{dropped_kind, Dropped};
    let mut h = harness();
    let dir = data_dir().join("m9-m20");
    std::fs::create_dir_all(&dir).unwrap();
    let empty = dir.join("empty.png");
    std::fs::write(&empty, b"").unwrap();
    let at = h.state().canvas_center_world();
    h.state_mut().insert_image_file(&empty, at);
    h.run_steps(1);
    assert_eq!(h.state().toast.as_ref().map(|t| t.text.clone()), Some(zebra_app::t!("app-error-damaged")));
    assert_eq!(dropped_kind(&empty).unwrap(), Dropped::Damaged);
    // The `ftyp` box of an iPhone photo, then its `meta` and `mdat`.
    let heic = dir.join("IMG_0001.HEIC");
    let mut f = b"\0\0\0\x2cftypheic\0\0\0\0mif1MiHEMiPrmiafMiHBheic".to_vec();
    f.extend(b"\0\0\0\x08meta\0\0\0\x08mdat");
    std::fs::write(&heic, &f).unwrap();
    assert_eq!(dropped_kind(&heic).unwrap(), Dropped::Media(zebra_core::MediaKind::Image));
    let _ = std::fs::remove_dir_all(&dir);
}

/// M22 follow-up (Mac recheck, 21 Nov): Cancel took the sheet down, but
/// the paste kept writing its picture for about 9 s. Now the worker stops
/// between rows, and its scratch file goes at once.
#[test]
fn cancelling_a_paste_stops_writing_it() {
    use egui_kittest::kittest::Queryable;
    use zebra_app::media_ui::{SHEET_AFTER, inserting_sheet_up, paste_picture_with};
    let mut h = harness();
    // Noise, so the PNG takes a while to write (several seconds here).
    let (w, hh) = (6000u32, 6000u32);
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let rgba: Vec<u8> = (0..w as usize * hh as usize * 4)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let big = |dir: &std::path::Path| -> Vec<std::path::PathBuf> {
        let mut out = vec![];
        let mut todo = vec![dir.to_path_buf()];
        while let Some(d) = todo.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    todo.push(p);
                } else if p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("-paste.png")) && e.metadata().is_ok_and(|m| m.len() > 1 << 20) {
                    out.push(p);
                }
            }
        }
        out
    };
    let dirs = [data_dir(), std::env::temp_dir().join("Zebra-media")];
    let writing = || dirs.iter().flat_map(|d| big(d)).collect::<Vec<_>>();
    paste_picture_with(h.state_mut(), move || Some((w, hh, rgba)));
    std::thread::sleep(SHEET_AFTER + std::time::Duration::from_millis(100));
    h.run_steps(2);
    assert!(inserting_sheet_up(h.state()), "the sheet is up");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while writing().is_empty() {
        assert!(std::time::Instant::now() < until, "the picture was never being written");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    let t = std::time::Instant::now();
    while !writing().is_empty() {
        assert!(t.elapsed() < std::time::Duration::from_millis(1500), "still writing {:?} after Cancel", writing());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(3);
    assert!(h.state().editor.doc.elements.is_empty(), "the cancelled picture never came in");
}
