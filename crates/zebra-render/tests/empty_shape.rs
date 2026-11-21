//! A shape with no fill and no line draws nothing (test drive T1: an agent's
//! `"fill": ""` left the shape the theme's colour, because the renderer
//! filled any shape that had neither).
use zebra_core::*;
use zebra_render::{CaptureTarget, Renderer};

fn centre(r: &Renderer, ed: &Editor) -> [u8; 4] {
    let cam = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let png = r.capture(ed, &CaptureTarget::Camera { camera: cam }, 200, 200).unwrap();
    image::load_from_memory(&png).unwrap().to_rgba8().get_pixel(100, 100).0
}

#[test]
fn a_shape_with_no_fill_and_no_line_draws_nothing() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    let empty = centre(&r, &ed);
    let add = |ed: &mut Editor, v: serde_json::Value| ed.apply(serde_json::from_value(v).unwrap(), Origin::agent("t")).unwrap();
    add(&mut ed, serde_json::json!({"op":"add_shape","id":"s","shape":"rectangle","fill":"","x":0,"y":0,"w":400,"h":400}));
    let ElementKind::Shape(s) = &ed.doc.elements["s"].kind else { panic!() };
    assert_eq!((s.fill.as_ref(), s.stroke.as_ref()), (None, None));
    assert_eq!(centre(&r, &ed), empty, "nothing drawn");
    // A plain new shape still gets the theme's colour when it's added.
    add(&mut ed, serde_json::json!({"op":"add_shape","id":"t","shape":"rectangle","x":0,"y":0,"w":400,"h":400}));
    assert_ne!(centre(&r, &ed), empty);
}
