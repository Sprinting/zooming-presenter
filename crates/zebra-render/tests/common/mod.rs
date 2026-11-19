//! Shared sample documents for render tests.
#![allow(dead_code, clippy::too_many_arguments)]

use zebra_core::*;
use zebra_render::Renderer;

pub fn out_dir() -> std::path::PathBuf {
    let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/render");
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn add(ed: &mut Editor, id: &str, x: f64, y: f64, w: f64, h: f64, scale: f64, rot: f64, kind: ElementKind) {
    let mut e = Element {
        id: id.into(),
        transform: Transform { x, y, scale, rotation: rot },
        size: Size { w, h },
        locked: false,
        name: String::new(),
        placeholder: false,
        group: None,
        kind,
    };
    let theme = ed.theme();
    zebra_core::editor::measure_text(ed.measure.as_ref(), &theme, &mut e);
    ed.doc.elements.insert(id.into(), e);
    ed.doc.z_order.push(id.into());
}

pub fn text(s: &str, style: TextStyle) -> ElementKind {
    ElementKind::Text(TextData { text: s.into(), style, ..Default::default() })
}

pub fn frame(style: FrameStyle) -> ElementKind {
    ElementKind::Frame(FrameData { style, color: None })
}

pub fn shape(shape: ShapeKind, fill: Option<&str>, stroke: Option<&str>, sw: f64) -> ElementKind {
    ElementKind::Shape(ShapeData {
        shape,
        fill: fill.map(Into::into),
        stroke: stroke.map(Into::into),
        stroke_width: sw,
        ..Default::default()
    })
}

/// A checkerboard PNG, red/blue, 64x64.
pub fn checker_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(64, 64, |x, y| {
        if (x / 16 + y / 16) % 2 == 0 { image::Rgba([220, 40, 40, 255]) } else { image::Rgba([40, 40, 220, 255]) }
    });
    let mut v = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
    v
}

/// A small Prezi-like presentation exercising every element kind.
pub fn sample(r: &Renderer, theme: &str) -> Editor {
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.doc.theme = theme.into();
    ed.view.viewport = Vec2::new(1280.0, 800.0);

    // Big overview frame with a title.
    add(&mut ed, "f1", 0.0, 0.0, 1600.0, 1000.0, 1.0, 0.0, frame(FrameStyle::Bracket));
    add(&mut ed, "t1", 0.0, -340.0, 0.0, 0.0, 1.0, 0.0, text("Zebra: zoom into ideas", TextStyle::Title));
    let sub = TextData { text: "A classic-Prezi style canvas".into(), style: TextStyle::Subtitle, italic: true, ..Default::default() };
    add(&mut ed, "t2", 0.0, -250.0, 0.0, 0.0, 1.0, 0.0, ElementKind::Text(sub));
    // Circle frame with a rotated child.
    add(&mut ed, "f2", -450.0, 120.0, 420.0, 420.0, 1.0, 0.0, frame(FrameStyle::Circle));
    add(&mut ed, "t3", -450.0, 120.0, 0.0, 0.0, 1.0, -12.0, text("Rotated!", TextStyle::Subtitle));
    // Rectangle frame with bullets.
    add(&mut ed, "f3", 120.0, 150.0, 560.0, 380.0, 1.0, 0.0, frame(FrameStyle::Rectangle));
    let mut bullets = TextData {
        text: "Infinite canvas\nZoom and rotate transitions, with a long line that wraps nicely\nAgents and humans together".into(),
        bullets: true,
        fixed_width: true,
        ..Default::default()
    };
    bullets.style = TextStyle::Body;
    add(&mut ed, "t4", 120.0, 150.0, 480.0, 0.0, 1.0, 0.0, ElementKind::Text(bullets));
    // Invisible frame.
    add(&mut ed, "f4", 620.0, 150.0, 240.0, 300.0, 1.0, 0.0, frame(FrameStyle::Invisible));
    // Shapes.
    add(&mut ed, "s1", 560.0, 60.0, 120.0, 80.0, 1.0, 0.0, shape(ShapeKind::Rectangle, Some("theme:shape1"), None, 4.0));
    add(&mut ed, "s2", 680.0, 60.0, 90.0, 90.0, 1.0, 0.0, shape(ShapeKind::Ellipse, Some("theme:shape2"), Some("theme:title"), 4.0));
    add(&mut ed, "s3", 560.0, 200.0, 100.0, 90.0, 1.0, 0.0, shape(ShapeKind::Triangle, Some("theme:shape3"), None, 4.0));
    add(&mut ed, "s4", 640.0, 300.0, 200.0, 40.0, 1.0, 0.0, shape(ShapeKind::Arrow, None, Some("theme:accent"), 8.0));
    add(&mut ed, "s5", 640.0, 360.0, 200.0, 10.0, 1.0, 20.0, shape(ShapeKind::Line, None, Some("theme:shape4"), 6.0));
    // Ink and highlighter.
    let pts: Vec<Vec2> = (0..40).map(|i| {
        let t = i as f64 / 39.0;
        Vec2::new(-150.0 + 300.0 * t, (t * 12.0).sin() * 40.0)
    }).collect();
    add(&mut ed, "k1", -450.0, 400.0, 300.0, 100.0, 1.0, 0.0, ElementKind::Ink(InkData { points: pts.clone(), color: "theme:accent".into(), width: 6.0, highlighter: false }));
    add(&mut ed, "k2", -450.0, -200.0, 300.0, 100.0, 1.0, 0.0, ElementKind::Ink(InkData { points: pts, color: "#ffd400".into(), width: 30.0, highlighter: true }));
    // Image.
    let key = ed.add_asset(checker_png()).unwrap();
    add(&mut ed, "i1", 450.0, -120.0, 160.0, 120.0, 1.0, 8.0, ElementKind::Image(ImageData { asset: key, alt: "checker".into(), crop: None, mask: Default::default(), symbol: None, page: None }));
    // Path: overview, circle, rectangle frame.
    ed.doc.path = vec![
        PathStep { target: StepTarget::Element { id: "f1".into() }, builds: vec![], notes: String::new(), duration: None, dwell: None, hide_builds: false, voiceover: None },
        PathStep { target: StepTarget::Element { id: "f2".into() }, builds: vec![], notes: String::new(), duration: None, dwell: None, hide_builds: false, voiceover: None },
        PathStep { target: StepTarget::Element { id: "f3".into() }, builds: vec!["t4".into()], notes: String::new(), duration: None, dwell: None, hide_builds: false, voiceover: None },
    ];
    ed.view.camera = ed.fit_all_camera();
    ed
}
