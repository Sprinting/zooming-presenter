//! Theme backgrounds: a screen-fixed picture, 3D (parallax) layers, and
//! layout placeholders (faint while editing, gone when presenting).
mod common;

use common::*;
use zebra_core::*;
use zebra_render::{CaptureTarget, Renderer};

fn decode(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory(png).unwrap().to_rgba8()
}

/// A horizontal gradient: red channel = x.
fn ramp_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(256, 128, |x, _| image::Rgba([x as u8, 80, 160, 255]));
    let mut v = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
    v
}

/// Two small shapes far apart, so most of the screen shows the background.
fn deck(r: &Renderer) -> Editor {
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(400.0, 250.0);
    add(&mut ed, "a", -2000.0, 0.0, 20.0, 20.0, 1.0, 0.0, shape(ShapeKind::Rectangle, Some("#000000"), None, 0.0));
    add(&mut ed, "b", 2000.0, 0.0, 20.0, 20.0, 1.0, 0.0, shape(ShapeKind::Rectangle, Some("#000000"), None, 0.0));
    ed
}

fn theme_with(ed: &mut Editor, f: impl FnOnce(&mut Theme, String)) {
    let key = ed.add_asset(ramp_png()).unwrap();
    let mut t = Theme::resolve("classic");
    t.id = "t".into();
    f(&mut t, key);
    ed.apply(Command::SaveTheme { theme: t, apply: true }, Origin::Human).unwrap();
}

fn red_at_centre(r: &Renderer, ed: &Editor, cx: f64, zoom: f64) -> i32 {
    let cam = Camera { cx, cy: 0.0, zoom, rotation: 0.0 };
    let img = decode(&r.capture(ed, &CaptureTarget::Camera { camera: cam }, 400, 250).unwrap());
    img.get_pixel(200, 60).0[0] as i32
}

#[test]
fn background_picture_stays_fixed_to_the_screen() {
    let r = Renderer::new();
    let mut ed = deck(&r);
    theme_with(&mut ed, |t, k| t.background_image = Some(k));
    let a = red_at_centre(&r, &ed, -1000.0, 0.1);
    let b = red_at_centre(&r, &ed, 1000.0, 0.1);
    let c = red_at_centre(&r, &ed, 0.0, 2.0);
    assert!((a - b).abs() <= 2 && (a - c).abs() <= 2, "{a} {b} {c}");
    assert!((100..=156).contains(&a), "the middle of the ramp shows at the centre: {a}");
}

#[test]
fn layers_move_less_than_the_canvas_and_more_with_depth() {
    let r = Renderer::new();
    let shift = |depth: f64| {
        let mut ed = deck(&r);
        theme_with(&mut ed, |t, k| t.layers = vec![BackgroundLayer { asset: k, depth, opacity: 1.0 }]);
        // Pan right by 300 world units at the overview zoom.
        let home = Camera::fit_rect(&ed.doc.bounds().unwrap(), Vec2::new(400.0, 250.0), 0.06);
        let a = red_at_centre(&r, &ed, home.cx, home.zoom);
        let b = red_at_centre(&r, &ed, home.cx + 300.0, home.zoom);
        b - a
    };
    let (far, near) = (shift(0.0), shift(0.6));
    assert!(far.abs() <= 1, "depth 0 doesn't move: {far}");
    assert!(near > 3, "a nearer layer slides with the pan: {near}");
    let nearer = shift(1.0);
    assert!(nearer > near, "depth 1 moves most: {near} vs {nearer}");
}

#[test]
fn placeholders_are_faint_in_the_editor_and_hidden_when_presenting() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(400.0, 250.0);
    ed.apply(Command::AddLayout { layout: "title".into(), x: 0.0, y: 0.0, width: Some(1600.0), add_to_path: true }, Origin::Human).unwrap();
    let cam = Camera::fit_rect(&ed.doc.bounds().unwrap(), Vec2::new(400.0, 250.0), 0.06);
    let shot = |ed: &Editor, t: &CaptureTarget| decode(&r.capture(ed, t, 400, 250).unwrap());
    // Near-black (title colour #2b2b2b) and grey (any text) pixel counts.
    let black = |img: &image::RgbaImage| img.pixels().filter(|p| p.0[..3].iter().all(|c| *c < 90)).count();
    let grey = |img: &image::RgbaImage| img.pixels().filter(|p| p.0[0] < 235 && (p.0[0] as i32 - p.0[2] as i32).abs() < 12 && (p.0[0] as i32 - p.0[1] as i32).abs() < 12).count();
    let editing = shot(&ed, &CaptureTarget::Camera { camera: cam });
    let presenting = shot(&ed, &CaptureTarget::Step { index: 0, build: None });
    let mut real = ed.doc.clone();
    for e in real.elements.values_mut() {
        e.placeholder = false;
    }
    let mut ed2 = Editor::new(real);
    ed2.measure = r.measure();
    let content = shot(&ed2, &CaptureTarget::Camera { camera: cam });
    assert!(black(&content) > 40, "real title text is dark: {}", black(&content));
    assert!(black(&editing) < 5, "placeholder text is faint: {}", black(&editing));
    assert!(grey(&editing) > 200, "placeholder visible while editing: {}", grey(&editing));
    assert!(grey(&presenting) < 20, "placeholder hidden when presenting: {}", grey(&presenting));
}
