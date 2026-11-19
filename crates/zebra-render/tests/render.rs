mod common;

use common::*;
use zebra_core::*;
use zebra_render::{CaptureTarget, Renderer, SceneOptions};

fn decode(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory(png).unwrap().to_rgba8()
}

fn px(img: &image::RgbaImage, x: u32, y: u32) -> [u8; 4] {
    img.get_pixel(x, y).0
}

fn close(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (*x as i32 - *y as i32).abs() <= tol)
}

#[test]
fn samples_for_review() {
    let r = Renderer::new();
    for theme in Theme::ids() {
        let ed = sample(&r, &theme);
        let png = r.capture(&ed, &CaptureTarget::Overview, 1280, 800).unwrap();
        std::fs::write(out_dir().join(format!("overview_{theme}.png")), &png).unwrap();
        let img = decode(&png);
        let bg = Theme::resolve(&theme).color(&Theme::resolve(&theme).background);
        let non_bg = img.pixels().filter(|p| !close(p.0, bg, 3)).count();
        assert!(non_bg > 10_000, "{theme}: capture looks blank ({non_bg} non-background pixels)");
    }
    let ed = sample(&r, "classic");
    for i in 0..ed.doc.path.len() {
        let png = r.capture(&ed, &CaptureTarget::Step { index: i, build: None }, 960, 600).unwrap();
        std::fs::write(out_dir().join(format!("step_{i}.png")), &png).unwrap();
    }
}

#[test]
fn frame_and_shape_pixels() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(400.0, 300.0);
    let th = Theme::resolve("classic");
    // Rectangle frame filled with frame_fill, a red square on top, a bracket frame.
    add(&mut ed, "f", 0.0, 0.0, 300.0, 200.0, 1.0, 0.0, frame(FrameStyle::Rectangle));
    add(&mut ed, "s", 0.0, 0.0, 40.0, 40.0, 1.0, 0.0, shape(ShapeKind::Rectangle, Some("#ff0000"), None, 0.0));
    add(&mut ed, "b", 0.0, 0.0, 380.0, 280.0, 1.0, 0.0, frame(FrameStyle::Bracket));
    let cam = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let png = r.capture(&ed, &CaptureTarget::Camera { camera: cam }, 400, 300).unwrap();
    std::fs::write(out_dir().join("pixels.png"), &png).unwrap();
    let img = decode(&png);
    assert!(close(px(&img, 200, 150), [255, 0, 0, 255], 2), "centre is the red square");
    assert!(close(px(&img, 200 - 100, 150), th.color(&th.frame_fill), 2), "inside frame is frame_fill");
    assert!(close(px(&img, 5, 5), th.color(&th.background), 2), "corner gap is background");
    // Bracket: left bar at x = 10 + t/2 (t = 280*0.012 = 3.36), at mid height.
    let frame_c = th.color(&th.frame);
    assert!(close(px(&img, 11, 150), frame_c, 10), "left bracket bar: {:?}", px(&img, 11, 150));
    assert!(close(px(&img, 388, 150), frame_c, 10), "right bracket bar: {:?}", px(&img, 388, 150));
    // Top arm on the left exists, but the middle of the top edge is open.
    assert!(close(px(&img, 20, 11), frame_c, 10), "top-left arm: {:?}", px(&img, 20, 11));
    assert!(close(px(&img, 200, 11), th.color(&th.background), 2), "top edge open between brackets");
}

#[test]
fn rotation_is_clockwise() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(200.0, 200.0);
    // A long thin bar rotated 45deg clockwise (y down): it should run from
    // top-left to bottom-right.
    add(&mut ed, "s", 0.0, 0.0, 200.0, 10.0, 1.0, 45.0, shape(ShapeKind::Rectangle, Some("#000000"), None, 0.0));
    let cam = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let img = decode(&r.capture(&ed, &CaptureTarget::Camera { camera: cam }, 200, 200).unwrap());
    assert!(close(px(&img, 50, 50), [0, 0, 0, 255], 5));
    assert!(close(px(&img, 150, 150), [0, 0, 0, 255], 5));
    assert!(close(px(&img, 150, 50), [255, 255, 255, 255], 5));
    // A camera rolled by 45 shows the bar horizontal.
    let cam = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 45.0 };
    let img = decode(&r.capture(&ed, &CaptureTarget::Camera { camera: cam }, 200, 200).unwrap());
    assert!(close(px(&img, 20, 100), [0, 0, 0, 255], 5));
    assert!(close(px(&img, 100, 20), [255, 255, 255, 255], 5));
}

