//! Insert Web Video: the sheet (Insert dimmed until the field holds an
//! https link, each error line), the inspector's Link and Download rows,
//! video cards and the presenter view's messages. A local server stands in
//! for the web (`ZEBRA_TEST_WEB`, links on https://zebra-test.invalid), and
//! the fake media backend for AVFoundation, so nothing here touches the
//! internet or needs a Mac. Its own test binary: it sets the environment.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, ElementKind, LinkKind, Mode, Origin, Vec2, ViewCommand};
use zebra_media::link::STAND_IN;

// ---------------------------------------------------------------------
// The stand-in web

fn png(w: u32, h: u32) -> Vec<u8> {
    // A sunset-ish gradient, so a card's poster looks like a picture.
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let t = y as f32 / h as f32;
        let s = x as f32 / w as f32;
        image::Rgb([(230.0 - 90.0 * t) as u8, (120.0 - 60.0 * t + 40.0 * s) as u8, (60.0 + 120.0 * t) as u8])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn respond(mut s: TcpStream) {
    let mut buf = vec![];
    let mut chunk = [0u8; 4096];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let req = String::from_utf8_lossy(&buf).to_string();
    let target = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let path = target.split('?').next().unwrap_or("/").to_string();
    let mp4 = zebra_core::media::fixtures::tiny_mp4(1280, 720, 30, true);
    let head = |status: &str, ctype: &str, len: usize| format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
    let (status, ctype, body): (&str, &str, Vec<u8>) = match path.as_str() {
        "/talk.mp4" | "/slow-poster.mp4" => ("200 OK", "video/mp4", mp4),
        "/slow.mp4" => {
            // Answers, but only after the check's whole time.
            std::thread::sleep(Duration::from_secs(8));
            ("200 OK", "video/mp4", mp4)
        }
        "/trickle.mp4" => {
            // Half the clip, then a long pause: a download in progress.
            let len = 64 << 20;
            let _ = s.write_all(head("200 OK", "video/mp4", len).as_bytes());
            let _ = s.write_all(&mp4);
            let _ = s.write_all(&vec![0u8; 2 << 20]);
            std::thread::sleep(Duration::from_secs(20));
            return;
        }
        "/page.html" => ("200 OK", "text/html", b"<!doctype html><p>hi".to_vec()),
        "/private.mp4" => ("403 Forbidden", "text/plain", vec![]),
        "/live.m3u8" => ("200 OK", "application/x-mpegURL", b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nx.ts\n".to_vec()),
        "/oembed/youtube" => (
            "200 OK",
            "application/json",
            format!(r#"{{"type":"video","title":"Launch talk","width":200,"height":113,"thumbnail_url":"{STAND_IN}/thumb.png"}}"#).into_bytes(),
        ),
        "/thumb.png" => ("200 OK", "image/png", png(480, 270)),
        _ => ("404 Not Found", "text/plain", vec![]),
    };
    let _ = s.write_all(head(status, ctype, body.len()).as_bytes());
    let _ = s.write_all(&body);
}

/// The server and the environment, once for the whole binary (before any
/// app starts).
fn web() -> &'static str {
    static BASE: OnceLock<String> = OnceLock::new();
    BASE.get_or_init(|| {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                std::thread::spawn(move || respond(s));
            }
        });
        let dir = std::env::temp_dir().join(format!("zebra-web-video-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once, before the first app (or any thread that
        // reads them) starts; the server thread reads no environment.
        unsafe {
            std::env::set_var("ZEBRA_FAKE_MEDIA", "1");
            std::env::set_var("ZEBRA_MEDIA_CACHE", dir.join("cache"));
            std::env::set_var("ZEBRA_TEST_WEB", &base);
            std::env::set_var("ZEBRA_TEST_WEB_TIMEOUT_MS", "3000");
        }
        base
    })
}

// ---------------------------------------------------------------------
// Driving the app

fn harness() -> Harness<'static, ZebraApp> {
    web();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn shot(h: &mut Harness<'static, ZebraApp>, name: &str) {
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        // Posters decode and previews render off the UI thread.
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(25));
            h.step();
        }
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

