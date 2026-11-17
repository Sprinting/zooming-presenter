//! Media interactions with the fake media backend (synthetic frames, wall
//! clock), so playback logic is tested without a Mac.
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use serde_json::json;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::media::fixtures::{tiny_mp4, tiny_wav};
use zebra_core::{Command, ElementKind, Mode, Origin, Vec2, ViewCommand};

fn harness() -> Harness<'static, ZebraApp> {
    let dir = std::env::temp_dir().join(format!("zebra-media-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    unsafe {
        std::env::set_var("ZEBRA_FAKE_MEDIA", "1");
        std::env::set_var("ZEBRA_MEDIA_CACHE", dir.join("cache"));
    }
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn file(name: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("zebra-media-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p.to_string_lossy().to_string()
}

fn apply(h: &mut Harness<'static, ZebraApp>, j: serde_json::Value) -> Vec<String> {
    let cmd: Command = serde_json::from_value(j).unwrap();
    h.state_mut().editor.apply(cmd, Origin::Human).unwrap().created
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

fn settle(h: &mut Harness<'static, ZebraApp>, ms: u64) {
    for _ in 0..(ms / 20).max(1) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.step();
    }
}

#[test]
fn double_click_plays_and_pauses_a_video_in_place() {
    let mut h = harness();
    let path = file("clip.mp4", &tiny_mp4(320, 180, 30, true));
    let id = apply(&mut h, json!({"op":"add_video","path":path,"x":0,"y":0,"width":400}))[0].clone();
    h.state_mut().view_cmd(ViewCommand::FitAll { animate: false });
    h.run_steps(3);
    let c = screen(&h, 0.0, 0.0);
    click(&mut h, c, 2);
    settle(&mut h, 500);
    assert!(h.state().media.is_playing(&id), "double-click should start the video");
    let mut opts = zebra_render::SceneOptions::default();
    h.state().media.scene(&mut opts);
    assert!(opts.video_frames.contains_key(&id), "a live frame should be shown");
    assert!(opts.playing.contains(&id));
    // Frames are converted near on-screen size, not above the video's.
    assert!(opts.video_frames[&id].width <= 512);
    click(&mut h, c, 2);
    settle(&mut h, 60);
    assert!(!h.state().media.is_playing(&id));
    // Playing changed nothing in the document.
    assert!(!h.state().editor.can_redo());
    assert_eq!(h.state().editor.history().len(), 1);
}

#[test]
fn trim_end_pauses_and_loop_restarts() {
    let mut h = harness();
    let path = file("short.mp4", &tiny_mp4(64, 36, 10, true));
    let id = apply(&mut h, json!({"op":"add_video","path":path,"width":300}))[0].clone();
    apply(&mut h, json!({"op":"update","id":id,"patch":{"start":1.0,"end":1.3}}));
    h.state_mut().view_cmd(ViewCommand::FitAll { animate: false });
    h.run_steps(2);
    zebra_app::media_ui::play(h.state_mut(), &id);
    settle(&mut h, 600);
    assert!(!h.state().media.is_playing(&id), "should stop at the trim end");
    apply(&mut h, json!({"op":"update","id":id,"patch":{"loop":true}}));
    zebra_app::media_ui::play(h.state_mut(), &id);
    settle(&mut h, 700);
    assert!(h.state().media.is_playing(&id), "looping keeps playing past the end");
}

#[test]
fn presenting_autoplays_step_videos_and_plays_soundtrack() {
    let mut h = harness();
    let f = apply(&mut h, json!({"op":"add_frame","x":0,"y":0,"w":800,"h":600,"add_to_path":true}))[0].clone();
    let _f2 = apply(&mut h, json!({"op":"add_frame","x":3000,"y":0,"w":800,"h":600,"add_to_path":true}));
    let path = file("auto.mp4", &tiny_mp4(320, 180, 30, true));
    let v = apply(&mut h, json!({"op":"add_video","path":path,"x":0,"y":0,"width":400}))[0].clone();
    let song = file("song.wav", &tiny_wav(2.0));
    apply(&mut h, json!({"op":"set_soundtrack","path":song}));
    apply(&mut h, json!({"op":"set_voiceover","index":1,"path":song}));
    let _ = f;
    h.state_mut().view_cmd(ViewCommand::Present { step: Some(0) });
    settle(&mut h, 1500);
    assert_eq!(h.state().editor.view.mode, Mode::Present);
    assert!(h.state().media.is_playing(&v), "video on step 1 should auto-play");
    let (players, _) = h.state().media.stats();
    assert!(players >= 2, "video and soundtrack should be open, got {players}");
    // Next step: the video stops, the voice-over starts.
    h.key_press(Key::ArrowRight);
    settle(&mut h, 1500);
    assert!(!h.state().media.is_playing(&v));
    // Leaving present mode stops everything.
    h.key_press(Key::Escape);
    settle(&mut h, 300);
    assert_eq!(h.state().editor.view.mode, Mode::Edit);
    assert!(!h.state().media.is_playing(&v));
    assert_eq!(h.state().media.stats().0, 0, "music and voice closed after presenting");
}

#[test]
fn crop_mode_drag_is_one_undo_step() {
    let mut h = harness();
    let img = image::RgbaImage::from_pixel(400, 200, image::Rgba([200, 40, 40, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let path = file("pic.png", &png);
    let bad: Command = serde_json::from_value(json!({"op":"replace_image","id":"nope","path":path})).unwrap();
    assert!(h.state_mut().editor.apply(bad, Origin::Human).is_err());
    let b64 = base64_encode(&png);
    let id = apply(&mut h, json!({"op":"add_image","data_base64":b64,"x":0,"y":0,"width":400}))[0].clone();
    h.state_mut().view_cmd(ViewCommand::FitAll { animate: false });
    h.run_steps(3);
    let hist0 = h.state().editor.history().len();
    let c = screen(&h, 0.0, 0.0);
    click(&mut h, c, 2);
    assert!(zebra_app::media_ui::crop_active(h.state()));
    // Drag the left edge handle to the right.
    let left = screen(&h, -200.0, 0.0);
    let to = screen(&h, -100.0, 0.0);
    h.event(Event::PointerMoved(left));
    h.step();
    h.event(Event::PointerButton { pos: left, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=8 {
        h.event(Event::PointerMoved(left + (to - left) * (i as f32 / 8.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(!zebra_app::media_ui::crop_active(h.state()));
    let app = h.state();
    let ElementKind::Image(i) = &app.editor.doc.get(&id).unwrap().kind else { panic!() };
    let c = i.crop.expect("cropped");
    assert!((c.x - 0.25).abs() < 0.03 && (c.w - 0.75).abs() < 0.03, "{c:?}");
    // The right edge stayed put.
    let e = app.editor.doc.get(&id).unwrap();
    assert!(((e.transform.x + e.world_size().0 / 2.0) - 200.0).abs() < 2.0);
    assert_eq!(app.editor.history().len(), hist0 + 1, "one undo step for the whole crop");
}

fn base64_encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}
