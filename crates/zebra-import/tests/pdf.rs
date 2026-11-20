//! PDF import: a deck Zebra exports as PDF comes back a picture per page,
//! drawn from the PDF kept in the document; tiles of a page match the
//! page; damaged files give an error, never a panic.
//!
//! Needs PDFium: `scripts/fetch-pdfium.sh linux-x64` (or mac-arm64) puts
//! it in target/pdfium/, or set ZEBRA_PDFIUM. Without it these tests say
//! so and pass.
use zebra_core::{Editor, Element, ElementKind, Origin, StepTarget};
use zebra_import::{Control, ImportError, ImportNote, Options};

fn engine() -> bool {
    if std::env::var_os("ZEBRA_PDFIUM").is_none() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/pdfium");
        let lib = if cfg!(target_os = "macos") { "libpdfium.dylib" } else { "libpdfium.so" };
        let found = ["mac-arm64", "mac-univ", "mac-x64", "linux-x64"].iter().map(|p| root.join(p).join("lib").join(lib)).find(|p| p.is_file());
        if let Some(p) = found {
            // SAFETY: set before any thread of this test binary reads it
            // (PDFium binds once, on first use).
            unsafe { std::env::set_var("ZEBRA_PDFIUM", p) };
        }
    }
    let ok = zebra_import::pdf::available();
    if !ok {
        eprintln!("SKIPPED: PDFium not found (run scripts/fetch-pdfium.sh linux-x64 or set ZEBRA_PDFIUM)");
    }
    ok
}

fn cmd(j: serde_json::Value) -> zebra_core::Command {
    serde_json::from_value(j).unwrap()
}

/// Three steps: a title, bullets with a bold word, shapes and a picture.
fn sample_pdf() -> Vec<u8> {
    let r = zebra_render::Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    let mut png = vec![];
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([(x * 4) as u8, (y * 5) as u8, 128])))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let key = ed.add_asset(png).unwrap();
    ed.apply_batch(
        vec![
            cmd(serde_json::json!({"op":"add_frame","id":"f1","style":"invisible","x":0,"y":0,"w":800,"h":600,"add_to_path":true})),
            cmd(serde_json::json!({"op":"add_text","id":"t1","text":"Hello Import","style":"title","x":0,"y":-150})),
            cmd(serde_json::json!({"op":"add_text","id":"t2","text":"Plain body line","x":0,"y":100})),
            cmd(serde_json::json!({"op":"add_frame","id":"f2","style":"invisible","x":2000,"y":0,"w":800,"h":600,"add_to_path":true})),
            cmd(serde_json::json!({"op":"add_text","id":"t3","text":"First point\nSecond point","x":1900,"y":0})),
            cmd(serde_json::json!({"op":"add_frame","id":"f3","style":"invisible","x":4000,"y":0,"w":800,"h":600,"add_to_path":true})),
            cmd(serde_json::json!({"op":"add_shape","shape":"rectangle","x":3800,"y":-100,"w":200,"h":120,"fill":"#3366cc"})),
            cmd(serde_json::json!({"op":"add_shape","shape":"ellipse","x":4200,"y":-100,"w":150,"h":150,"fill":"#cc3333"})),
            cmd(serde_json::json!({"op":"add_element","element":{"x":4000,"y":150,"w":256,"h":192,"kind":{"type":"image","asset":key,"alt":""}}})),
        ],
        Some("sample".into()),
        Origin::Human,
    )
    .unwrap();
    ed.apply(cmd(serde_json::json!({"op":"format_paragraphs","id":"t3","list":"bullet"})), Origin::Human).unwrap();
    r.export_pdf(&ed).unwrap()
}

fn rgba(img: &image::RgbaImage) -> &[u8] {
    img.as_raw()
}