/// The canvas as the Mac draws it (vello; these tests have no GPU, so the
/// egui shot shows the simple fallback painter): the same display list,
/// rasterised on the CPU.
fn canvas_shot(h: &Harness<'static, ZebraApp>, name: &str) {
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        let app = h.state();
        let ed = &app.editor;
        let opts = zebra_render::SceneOptions { card_progress: zebra_app::card_window::progress(app), ..Default::default() };
        let vp = [ed.view.viewport.x, ed.view.viewport.y];
        let list = app.renderer.build_display_list(ed, &ed.view.camera, vp, 1.0, &opts);
        let rgba = zebra_render::cpu::render_rgba(&list);
        let img = image::RgbaImage::from_raw(list.width_px, list.height_px, rgba).unwrap();
        img.save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

fn perform(h: &mut Harness<'static, ZebraApp>, a: Action) {
    h.state_mut().queued.push(a);
    h.run_steps(6);
}

/// Steps in real time until `done` holds (or `ms` pass).
fn wait_for(h: &mut Harness<'static, ZebraApp>, ms: u64, done: impl Fn(&ZebraApp) -> bool) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if done(h.state()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
        h.step();
    }
    done(h.state())
}

/// The sheet's Insert button (the menu bar has an Insert menu too: the
/// sheet's comes last).
fn insert_enabled(h: &Harness<'static, ZebraApp>) -> bool {
    let b = h.query_all_by_role_and_label(Role::Button, "Insert").last().expect("the sheet's Insert button");
    !b.accesskit_node().is_disabled()
}

fn click_insert(h: &mut Harness<'static, ZebraApp>) {
    h.query_all_by_role_and_label(Role::Button, "Insert").last().unwrap().click();
    h.run_steps(2);
}

fn set_link(h: &mut Harness<'static, ZebraApp>, text: &str) {
    let s = h.state_mut().web_sheet.as_mut().unwrap();
    s.text = text.to_string();
    s.error = zebra_app::web_video_ui::live_error(text);
    h.run_steps(2);
}

fn link_of(app: &ZebraApp, id: &str) -> Option<zebra_core::VideoLink> {
    match app.editor.doc.get(id).map(|e| &e.kind) {
        Some(ElementKind::Video(v)) => v.link.clone(),
        _ => None,
    }
}

fn apply(h: &mut Harness<'static, ZebraApp>, j: serde_json::Value) -> String {
    let cmd: Command = serde_json::from_value(j).unwrap();
    h.state_mut().editor.apply(cmd, Origin::Human).unwrap().created[0].clone()
}

fn screen(h: &Harness<'static, ZebraApp>, x: f64, y: f64) -> Pos2 {
    let app = h.state();
    app.canvas.w2s(&app.editor, Vec2::new(x, y))
}

fn click(h: &mut Harness<'static, ZebraApp>, p: Pos2, count: usize) {
    h.event(Event::PointerMoved(p));
    h.step();
    for _ in 0..count {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.step();
    }
}

// ---------------------------------------------------------------------

#[test]
fn the_sheet_dims_insert_until_the_link_is_https_and_names_each_problem() {
    let mut h = harness();
    perform(&mut h, Action::InsertWebVideo);
    assert!(h.state().web_sheet.is_some());
    h.get_by_label("Insert Web Video");
    h.get_by_label("MP4, MOV or HLS stream links.");
    assert_eq!(h.state().web_sheet.as_ref().unwrap().text, "", "an empty clipboard leaves the field empty");
    assert!(!insert_enabled(&h), "nothing to insert yet");
    shot(&mut h, "sheet-empty.png");

    // Typed into the field (focused when the sheet opens): http is named
    // at once and Insert stays dimmed.
    h.event(Event::Text("http://example.com/talk.mp4".into()));
    h.run_steps(3);
    assert_eq!(h.state().web_sheet.as_ref().unwrap().text, "http://example.com/talk.mp4");
    h.get_by_label("Use an https link.");
    assert!(!insert_enabled(&h));
    shot(&mut h, "sheet-error-http.png");

    set_link(&mut h, "https://me:secret@example.com/talk.mp4");
    h.get_by_label("Links with a password in them can't be used.");
    assert!(!insert_enabled(&h));
    shot(&mut h, "sheet-error-password.png");

    set_link(&mut h, "talk.mp4");
    assert!(!insert_enabled(&h), "not a link");

    // Each answer from the web has its own line; the sheet stays open and
    // nothing is added.
    let before = h.state().editor.doc.elements.len();
    let cases = [
        (format!("{STAND_IN}/page.html"), "This link isn't a video.", "sheet-error-not-video.png"),
        (format!("{STAND_IN}/private.mp4"), "This video needs a sign-in.", "sheet-error-sign-in.png"),
        (format!("{STAND_IN}/gone.mp4"), "Nothing found at this link.", "sheet-error-not-found.png"),
        ("https://127.0.0.1:1/talk.mp4".to_string(), "Couldn't reach this link. Check your connection.", "sheet-error-unreachable.png"),
    ];
    for (url, line, name) in cases {
        set_link(&mut h, &url);
        assert!(insert_enabled(&h), "{url} looks like a link");
        click_insert(&mut h);
        assert!(wait_for(&mut h, 6000, |a| a.web_sheet.as_ref().is_some_and(|s| !s.checking())), "{url}: the check ends");
        h.run_steps(2);
        h.get_by_label(line);
        shot(&mut h, name);
    }
    assert_eq!(h.state().editor.doc.elements.len(), before, "nothing was added");

    // Typing again clears the last answer.
    h.event(Event::Text("x".into()));
    h.run_steps(2);
    assert!(h.state().web_sheet.as_ref().unwrap().error.is_none());

    // Esc cancels.
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().web_sheet.is_none());
}

