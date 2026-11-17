//! Write the "dense deck" of the performance budgets to a `.zebra` file:
//! 2,000 elements (frames, text, shapes) and 50 photos.
//!
//!   cargo run --release -p zebra-core --example dense_deck -- out.zebra
//!
//! Videos are left out until the media stream lands. Photos are 2400x1600
//! noise JPEGs, so they compress like real photos and decode at full size.
use zebra_core::command::Command;
use zebra_core::{io, Editor, Origin};

fn photo(seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2_654_435_761).max(1);
    let img = image::RgbImage::from_fn(2400, 1600, |x, y| {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let n = (s & 63) as u8;
        image::Rgb([(x / 10) as u8 ^ n, (y / 7) as u8 ^ n, (seed * 37) as u8 ^ n])
    });
    let mut v = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Jpeg).unwrap();
    v
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "dense.zebra".into());
    let mut ed = Editor::default();
    let h = Origin::Human;
    let mut n = 0;
    'outer: for gy in 0..20 {
        for gx in 0..25 {
            let (x, y) = (gx as f64 * 1200.0, gy as f64 * 900.0);
            let cmds = vec![
                Command::AddFrame { style: Default::default(), x, y, w: 1000.0, h: 700.0, rotation: 0.0, name: None, id: None, add_to_path: gx % 5 == 0 },
                Command::AddText { text: format!("Section {gy}.{gx}"), x, y: y - 250.0, style: zebra_core::TextStyle::Title, scale: Some(0.6), place: None, id: None },
                Command::AddText { text: "First point\nSecond point\nThird point".into(), x, y, style: zebra_core::TextStyle::Body, scale: None, place: None, id: None },
                Command::AddShape { shape: zebra_core::ShapeKind::Ellipse, x: x + 350.0, y: y + 200.0, w: 120.0, h: 120.0, rotation: 0.0, fill: None, stroke: None, id: None },
            ];
            ed.apply_batch(cmds, Some("cell".into()), h.clone()).unwrap();
            n += 4;
            if n >= 2000 {
                break 'outer;
            }
        }
    }
    use base64::Engine;
    for i in 0..50u32 {
        let data_base64 = base64::engine::general_purpose::STANDARD.encode(photo(i));
        let (x, y) = ((i % 10) as f64 * 3000.0, 20.0 * 900.0 + (i / 10) as f64 * 2200.0);
        ed.apply(Command::AddImage { data_base64, x, y, width: Some(2400.0), alt: format!("photo {i}"), id: None }, h.clone()).unwrap();
    }
    io::save(&mut ed.doc, std::path::Path::new(&out)).unwrap();
    println!("{out}: {} elements, {} steps, {} photos", ed.doc.elements.len(), ed.doc.path.len(), ed.doc.assets.len());
}
