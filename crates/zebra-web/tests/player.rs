//! The web player's logic and the exporter, run natively (the browser
//! check in tests/browser/check.js runs the same player as wasm).
use zebra_core::model::{MediaKind, StepTarget};
use zebra_core::{Command, Editor, Origin, ViewCommand};
use zebra_render::{CaptureTarget, Renderer};
use zebra_web::export::{self, Labels};
use zebra_web::{Click, Player};

const H: Origin = Origin::Human;

/// Two frames, the second rotated, with two fade-in builds on the first.
fn deck(r: &Renderer) -> Editor {
    let mut ed = Editor::default();
    ed.measure = r.measure();
    let cmds = vec![
        Command::AddFrame { style: Default::default(), x: 0.0, y: 0.0, w: 1000.0, h: 700.0, rotation: 0.0, name: None, id: Some("f1".into()), add_to_path: false, find_space: false },
        Command::AddText { text: "Hello web".into(), x: 0.0, y: -200.0, style: zebra_core::TextStyle::Title, scale: None, place: None, id: Some("t1".into()), markdown: false, on: None, color: None },
        Command::AddShape { shape: zebra_core::ShapeKind::Ellipse, x: -200.0, y: 100.0, w: 150.0, h: 150.0, rotation: 0.0, fill: None, stroke: None, id: Some("a".into()) },
        Command::AddShape { shape: zebra_core::ShapeKind::Rectangle, x: 200.0, y: 100.0, w: 150.0, h: 150.0, rotation: 0.0, fill: None, stroke: None, id: Some("b".into()) },
        Command::AddFrame { style: Default::default(), x: 2000.0, y: 500.0, w: 800.0, h: 500.0, rotation: 20.0, name: None, id: Some("f2".into()), add_to_path: false, find_space: false },
        Command::AddText { text: "Turned".into(), x: 2000.0, y: 500.0, style: zebra_core::TextStyle::Body, scale: None, place: None, id: None, markdown: false, on: None, color: None },
        Command::AddStep { target: StepTarget::Element { id: "f1".into() }, index: None, builds: vec!["a".into(), "b".into()], notes: String::new() },
        Command::AddStep { target: StepTarget::Element { id: "f2".into() }, index: None, builds: vec![], notes: String::new() },
    ];
    for c in cmds {
        ed.apply(c, H).unwrap();
    }
    let _ = ed.view(ViewCommand::SetViewport { width: 640.0, height: 400.0 }, H);
    ed
}

/// A player of `ed`'s document fed the way the page feeds it.
fn player_for(r: &Renderer, ed: &Editor) -> Player {
    let mut p = Player::new();
    for f in r.font_files_used(ed) {
        p.add_font(f);
    }
    for (k, a) in &ed.doc.assets {
        p.add_asset(k, &a.ext, a.width, a.height, a.bytes.to_vec());
    }
    p.load(&zebra_core::io::to_json(&ed.doc)).unwrap();
    p.resize(640.0, 400.0, 1.0);
    p
}

/// Run frames until nothing animates.
fn settle(p: &mut Player, t: &mut f64) {
    for _ in 0..1000 {
        *t += 1.0 / 60.0;
        if p.frame(*t).is_none_or(|w| w > 0.0) {
            break;
        }
    }
    *t += 1.0;
    p.frame(*t);
}

fn capture_rgba(r: &Renderer, ed: &Editor, index: usize, build: usize) -> Vec<u8> {
    let list = r.capture_display_list(ed, &CaptureTarget::Step { index, build: Some(build) }, 640, 400).unwrap();
    zebra_render::cpu::render_rgba(&list)
}

#[test]
fn every_step_draws_what_capture_draws() {
    let r = Renderer::new();
    let ed = deck(&r);
    let mut p = player_for(&r, &ed);
    let mut t = 0.0;
    settle(&mut p, &mut t);
    for (step, build) in [(0, 0), (0, 1), (0, 2), (1, 0)] {
        let s = p.state();
        assert_eq!((s["step"].as_u64(), s["build"].as_u64()), (Some(step), Some(build)));
        assert_eq!((p.width_px, p.height_px), (640, 400));
        let want = capture_rgba(&r, &ed, step as usize, build as usize);
        let differ = p.pixels.chunks(4).zip(want.chunks(4)).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 2)).count();
        assert_eq!(differ, 0, "step {step} build {build}: {differ} pixels differ from the capture");
        p.next();
        settle(&mut p, &mut t);
    }
    assert_eq!(p.state()["at_end"], true);
}