#[test]
fn a_slow_link_shows_checking_and_gives_up_in_time() {
    let mut h = harness();
    perform(&mut h, Action::InsertWebVideo);
    set_link(&mut h, &format!("{STAND_IN}/slow.mp4"));
    click_insert(&mut h);
    h.run_steps(3);
    assert!(h.state().web_sheet.as_ref().unwrap().checking());
    h.get_by_label("Checking…");
    assert!(!insert_enabled(&h), "no second Insert while checking");
    shot(&mut h, "sheet-checking.png");
    // The check's limit here is 3 s (10 s in the app).
    let t = Instant::now();
    assert!(wait_for(&mut h, 8000, |a| a.web_sheet.as_ref().is_some_and(|s| !s.checking())));
    assert!(t.elapsed() < Duration::from_secs(6), "given up at the deadline, not when the server answers");
    h.run_steps(2);
    h.get_by_label("Couldn't reach this link. Check your connection.");
}

#[test]
fn the_clipboard_link_fills_the_field() {
    let mut h = harness();
    let link = format!("{STAND_IN}/talk.mp4");
    zebra_app::web_video_ui::open_with(h.state_mut(), Some(format!("  {link}\n")));
    h.run_steps(3);
    assert_eq!(h.state().web_sheet.as_ref().unwrap().text, link);
    h.key_press(Key::Escape);
    h.run_steps(2);
    // Anything else on the clipboard is left out.
    for clip in ["just some words", "file:///etc/passwd", "javascript:alert(1)", ""] {
        zebra_app::web_video_ui::open_with(h.state_mut(), Some(clip.into()));
        assert_eq!(h.state().web_sheet.as_ref().unwrap().text, "", "{clip:?}");
        h.state_mut().web_sheet = None;
    }
    let huge = format!("{STAND_IN}/{}.mp4", "a".repeat(1 << 20));
    zebra_app::web_video_ui::open_with(h.state_mut(), Some(huge));
    assert_eq!(h.state().web_sheet.as_ref().unwrap().text, "", "a megabyte link isn't pasted");
}

