//! Export a deck as a web page, outside the app.
//!
//!   cargo run -p zebra-web --example export_html -- in.zebra out.html
//!   cargo run -p zebra-web --example export_html -- --bench 1000 out.html
//!
//! `--bench N` writes a generated deck of N elements (frames, titles,
//! bullets, shapes; a path through every fifth frame) for measuring.
use zebra_core::command::Command;
use zebra_core::{Editor, Origin};

fn bench(n: usize) -> Editor {
    let mut ed = Editor::default();
    let h = Origin::Human;
    let mut made = 0;
    let mut cell = 0;
    while made < n {
        let (gx, gy) = (cell % 16, cell / 16);
        let (x, y) = (gx as f64 * 1200.0, gy as f64 * 900.0);
        let cmds = vec![
            Command::AddFrame { style: Default::default(), x, y, w: 1000.0, h: 700.0, rotation: 0.0, name: None, id: None, add_to_path: cell % 5 == 0, find_space: false },
            Command::AddText { text: format!("Section {gy}.{gx}"), x, y: y - 250.0, style: zebra_core::TextStyle::Title, scale: Some(0.6), place: None, id: None, markdown: false, on: None, color: None },
            Command::AddText { text: "First point\nSecond point\nThird point".into(), x, y, style: zebra_core::TextStyle::Body, scale: None, place: None, id: None, markdown: false, on: None, color: None },
            Command::AddShape { shape: zebra_core::ShapeKind::Ellipse, x: x + 350.0, y: y + 200.0, w: 120.0, h: 120.0, rotation: 0.0, fill: None, stroke: None, id: None },
        ];
        ed.apply_batch(cmds, Some("cell".into()), h.clone()).unwrap();
        made += 4;
        cell += 1;
    }
    ed.doc.title = format!("Bench {n}");
    ed
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = zebra_render::Renderer::new();
    let (mut ed, out) = if args[0] == "--bench" {
        (bench(args[1].parse().unwrap()), args[2].clone())
    } else {
        let doc = zebra_core::io::load(std::path::Path::new(&args[0])).unwrap();
        let mut ed = Editor::default();
        ed.load(doc, None);
        (ed, args[1].clone())
    };
    ed.measure = r.measure();
    let w = zebra_web::export::export(&r, &ed, &Default::default(), std::path::Path::new(&out)).unwrap();
    println!("{} ({} bytes{})", w.page.display(), w.bytes, if w.folder { ", folder" } else { "" });
}