#[test]
fn deep_zoom_is_stable() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(200.0, 200.0);
    // A tiny square far from the origin, viewed at zoom 1e4.
    let (x, y) = (1.0e6 + 0.123, -2.0e6 + 0.456);
    add(&mut ed, "s", x, y, 0.01, 0.01, 1.0, 0.0, shape(ShapeKind::Rectangle, Some("#00ff00"), None, 0.0));
    let cam = Camera { cx: x, cy: y, zoom: 1.0e4, rotation: 0.0 };
    let img = decode(&r.capture(&ed, &CaptureTarget::Camera { camera: cam }, 200, 200).unwrap());
    // 0.01 world * 1e4 = 100 px square centred.
    assert!(close(px(&img, 100, 100), [0, 255, 0, 255], 2));
    assert!(close(px(&img, 52, 100), [0, 255, 0, 255], 2));
    assert!(close(px(&img, 48, 100), [255, 255, 255, 255], 2));
    assert!(close(px(&img, 148, 148), [0, 255, 0, 255], 2));
    assert!(close(px(&img, 152, 152), [255, 255, 255, 255], 2));
}

#[test]
fn culling_and_options() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    add(&mut ed, "a", 0.0, 0.0, 100.0, 100.0, 1.0, 0.0, shape(ShapeKind::Rectangle, None, None, 0.0));
    add(&mut ed, "far", 1.0e5, 0.0, 100.0, 100.0, 1.0, 0.0, shape(ShapeKind::Rectangle, None, None, 0.0));
    add(&mut ed, "tiny", 0.0, 0.0, 1.0e-4, 1.0e-4, 1.0, 0.0, shape(ShapeKind::Rectangle, None, None, 0.0));
    add(&mut ed, "inv", 0.0, 0.0, 200.0, 200.0, 1.0, 0.0, frame(FrameStyle::Invisible));
    let cam = Camera::default();
    let vp = [400.0, 300.0];
    let n = |o: &SceneOptions| r.build_display_list(&ed, &cam, vp, 2.0, o).items.len();
    let edit = SceneOptions::default();
    // "a" (one fill) + invisible frame dashed outline; "far" and "tiny" culled.
    assert_eq!(n(&edit), 2);
    let list = r.build_display_list(&ed, &cam, vp, 2.0, &edit);
    assert_eq!((list.width_px, list.height_px), (800, 600));
    let present = SceneOptions { presenting: true, ..Default::default() };
    assert_eq!(n(&present), 1, "invisible frame hidden when presenting");
    let mut hidden = SceneOptions { presenting: true, ..Default::default() };
    hidden.hidden.insert("a".into());
    assert_eq!(n(&hidden), 0);
    let skip = SceneOptions { skip: Some("a".into()), ..Default::default() };
    assert_eq!(n(&skip), 1);
    let fade = SceneOptions { opacity: vec![("a".into(), 0.5)], ..Default::default() };
    assert_eq!(n(&fade), 4, "push opacity + fill + pop + frame");
    // Rebasing: far-away content near the camera still gives small device coords.
    let cam = Camera { cx: 1.0e5, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let list = r.build_display_list(&ed, &cam, vp, 1.0, &edit);
    assert_eq!(list.items.len(), 1);
    if let zebra_render::display::Item::Fill { path, .. } = &list.items[0] {
        let b = kurbo::Shape::bounding_box(path);
        assert!((b.x0 - 150.0).abs() < 1e-6 && (b.y1 - 200.0).abs() < 1e-6, "{b:?}");
    } else {
        panic!("expected fill");
    }
}