#[test]
fn each_page_comes_in_as_a_picture_drawn_from_the_pdf() {
    if !engine() {
        return;
    }
    let pdf = sample_pdf();
    let reader = zebra_import::open_bytes(pdf.clone(), "sample".into()).unwrap();
    assert_eq!(reader.kind(), "PDF");
    assert_eq!(reader.page_count(), 3);
    let mut ed = Editor::default();
    ed.measure = zebra_render::Renderer::new().measure();
    let deck = reader.read(&Options::all(ed.theme()), &Control::default()).unwrap();
    assert_eq!(deck.pages.len(), 3);
    assert_eq!(deck.pages[0].name, "Page 1");
    // The PDF once, plus a picture per page.
    let pdfs: Vec<_> = deck.assets.iter().filter(|(_, a)| a.ext == "pdf").collect();
    assert_eq!(pdfs.len(), 1);
    let (pdf_key, pdf_asset) = pdfs[0];
    assert_eq!(&pdf_asset.bytes[..], &pdf[..]);
    for (i, p) in deck.pages.iter().enumerate() {
        assert_eq!(p.items.len(), 1);
        let it = &p.items[0];
        let ElementKind::Image(img) = &it.element.kind else { panic!("{:?}", it.element.kind) };
        let page = img.page.as_ref().expect("page ref");
        assert_eq!((&page.pdf, page.index), (pdf_key, i as u32));
        let pic = &deck.assets[&img.asset];
        assert_eq!(pic.width.max(pic.height), zebra_import::pdf::PREVIEW_PX);
        // No spare room kept in the picture's buffer.
        if let zebra_core::AssetBytes::Memory(v) = &pic.bytes {
            assert_eq!(v.len(), v.capacity());
        }
        // The picture fills the page, which is its frame's size.
        assert_eq!((it.element.size.w, it.element.size.h), (p.w, p.h));
        assert!((p.w / p.h - pic.width as f64 / pic.height as f64).abs() < 0.01);
        assert!((p.w.max(p.h) - zebra_import::pdf::PAGE_UNITS).abs() < 1.0);
    }
    // The page's first line describes it.
    let ElementKind::Image(img0) = &deck.pages[0].items[0].element.kind else { unreachable!() };
    assert_eq!(img0.alt, "Hello Import");

    // On the canvas: one frame around the three pages, a step per page
    // (the page's picture, named after it), one undo.
    let before = ed.doc.clone();
    let out = ed.import_deck(&deck, "Import sample".into(), Origin::Human).unwrap();
    assert!(!out.created.is_empty());
    let frames: Vec<&Element> = ed.doc.ordered().filter(|e| e.is_frame()).collect();
    assert_eq!(frames.len(), 1);
    assert_eq!(ed.doc.frame_contents(&frames[0].id).len(), 3);
    assert_eq!(ed.doc.path.len(), 3);
    for (i, s) in ed.doc.path.iter().enumerate() {
        let StepTarget::Element { id } = &s.target else { panic!("{s:?}") };
        let e = ed.doc.get(id).unwrap();
        assert!(matches!(e.kind, ElementKind::Image(_)));
        assert_eq!(e.name, deck.pages[i].name);
    }
    // Saved and opened again, the PDF is still there for the pages.
    let back = zebra_core::io::load_bytes(&zebra_core::io::save_bytes(&ed.doc).unwrap()).unwrap();
    assert_eq!(back.assets[pdf_key].ext, "pdf");
    assert_eq!(back.assets[pdf_key].bytes.len(), pdf.len());
    let pages = back.elements.values().filter(|e| matches!(&e.kind, ElementKind::Image(i) if i.page.is_some())).count();
    assert_eq!(pages, 3);
    ed.undo(Origin::Human).unwrap();
    // (The pictures' bytes stay in memory, unused, as after any undone
    // paste; saving leaves them out.)
    assert_eq!((&ed.doc.elements, &ed.doc.z_order, &ed.doc.path), (&before.elements, &before.z_order, &before.path));
    assert!(zebra_core::io::load_bytes(&zebra_core::io::save_bytes(&ed.doc).unwrap()).unwrap().assets.is_empty());
}

