//! Import a PDF and draw each page next to what Zebra made of it.
//!
//!   ZEBRA_PDFIUM=/path/libpdfium.so cargo run -p zebra-import --example pdf_compare -- in.pdf out_dir [pages]
//!
//! Writes out_dir/<name>.zebra, out_dir/<name>-NN.png (PDFium left, Zebra
//! right) and prints the import's notes and timing.
use std::path::Path;

use image::{GenericImage, Rgba, RgbaImage};
use pdfium_render::prelude::*;
use zebra_core::{Editor, Origin};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (input, out) = (Path::new(&a[1]), Path::new(&a[2]));
    let pages: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    std::fs::create_dir_all(out).unwrap();
    let name = input.file_stem().unwrap().to_string_lossy().chars().take(24).collect::<String>().replace(' ', "_");
    let renderer = zebra_render::Renderer::new();
    let mut ed = Editor::default();
    ed.measure = renderer.measure();
    let t0 = std::time::Instant::now();
    let reader = zebra_import::open(input).expect("open");
    let mut opts = zebra_import::Options::all(ed.theme());
    opts.last = pages;
    let deck = reader.read(&opts, &zebra_import::Control::default()).expect("read");
    let dt = t0.elapsed();
    let out_ed = ed.import_deck(&deck, "Import".into(), Origin::Human).expect("import");
    println!("{}: {} pages, {} objects, {} assets, read in {:.0} ms", name, deck.pages.len(), out_ed.created.len(), deck.assets.len(), dt.as_secs_f64() * 1000.0);
    for (n, c) in &deck.notes {
        println!("  {}", n.english(*c));
    }
    let mut kinds = std::collections::BTreeMap::new();
    for e in ed.doc.elements.values() {
        let k = match &e.kind {
            zebra_core::ElementKind::Image(i) if ed.doc.assets.get(&i.asset).is_some_and(|a| a.ext == "svg") => "svg picture".to_string(),
            k => k.type_name().to_string(),
        };
        *kinds.entry(k).or_insert(0) += 1;
    }
    println!("  {kinds:?}");
    zebra_core::io::save(&mut ed.doc, &out.join(format!("{name}.zebra"))).unwrap();
    // Side by side, 640 px a page.
    let lib = zebra_import::pdf::pdfium().unwrap();
    let doc = lib.load_pdf_from_file(input, None).unwrap();
    for i in 0..deck.pages.len() {
        let page = doc.pages().get(i as i32).unwrap();
        let left = page.render_with_config(&PdfRenderConfig::new().set_target_width(640)).unwrap().as_image().unwrap().to_rgba8();
        let png = renderer.capture(&ed, &zebra_render::CaptureTarget::Step { index: i, build: None }, 640, left.height()).unwrap();
        let right = image::load_from_memory(&png).unwrap().to_rgba8();
        let mut sheet = RgbaImage::from_pixel(1300, left.height().max(right.height()) + 20, Rgba([128, 128, 128, 255]));
        sheet.copy_from(&left, 6, 10).unwrap();
        sheet.copy_from(&right, 654, 10).unwrap();
        sheet.save(out.join(format!("{name}-{:02}.png", i + 1))).unwrap();
    }
}