#[test]
fn builds_fade_in_and_the_camera_flies() {
    let r = Renderer::new();
    let ed = deck(&r);
    let mut p = player_for(&r, &ed);
    let mut t = 10.0;
    settle(&mut p, &mut t);
    let before = p.frames;
    p.next(); // first build
    assert_eq!(p.frame(t + 0.01), Some(0.0), "a fade animates");
    let mid = p.pixels.clone();
    settle(&mut p, &mut t);
    assert_ne!(mid, p.pixels, "the fade ends on a different frame than it starts");
    assert!(p.frames > before + 1);
    // Idle: no frames, no wake-up.
    let n = p.frames;
    assert_eq!(p.frame(t + 5.0), None);
    assert_eq!(p.frames, n, "nothing drawn while idle");
    // To the rotated frame: a flight of several frames.
    p.next();
    p.next();
    let n = p.frames;
    settle(&mut p, &mut t);
    assert!(p.frames > n + 10, "the flight drew {} frames", p.frames - n);
    // Prev from a build-free step lands on the last build of the step before.
    p.prev();
    let s = p.state();
    assert_eq!((s["step"].as_u64(), s["build"].as_u64()), (Some(0), Some(2)));
}

#[test]
fn clicks_zoom_advance_and_wheel_leaves_the_path() {
    let r = Renderer::new();
    let ed = deck(&r);
    let mut p = player_for(&r, &ed);
    let mut t = 0.0;
    settle(&mut p, &mut t);
    // A corner of the screen holds nothing: next.
    assert_eq!(p.click(2.0, 2.0), Click::Next);
    assert_eq!(p.state()["build"], 1);
    // The title: zoom to it, off the path.
    let c = ed.camera_for_step(0);
    let at = c.world_to_screen(zebra_core::Vec2::new(0.0, -200.0), zebra_core::Vec2::new(640.0, 400.0));
    assert_eq!(p.click(at.x, at.y), Click::Zoom);
    assert_eq!(p.state()["off_path"], true);
    p.next();
    assert_eq!(p.state()["off_path"], false, "next returns to the path");
    p.zoom(1.5, 100.0, 100.0);
    assert_eq!(p.state()["off_path"], true);
}

#[test]
fn page_carries_player_document_and_used_fonts_only() {
    let r = Renderer::new();
    let ed = deck(&r);
    let page = export::build(&r, &ed, &Labels::default(), export::INLINE_LIMIT).unwrap();
    assert!(!page.is_folder());
    let html = &page.html;
    assert!(html.starts_with("<!doctype html>"));
    let start = html.find("id=\"zw-manifest\">").unwrap() + "id=\"zw-manifest\">".len();
    let end = start + html[start..].find("</script>").unwrap();
    let m: serde_json::Value = serde_json::from_str(&html[start..end]).unwrap();
    // Only Inter Regular (titles and body are both Inter here).
    assert_eq!(m["fonts"].as_array().unwrap().len(), r.font_files_used(&ed).len());
    assert_eq!(r.font_files_used(&ed).len(), 1);
    for id in [&m["wasm"], &m["doc"]] {
        assert!(html.contains(&format!("id=\"{}\">", id.as_str().unwrap())));
    }
    assert!(html.contains("zw_frame"), "the page script is inlined");
}

#[test]
fn big_media_moves_into_a_folder() {
    let r = Renderer::new();
    let mut ed = deck(&r);
    let mp4 = zebra_core::media::fixtures::tiny_mp4(320, 180, 2, true);
    let b64 = |d: &[u8]| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, d);
    ed.apply(Command::AddVideo { data_base64: b64(&mp4), path: None, x: 0.0, y: 2000.0, width: Some(400.0), alt: String::new(), poster_base64: None, autoplay: true, looping: false, muted: false, id: Some("v".into()) }, H).unwrap();
    let dir = std::env::temp_dir().join(format!("zebra-web-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Under the limit: one file, the video inside it.
    let one = export::export(&r, &ed, &Labels::default(), &dir.join("talk.html")).unwrap();
    assert!(!one.folder && one.page == dir.join("talk.html"));
    // Over it: a folder with the page and the video beside it.
    let w = export::export_with_limit(&r, &ed, &Labels::default(), &dir.join("big.html"), 1).unwrap();
    assert!(w.folder);
    assert_eq!(w.page, dir.join("big").join("index.html"));
    let key = ed.doc.assets.iter().find(|(_, a)| a.media_kind() == MediaKind::Video).unwrap().0;
    let file = dir.join("big/media").join(format!("{key}.mp4"));
    assert_eq!(std::fs::read(&file).unwrap(), mp4);
    let html = std::fs::read_to_string(&w.page).unwrap();
    assert!(html.contains(&format!("\"src\":\"media/{key}.mp4\"")));
    // Pictures only: never a folder, whatever the size.
    let r2 = Renderer::new();
    let pics = deck(&r2);
    assert!(!export::build(&r2, &pics, &Labels::default(), 1).unwrap().is_folder());
    let _ = std::fs::remove_dir_all(&dir);
}
