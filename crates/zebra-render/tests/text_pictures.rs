//! Text drawn as a picture for a web page that may not carry its font:
//! a box taller than one capture is drawn smaller so all of it fits.
//! (Was: cut off at 8,192 pixels down, without a word.)
mod common;
use common::*;
use zebra_core::*;
use zebra_render::Renderer;

#[test]
fn a_tall_text_box_is_drawn_whole() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    let lines: Vec<String> = (1..=400).map(|i| format!("Line {i}")).collect();
    add(&mut ed, "t", 0.0, 0.0, 200.0, 20.0, 1.0, 0.0, text(&lines.join("\n"), TextStyle::Body));
    let e = ed.doc.get("t").unwrap().clone();
    let pic = r.rasterise_text(&ed, "t", 4096).unwrap();
    assert!(pic.height <= 8192 && pic.width <= 4096, "{}x{}", pic.width, pic.height);
    let (bw, bh) = (e.size.w + 2.0 * pic.pad, e.size.h + 2.0 * pic.pad);
    assert!(bh / bw > 8192.0 / 4096.0, "the test box should be taller than a capture at 4,096 wide");
    let ratio = pic.height as f64 / pic.width as f64;
    assert!((ratio - bh / bw).abs() / (bh / bw) < 0.02, "squashed or cut: {ratio} vs {}", bh / bw);
    // The last line is in the picture: ink in its bottom band.
    let img = image::load_from_memory(&pic.png).unwrap().to_rgba8();
    let pad_px = (pic.pad / bh * pic.height as f64) as u32;
    let band = (pic.height - pad_px - pic.height / 400)..(pic.height - pad_px);
    assert!(band.flat_map(|y| (0..pic.width).map(move |x| (x, y))).any(|(x, y)| img.get_pixel(x, y)[3] > 128), "the last line is missing");
}