#[test]
fn measure_matches_draw() {
    let r = Renderer::new();
    let m = r.measure();
    let th = Theme::resolve("paper");
    let t = TextData { text: "Hello world, this wraps around".into(), style: TextStyle::Title, ..Default::default() };
    let (w, h) = m.measure(&t, &th, None);
    assert!(w > 300.0 && (h - 72.0 * 1.25).abs() < 0.5, "single line: {w}x{h}");
    let (w2, h2) = m.measure(&t, &th, Some(400.0));
    assert_eq!(w2, 400.0);
    assert!(h2 > h * 1.9, "wraps to several lines: {h2}");
    // Ink bounding box of drawn glyphs lies inside the measured box.
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.doc.theme = "paper".into();
    add(&mut ed, "t", 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, ElementKind::Text(t.clone()));
    let e = ed.doc.get("t").unwrap();
    assert_eq!((e.size.w, e.size.h), (w, h));
    let cam = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let list = r.build_display_list(&ed, &cam, [1400.0, 400.0], 1.0, &SceneOptions::default());
    let zebra_render::display::Item::Text { layout, .. } = &list.items[0] else { panic!() };
    assert_eq!((layout.width, layout.height), (w, h));
    let max_x = layout.runs.iter().flat_map(|r| r.glyphs.iter()).map(|g| g.x + g.advance).fold(0.0f32, f32::max);
    assert!((max_x as f64 - w).abs() < 1.0, "glyph extent {max_x} vs measured {w}");
    // Bullets and alignment change the measurement consistently.
    let b = TextData { bullets: true, ..t.clone() };
    let (wb, _) = m.measure(&b, &th, None);
    assert!(wb > w);
}

#[test]
fn bundled_fonts_registered() {
    let r = Renderer::new();
    for (fam, n, files) in r.font_report() {
        assert_eq!(n, files, "family {fam}");
    }
}

#[test]
fn capture_errors_and_no_state_change() {
    let r = Renderer::new();
    let ed = sample(&r, "night");
    let before = (ed.view.camera, ed.revision);
    assert!(r.capture(&ed, &CaptureTarget::Step { index: 99, build: None }, 100, 100).is_err());
    assert!(r.capture(&ed, &CaptureTarget::Element { id: "nope".into() }, 100, 100).is_err());
    assert!(r.capture(&ed, &CaptureTarget::Overview, 0, 100).is_err());
    for t in [
        CaptureTarget::Viewport,
        CaptureTarget::AgentView,
        CaptureTarget::Element { id: "f2".into() },
        CaptureTarget::Step { index: 2, build: Some(0) },
    ] {
        let png = r.capture(&ed, &t, 320, 200).unwrap();
        let img = decode(&png);
        assert_eq!(img.dimensions(), (320, 200));
    }
    assert_eq!(before, (ed.view.camera, ed.revision));
}

#[test]
fn pdf_smoke() {
    let r = Renderer::new();
    let ed = sample(&r, "paper");
    let pdf = r.export_pdf(&ed).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    std::fs::write(out_dir().join("sample.pdf"), &pdf).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    let count = |pat: &str| text.matches(pat).count();
    let pages = count("/Type /Page") - count("/Type /Pages") + count("/Type/Page") - count("/Type/Pages");
    assert_eq!(pages, ed.doc.path.len(), "one page per step");
    // Empty path: one overview page.
    let mut ed2 = sample(&r, "classic");
    ed2.doc.path.clear();
    let pdf2 = r.export_pdf(&ed2).unwrap();
    assert!(pdf2.starts_with(b"%PDF"));
}

#[test]
fn italic_rotation_and_deep_text_samples() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.doc.theme = "chalkboard".into();
    ed.view.viewport = Vec2::new(800.0, 500.0);
    let it = |s: &str, bold: bool| ElementKind::Text(TextData { text: s.into(), italic: true, bold, ..Default::default() });
    add(&mut ed, "a", 0.0, -60.0, 0.0, 0.0, 2.0, 0.0, it("Synthetic italic Caveat", false));
    add(&mut ed, "b", 0.0, 40.0, 0.0, 0.0, 2.0, 0.0, it("Bold italic Caveat", true));
    add(&mut ed, "tiny", 0.0, 140.0, 0.0, 0.0, 0.001, 30.0, text("deep zoom text stays crisp", TextStyle::Body));
    let cam = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let png = r.capture(&ed, &CaptureTarget::Camera { camera: cam }, 800, 500).unwrap();
    std::fs::write(out_dir().join("italic.png"), &png).unwrap();
    // Zoom 2000x onto the tiny text, camera rolled to match its rotation.
    let png = r.capture(&ed, &CaptureTarget::Element { id: "tiny".into() }, 800, 300).unwrap();
    std::fs::write(out_dir().join("deep_text.png"), &png).unwrap();
    let img = decode(&png);
    let bg = Theme::resolve("chalkboard").color(&Theme::resolve("chalkboard").background);
    assert!(img.pixels().filter(|p| !close(p.0, bg, 3)).count() > 2000);
    let pdf = {
        ed.doc.path = vec![PathStep { target: StepTarget::Element { id: "a".into() }, builds: vec![], notes: String::new(), duration: None, dwell: None, hide_builds: false, voiceover: None }];
        r.export_pdf(&ed).unwrap()
    };
    std::fs::write(out_dir().join("italic.pdf"), &pdf).unwrap();
}