#[test]
fn a_file_link_is_inserted_with_its_link_mark_and_inspector_rows() {
    let mut h = harness();
    perform(&mut h, Action::InsertWebVideo);
    let undo_before = h.state().editor.history().len();
    // Typed, then Return.
    h.event(Event::Text(format!("{STAND_IN}/talk.mp4")));
    h.run_steps(2);
    assert!(insert_enabled(&h));
    h.key_press(Key::Enter);
    assert!(wait_for(&mut h, 8000, |a| a.web_sheet.is_none()), "inserted and closed");
    let app = h.state();
    let sel = app.selection();
    assert_eq!(sel.len(), 1, "the new video is selected");
    let id = sel[0].clone();
    let link = link_of(app, &id).expect("a web video");
    assert_eq!(link.url, format!("{STAND_IN}/talk.mp4"));
    assert_eq!(link.kind, LinkKind::File);
    let ElementKind::Video(v) = &app.editor.doc.get(&id).unwrap().kind else { unreachable!() };
    assert!(v.poster.is_some(), "a poster from the clip");
    assert!(v.asset.is_empty(), "the file keeps only the link and the poster");
    let e = app.editor.doc.get(&id).unwrap();
    assert!((e.size.w / e.size.h - 16.0 / 9.0).abs() < 0.01, "its own shape");
    assert_eq!(app.editor.history().len(), undo_before + 1, "one undo step");
    assert_eq!(app.editor.undo_label().as_deref(), Some("Insert Video (You)"));

    // The inspector: the site, Copy Link and Download.
    h.state_mut().inspector_tab = zebra_app::inspector::Tab::Style;
    h.state_mut().view_cmd(ViewCommand::FitAll { animate: false });
    h.run_steps(6);
    h.get_by_label("zebra-test.invalid");
    h.get_by_label("Copy Link").click();
    h.run_steps(3);
    assert_eq!(h.state().toast.as_ref().map(|t| t.text.as_str()), Some("Link copied"));
    let dl = h.get_by_role_and_label(Role::Button, "Download");
    assert!(!dl.accesskit_node().is_disabled(), "a file link can be downloaded");
    shot(&mut h, "web-video-selected.png");
    canvas_shot(&h, "web-video-canvas.png");

    // Download: the clip goes into the deck, as one undo step.
    let undo_before = h.state().editor.history().len();
    h.get_by_role_and_label(Role::Button, "Download").click();
    assert!(wait_for(&mut h, 8000, |a| link_of(a, &id).is_none()), "downloaded");
    let app = h.state();
    let ElementKind::Video(v) = &app.editor.doc.get(&id).unwrap().kind else { unreachable!() };
    assert!(!v.asset.is_empty(), "a local video now");
    assert_eq!(app.editor.history().len(), undo_before + 1);
    assert!(app.media.downloads.is_empty());
    let dir = zebra_core::media::cache_dir().join("downloads");
    // (Another test's download may be under way: its .part file.)
    let left = std::fs::read_dir(&dir).map(|d| d.flatten().map(|f| f.file_name().to_string_lossy().to_string()).filter(|n| !n.ends_with(".part")).count()).unwrap_or(0);
    assert_eq!(left, 0, "the temporary download is gone");
    // Undo brings the link back.
    h.state_mut().editor.undo(Origin::Human).unwrap();
    assert!(link_of(h.state(), &id).is_some());
}

#[test]
fn a_download_shows_progress_and_can_be_cancelled() {
    let mut h = harness();
    let id = apply(&mut h, json!({"op":"add_web_video","url":format!("{STAND_IN}/trickle.mp4"),"width":800,
                                  "found":{"kind":"file","width":1280,"height":720,"duration":20}}));
    h.state_mut().select(vec![id.clone()]);
    h.state_mut().inspector_tab = zebra_app::inspector::Tab::Style;
    h.state_mut().view_cmd(ViewCommand::FitAll { animate: false });
    h.run_steps(4);
    h.get_by_role_and_label(Role::Button, "Download").click();
    assert!(wait_for(&mut h, 5000, |a| a.media.downloads.get(&id).is_some_and(|d| d.bytes() > 0)));
    h.run_steps(3);
    h.get_by_label_contains("Downloading");
    assert!(h.query_by_role_and_label(Role::Button, "Download").is_none(), "Cancel instead while it runs");
    shot(&mut h, "inspector-downloading.png");
    let history = h.state().editor.history().len();
    h.query_all_by_role_and_label(Role::Button, "Cancel").last().unwrap().click();
    h.run_steps(3);
    assert!(h.state().media.downloads.is_empty());
    assert!(link_of(h.state(), &id).is_some(), "still a web video");
    assert_eq!(h.state().editor.history().len(), history, "nothing changed");
    // The worker deletes its part file as it stops.
    let dir = zebra_core::media::cache_dir().join("downloads");
    let mine = || std::fs::read_dir(&dir).map(|d| d.flatten().filter(|f| f.file_name().to_string_lossy().starts_with(&format!("{}-", std::process::id()))).count()).unwrap_or(0);
    let end = Instant::now() + Duration::from_secs(5);
    while mine() > 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(mine(), 0, "no part file left");
}

#[test]
fn a_live_stream_cant_be_downloaded_or_trimmed() {
    let mut h = harness();
    let id = apply(&mut h, json!({"op":"add_web_video","url":format!("{STAND_IN}/live.m3u8"),"width":800,
                                  "found":{"kind":"live","width":1280,"height":720}}));
    h.state_mut().select(vec![id]);
    h.state_mut().inspector_tab = zebra_app::inspector::Tab::Style;
    h.run_steps(5);
    h.get_by_label("Live stream: Start, End and Loop don't apply.");
    let dl = h.get_by_role_and_label(Role::Button, "Download");
    assert!(dl.accesskit_node().is_disabled(), "a stream can't be downloaded");
    shot(&mut h, "inspector-live.png");
}

