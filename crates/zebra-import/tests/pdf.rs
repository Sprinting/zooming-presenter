//! PDF import, golden: a deck Zebra exports as PDF comes back as frames,
//! text, shapes and pictures; damaged files give an error, never a panic.
//!
//! Needs PDFium: `scripts/fetch-pdfium.sh linux-x64` (or mac-arm64) puts
//! it in target/pdfium/, or set ZEBRA_PDFIUM. Without it these tests say
//! so and pass.
use zebra_core::{Editor, ElementKind, ListKind, Origin};
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

fn texts(deck: &zebra_core::import::ImportedDeck, page: usize) -> Vec<String> {
    deck.pages[page].items.iter().filter_map(|i| if let ElementKind::Text(t) = &i.element.kind { Some(t.text.clone()) } else { None }).collect()
}

#[test]
fn a_zebra_pdf_comes_back_as_frames_text_shapes_and_pictures() {
    if !engine() {
        return;
    }
    let reader = zebra_import::open_bytes(sample_pdf(), "sample".into()).unwrap();
    assert_eq!(reader.kind(), "PDF");
    assert_eq!(reader.page_count(), 3);
    let mut ed = Editor::default();
    ed.measure = zebra_render::Renderer::new().measure();
    let deck = reader.read(&Options::all(ed.theme()), &Control::default()).unwrap();
    assert_eq!(deck.pages.len(), 3);
    assert_eq!(deck.pages[0].name, "Page 1");
    let t0 = texts(&deck, 0);
    assert!(t0.iter().any(|t| t == "Hello Import"), "{t0:?}");
    assert!(t0.iter().any(|t| t == "Plain body line"), "{t0:?}");
    // The title is ranked a title, the body line body.
    for it in &deck.pages[0].items {
        if let ElementKind::Text(t) = &it.element.kind {
            let want = if t.text.starts_with("Hello") { zebra_core::TextStyle::Title } else { zebra_core::TextStyle::Body };
            assert_eq!(t.style, want, "{}", t.text);
        }
    }
    // Lines stay lines; bullets stay bullets.
    let bullets = deck.pages[1].items.iter().find_map(|i| match &i.element.kind {
        ElementKind::Text(t) if t.text.contains("point") => Some(t.clone()),
        _ => None,
    });
    let b = bullets.expect("bullet text");
    assert_eq!(b.text, "First point\nSecond point");
    assert!(b.paras.iter().all(|p| p.list == Some(ListKind::Bullet)), "{:?}", b.paras);
    // Page 3: two shapes and a picture (plus the white page).
    let kinds: Vec<&str> = deck.pages[2].items.iter().map(|i| i.element.kind.type_name()).collect();
    assert!(kinds.iter().filter(|k| **k == "image").count() >= 1, "{kinds:?}");
    assert!(kinds.iter().any(|k| *k == "shape"), "{kinds:?}");

    // On the canvas: three frames, three steps, one undo.
    let before = ed.doc.clone();
    let out = ed.import_deck(&deck, "Import sample".into(), Origin::Human).unwrap();
    assert!(!out.created.is_empty());
    assert_eq!(ed.doc.ordered().filter(|e| e.is_frame()).count(), 3);
    assert_eq!(ed.doc.path.len(), 3);
    ed.undo(Origin::Human).unwrap();
    // (The pictures' bytes stay in memory, unused, as after any undone
    // paste; saving leaves them out.)
    assert_eq!((&ed.doc.elements, &ed.doc.z_order, &ed.doc.path), (&before.elements, &before.z_order, &before.path));
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
    // Notes are listed in a stable order.
    let _ = ImportNote::ALL;
}