#[test]
fn tiles_match_the_whole_page_drawn_at_that_size() {
    if !engine() {
        return;
    }
    let pdf = sample_pdf();
    let bytes = zebra_core::AssetBytes::from(pdf);
    // Page 3 (shapes and a picture), 1024 wide.
    let (w, h) = (1024u32, 768u32);
    let whole = zebra_import::pdf::raster("k", &bytes, 2, (w, h), (0, 0, w, h)).expect("whole page");
    assert_eq!(whole.len(), (w * h * 4) as usize);
    let whole = image::RgbaImage::from_raw(w, h, whole).unwrap();
    // Not a blank page.
    assert!(rgba(&whole).chunks(4).any(|p| p[2] > 150 && p[0] < 100), "no blue rectangle");
    // Each 512 tile is that part of the whole page.
    for (x, y) in [(0, 0), (512, 0), (0, 512), (512, 512)] {
        let th = 512.min(h - y);
        let tile = zebra_import::pdf::raster("k", &bytes, 2, (w, h), (x, y, 512, th)).expect("tile");
        let tile = image::RgbaImage::from_raw(512, th, tile).unwrap();
        let want = image::imageops::crop_imm(&whole, x, y, 512, th).to_image();
        let diff: u64 = rgba(&tile).iter().zip(rgba(&want)).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as u64).sum();
        assert!(diff / (512 * th as u64 * 4) < 2, "tile {x},{y} differs by {diff}");
    }
    // Another PDF on the same thread, and back: each is drawn from its own.
    assert!(zebra_import::pdf::raster("other", &zebra_core::AssetBytes::from(b"%PDF-1.4 junk".to_vec()), 0, (64, 64), (0, 0, 64, 64)).is_none());
    assert!(zebra_import::pdf::raster("k", &bytes, 9, (64, 64), (0, 0, 64, 64)).is_none(), "no page 10");
    assert!(zebra_import::pdf::raster("k", &bytes, 0, (64, 64), (0, 0, 64, 64)).is_some());
}

/// `n` steps, each a title, a paragraph and a few shapes, as a PDF.
fn many_pages_pdf(n: usize) -> Vec<u8> {
    let r = zebra_render::Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    let mut cmds = vec![];
    for i in 0..n {
        let x = i as f64 * 2000.0;
        cmds.push(cmd(serde_json::json!({"op":"add_frame","style":"invisible","x":x,"y":0,"w":800,"h":600,"add_to_path":true})));
        cmds.push(cmd(serde_json::json!({"op":"add_text","text":format!("Slide number {i}"),"style":"title","x":x,"y":-200})));
        cmds.push(cmd(serde_json::json!({"op":"add_text","text":"A line of body text that runs across the slide","x":x,"y":0})));
        cmds.push(cmd(serde_json::json!({"op":"add_shape","shape":"rectangle","x":x - 200.0,"y":180,"w":200,"h":120,"fill":"#3366cc"})));
        cmds.push(cmd(serde_json::json!({"op":"add_shape","shape":"ellipse","x":x + 200.0,"y":180,"w":150,"h":150,"fill":"#cc3333"})));
    }
    ed.apply_batch(cmds, Some("pages".into()), Origin::Human).unwrap();
    r.export_pdf(&ed).unwrap()
}