#[test]
fn a_youtube_link_becomes_a_card_that_opens_and_resumes() {
    let mut h = harness();
    perform(&mut h, Action::InsertWebVideo);
    set_link(&mut h, "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=65");
    click_insert(&mut h);
    assert!(wait_for(&mut h, 8000, |a| a.web_sheet.is_none()), "inserted");
    let id = h.state().selection()[0].clone();
    let link = link_of(h.state(), &id).unwrap();
    assert!(link.kind.is_card());
    {
        let app = h.state();
        let ElementKind::Video(v) = &app.editor.doc.get(&id).unwrap().kind else { unreachable!() };
        assert_eq!(v.start, 65.0, "the link's own time");
        assert!(v.poster.is_some(), "the site's thumbnail");
        assert_eq!(v.alt, "Launch talk");
    }
    h.state_mut().inspector_tab = zebra_app::inspector::Tab::Style;
    h.state_mut().view_cmd(ViewCommand::FitAll { animate: false });
    h.run_steps(5);
    h.get_by_label("YouTube");
    assert!(h.query_by_role_and_label(Role::Button, "Download").is_none(), "a card has no Download");

    // Double-click (editing) opens it; Esc closes it again.
    h.state_mut().select(vec![]);
    // Long enough after the last click that this isn't a triple-click.
    h.run_steps(20);
    let b = h.state().editor.doc.get(&id).unwrap().bounds();
    let mid = screen(&h, b.center().x, b.center().y);
    click(&mut h, mid, 2);
    h.run_steps(2);
    assert_eq!(h.state().media.card.as_ref().map(|c| c.id.clone()), Some(id.clone()), "the play window opened");
    assert_eq!(zebra_app::card_window::playing_site(h.state()).as_deref(), Some("YouTube"));
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().media.card.is_none(), "Esc closes it");

    // Where it got to shows as a line along the bottom.
    h.state_mut().media.card_resume.insert(id.clone(), zebra_app::card_window::Resume { at: 80.0, length: Some(212.0) });
    let p = zebra_app::card_window::progress(h.state());
    assert!((p[&id] - 80.0 / 212.0).abs() < 1e-4);
    h.state_mut().select(vec![]);
    shot(&mut h, "youtube-card.png");
    canvas_shot(&h, "youtube-card-canvas.png");

    // Presenting from step 1 starts it again from its start.
    h.key_press(Key::F5);
    h.run_steps(3);
    assert!(h.state().media.card_resume.is_empty());
}

#[test]
fn the_presenter_view_says_when_a_web_video_loads_or_fails() {
    let mut h = harness();
    // "slow" never loads, "fail" fails (the fake player).
    let slow = apply(&mut h, json!({"op":"add_web_video","url":format!("{STAND_IN}/slow-talk.mp4"),"x":0,"y":0,"width":800,
                                    "found":{"kind":"file","width":1280,"height":720,"duration":20}}));
    let fail = apply(&mut h, json!({"op":"add_web_video","url":format!("{STAND_IN}/fail-talk.mp4"),"x":3000,"y":0,"width":800,
                                    "found":{"kind":"file","width":1280,"height":720,"duration":20}}));
    for id in [&slow, &fail] {
        let cmd: Command = serde_json::from_value(json!({"op":"add_step","target":{"kind":"element","id":id}})).unwrap();
        h.state_mut().editor.apply(cmd, Origin::Human).unwrap();
    }
    h.ctx.set_embed_viewports(true);
    h.state_mut().stage.presenter_view = true;
    h.state_mut().editor.view(ViewCommand::Present { step: Some(0) }, Origin::Human).unwrap();
    h.state_mut().last_pointer_move = 1e9;
    h.run_steps(10);
    assert_eq!(h.state().editor.view.mode, Mode::Present);
    // The next step's video is loading already.
    assert!(h.state().media.web_loading(&slow));
    assert!(h.state().media.web_loading(&fail) || h.state().media.web_failed(&fail), "the next step's video preloads");
    assert_eq!(zebra_app::web_video_ui::presenter_status(h.state()).as_deref(), Some("Loading video…"));
    shot(&mut h, "presenter-loading.png");

    h.state_mut().editor.view(ViewCommand::Present { step: Some(1) }, Origin::Human).unwrap();
    assert!(wait_for(&mut h, 3000, |a| a.media.web_failed(&fail)));
    h.run_steps(30);
    assert_eq!(zebra_app::web_video_ui::presenter_status(h.state()).as_deref(), Some("Video couldn't play"));
    assert!(h.state().toast.is_none(), "no alerts while presenting");
    shot(&mut h, "presenter-failed.png");
}