#[test]
fn rich_text_links_hit_and_formats_draw() {
    let r = zebra_render::Renderer::new();
    let mut ed = zebra_core::Editor::default();
    ed.measure = r.measure();
    let cmd: zebra_core::Command = serde_json::from_value(serde_json::json!({"op":"add_text","id":"t","x":100,"y":50,"markdown":true,"text":"[Open the site](https://example.com) **now**"})).unwrap();
    ed.apply(cmd, zebra_core::Origin::Human).unwrap();
    let e = ed.doc.get("t").unwrap().clone();
    let (w, _h) = e.world_size();
    // Left part of the text is the link; the far right ("now") is not.
    let left = zebra_core::Vec2::new(100.0 - w / 2.0 + w * 0.15, 50.0);
    let right = zebra_core::Vec2::new(100.0 + w / 2.0 - 2.0, 50.0);
    assert_eq!(r.link_at(&ed, &e, left).as_deref(), Some("https://example.com"));
    assert_eq!(r.link_at(&ed, &e, right), None);
    // The link is underlined: the display list has a filled decoration.
    let cam = zebra_core::Camera { cx: 100.0, cy: 50.0, zoom: 1.0, rotation: 0.0 };
    let list = r.build_display_list(&ed, &cam, [800.0, 600.0], 1.0, &Default::default());
    assert!(list.items.iter().any(|i| matches!(i, zebra_render::display::Item::Fill { .. })), "underline drawn");
}

#[test]
fn ring_has_a_hole_and_icons_draw_strokes() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(400.0, 200.0);
    let th = Theme::resolve("classic");
    add(&mut ed, "ring", -100.0, 0.0, 160.0, 160.0, 1.0, 0.0, shape(ShapeKind::Ring, Some("#ff0000"), None, 0.0));
    add(
        &mut ed,
        "n",
        100.0,
        0.0,
        160.0,
        160.0,
        1.0,
        0.0,
        ElementKind::Icon(IconData { name: "minus".into(), color: "#0000ff".into(), stroke_width: 2.0 }),
    );
    let cam = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let png = r.capture(&ed, &CaptureTarget::Camera { camera: cam }, 400, 200).unwrap();
    std::fs::write(out_dir().join("ring-icon.png"), &png).unwrap();
    let img = decode(&png);
    assert!(close(px(&img, 100, 100), th.color(&th.background), 2), "ring centre is a hole: {:?}", px(&img, 100, 100));
    assert!(close(px(&img, 100, 30), [255, 0, 0, 255], 2), "ring band is filled");
    // Lucide "minus" is a horizontal stroke across the middle of the grid.
    assert!(close(px(&img, 300, 100), [0, 0, 255, 255], 8), "icon stroke: {:?}", px(&img, 300, 100));
    assert!(close(px(&img, 300, 60), th.color(&th.background), 2), "icon is a stroke, not a fill");
}

/// Text a few device pixels tall is drawn as faint bars, one per line,
/// without shaping it; big enough to read, it is text as before.
#[test]
fn tiny_text_is_drawn_as_bars() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.doc.theme = "paper".into();
    let t = TextData { text: "One line\nand another".into(), style: TextStyle::Body, ..Default::default() };
    add(&mut ed, "t", 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, ElementKind::Text(t));
    let size = Theme::resolve("paper").text_style(TextStyle::Body).size;
    let kinds = |zoom: f64| {
        let cam = Camera { cx: 0.0, cy: 0.0, zoom, rotation: 0.0 };
        let list = r.build_display_list(&ed, &cam, [800.0, 600.0], 1.0, &SceneOptions::default());
        let texts = list.items.iter().filter(|i| matches!(i, zebra_render::display::Item::Text { .. })).count();
        let fills = list.items.iter().filter(|i| matches!(i, zebra_render::display::Item::Fill { .. })).count();
        (texts, fills)
    };
    assert_eq!(kinds(1.0), (1, 0), "readable: text");
    assert_eq!(kinds(4.0 / size), (1, 0), "4 px: still text");
    assert_eq!(kinds(2.0 / size), (0, 2), "2 px: a bar per line");
}
