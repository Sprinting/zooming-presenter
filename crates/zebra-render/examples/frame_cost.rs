//! Time building the display list for a document (CPU side of a frame).
//!
//!   cargo run --release -p zebra-render --example frame_cost -- deck.zebra
use std::time::Instant;
use zebra_core::{io, Camera, Editor};
use zebra_render::{Renderer, SceneOptions};

fn main() {
    let path = std::env::args().nth(1).expect("usage: frame_cost FILE.zebra");
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.load(io::load(std::path::Path::new(&path)).unwrap(), None);
    let b = ed.doc.bounds().unwrap();
    let opts = SceneOptions { async_images: true, ..Default::default() };
    for (name, zoom) in [("overview", 1440.0 / (b.x1 - b.x0)), ("4x", 4.0 * 1440.0 / (b.x1 - b.x0)), ("one frame", 1.2)] {
        let c = b.center();
        let cam = Camera { cx: c.x, cy: c.y, zoom, rotation: 0.0 };
        let mut v = vec![];
        for i in 0..60 {
            let cam = Camera { cx: cam.cx + i as f64 * 3.0 / zoom, ..cam };
            let t = Instant::now();
            let l = r.build_display_list(&ed, &cam, [1440.0, 900.0], 2.0, &opts);
            v.push(t.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(l);
        }
        v.sort_by(f64::total_cmp);
        println!("{name:10} p50 {:.2} ms  p95 {:.2} ms", v[30], v[57]);
    }
}