#[test]
fn page_pictures_are_small_previews() {
    if !engine() {
        return;
    }
    let pdf = many_pages_pdf(50);
    let reader = zebra_import::open_bytes(pdf.clone(), "many".into()).unwrap();
    let deck = reader.read(&Options::all(Editor::default().theme()), &Control::default()).unwrap();
    assert_eq!(deck.pages.len(), 50);
    let pics: Vec<_> = deck.assets.values().filter(|a| a.ext != "pdf").collect();
    for a in &pics {
        assert!(a.width.max(a.height) <= zebra_import::pdf::PREVIEW_PX);
        if let zebra_core::AssetBytes::Memory(v) = &a.bytes {
            assert_eq!(v.len(), v.capacity());
        }
    }
    let new_total: usize = deck.assets.values().map(|a| a.bytes.len()).sum();
    // What import kept before: a 2048 px PNG per page, in a Vec that grew
    // as it was written, beside the PDF.
    let bytes = zebra_core::AssetBytes::from(pdf.clone());
    let mut old_total = pdf.len();
    for (i, a) in pics.iter().enumerate().take(50) {
        let (w, h) = if a.width >= a.height { (2048, (2048 * a.height).div_ceil(a.width)) } else { ((2048 * a.width).div_ceil(a.height), 2048) };
        let px = zebra_import::pdf::raster("old", &bytes, i as u32, (w, h), (0, 0, w, h)).unwrap();
        let rgb = image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, px).unwrap()).to_rgb8();
        let mut png = vec![];
        rgb.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        old_total += png.capacity();
    }
    eprintln!("50 pages: assets {new_total} bytes now, {old_total} before");
    assert!(new_total * 8 < old_total, "{new_total} vs {old_total}");
}

#[test]
fn pages_from_to_and_cancel() {
    if !engine() {
        return;
    }
    let reader = zebra_import::open_bytes(sample_pdf(), "sample".into()).unwrap();
    let theme = Editor::default().theme();
    let mut o = Options::all(theme);
    o.first = 2;
    o.last = 2;
    let deck = reader.read(&o, &Control::default()).unwrap();
    assert_eq!(deck.pages.len(), 1);
    assert_eq!(deck.pages[0].name, "Page 2");
    let ElementKind::Image(img) = &deck.pages[0].items[0].element.kind else { panic!() };
    assert_eq!(img.page.as_ref().unwrap().index, 1);
    // Out-of-range asks are clamped to the file.
    o.first = 0;
    o.last = 99;
    let ctl = Control::default();
    assert_eq!(reader.read(&o, &ctl).unwrap().pages.len(), 3);
    assert_eq!(ctl.done(), 3);
    let ctl = Control::default();
    ctl.cancel();
    assert_eq!(reader.read(&o, &ctl).err(), Some(ImportError::Cancelled));
}

#[test]
fn damaged_files_give_an_error_not_a_panic() {
    if !engine() {
        return;
    }
    assert_eq!(zebra_import::open_bytes(vec![], "x".into()).err(), Some(ImportError::NotSupported));
    assert_eq!(zebra_import::open_bytes(b"PK\x03\x04 not a pdf".to_vec(), "x".into()).err(), Some(ImportError::NotSupported));
    assert!(zebra_import::open_bytes(b"%PDF-1.7\n garbage".to_vec(), "x".into()).is_err());
    let good = sample_pdf();
    let theme = Editor::default().theme();
    // Cut short at many places, and with bytes flipped: errors or pages,
    // never a panic.
    for cut in [16, 100, good.len() / 4, good.len() / 2, good.len() - 200, good.len() - 10] {
        if let Ok(r) = zebra_import::open_bytes(good[..cut].to_vec(), "x".into()) {
            let _ = r.read(&Options::all(theme.clone()), &Control::default());
        }
    }
    let mut seed = 7u64;
    for _ in 0..12 {
        let mut b = good.clone();
        for _ in 0..40 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let i = (seed >> 33) as usize % b.len();
            b[i] ^= (seed >> 13) as u8;
        }
        if let Ok(r) = zebra_import::open_bytes(b, "x".into()) {
            let _ = r.read(&Options::all(theme.clone()), &Control::default());
        }
    }
    assert!(matches!(zebra_import::open(std::path::Path::new("/nonexistent/file.pdf")), Err(ImportError::Read(_))));
    assert_eq!(ImportNote::ALL.len(), 14);
}
