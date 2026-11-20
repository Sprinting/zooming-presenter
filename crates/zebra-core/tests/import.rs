//! The part of File › Import… every format shares: placement, frames,
//! path steps, one undo step.
use zebra_core::import::{ImportedDeck, ImportedItem, ImportedPage, SourceBox};
use zebra_core::*;

fn text(s: &str, align: Align) -> Element {
    Element {
        id: "x".into(),
        transform: Transform { x: 0.0, y: 0.0, scale: 1.0, rotation: 0.0 },
        size: Size { w: 10.0, h: 10.0 },
        locked: false,
        name: String::new(),
        placeholder: false,
        group: None,
        kind: ElementKind::Text(TextData { text: s.into(), align, ..Default::default() }),
    }
}

fn deck(pages: usize) -> ImportedDeck {
    let mut d = ImportedDeck { title: "t".into(), ..Default::default() };
    for i in 0..pages {
        let mut p = ImportedPage { w: 960.0, h: 540.0, name: format!("Page {}", i + 1), notes: format!("notes {i}"), ..Default::default() };
        p.items.push(ImportedItem { element: text(&format!("Title {i}\nsecond line"), Align::Left), fit: Some(SourceBox { x0: 100.0, y0: 50.0, x1: 900.0, y1: 150.0, anchor: Default::default() }) });
        p.items.push(ImportedItem { element: text("centred", Align::Center), fit: Some(SourceBox { x0: 400.0, y0: 300.0, x1: 560.0, y1: 340.0, anchor: Default::default() }) });
        d.pages.push(p);
    }
    d
}

#[test]
fn import_is_one_undo_step_with_a_frame_and_step_per_page() {
    let mut ed = Editor::default();
    ed.apply(serde_json::from_value(serde_json::json!({"op":"add_frame","id":"f1","x":0,"y":0,"w":800,"h":600,"add_to_path":true})).unwrap(), Origin::Human).unwrap();
    let before = ed.doc.clone();
    let out = ed.import_deck(&deck(5), "Import t.pdf".into(), Origin::Human).unwrap();
    assert_eq!(ed.undo_label().as_deref(), Some("Import t.pdf (You)"));
    // One frame around the file (named after it, behind the pages), then
    // a frame per page, since these pages are several objects each.
    let all: Vec<&Element> = ed.doc.ordered().filter(|e| e.is_frame() && e.id != "f1").collect();
    assert_eq!(all.len(), 6);
    let outer = all[0];
    assert_eq!(outer.name, "t");
    assert_eq!(ed.doc.frame_contents(&outer.id).len(), 15);
    let frames = &all[1..];
    assert_eq!(out.created.len(), 16);
    assert_eq!(ed.doc.path.len(), 6);
    assert_eq!(ed.doc.path[1].notes, "notes 0");
    assert_eq!(ed.doc.path[5].notes, "notes 4");
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f.name, format!("Page {}", i + 1));
        assert!(matches!(&f.kind, ElementKind::Frame(d) if d.style == FrameStyle::Rectangle));
        // Right of the frame that was there, overlapping nothing.
        assert!(f.bounds().x0 > 400.0, "{:?}", f.bounds());
        // Each frame holds its page's two texts.
        assert_eq!(ed.doc.frame_contents(&f.id).len(), 2, "page {i}");
    }
    // Path order is page order.
    let ids: Vec<String> = frames.iter().map(|f| f.id.clone()).collect();
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(ed.doc.path[i + 1].target, StepTarget::Element { id: id.clone() });
    }
    ed.undo(Origin::Human).unwrap();
    assert_eq!(ed.doc, before);
    ed.redo(Origin::Human).unwrap();
    assert_eq!(ed.doc.path.len(), 6);
}

#[test]
fn imported_text_keeps_its_source_edge() {
    let mut ed = Editor::default();
    ed.import_deck(&deck(1), "Import".into(), Origin::Human).unwrap();
    let f = ed.doc.ordered().filter(|e| e.is_frame()).nth(1).unwrap().bounds();
    let texts: Vec<&Element> = ed.doc.ordered().filter(|e| matches!(e.kind, ElementKind::Text(_))).collect();
    let left = texts[0].bounds();
    assert!((left.x0 - (f.x0 + 100.0)).abs() < 0.01, "left edge kept: {left:?} in {f:?}");
    assert!(left.x1 <= f.x0 + 900.0 * 1.04 + 0.01);
    let mid = texts[1].bounds();
    assert!(((mid.x0 + mid.x1) / 2.0 - (f.x0 + 480.0)).abs() < 0.01, "centre kept");
    // Lines are not joined.
    assert!(matches!(&texts[0].kind, ElementKind::Text(t) if t.text.contains('\n')));
}

#[test]
fn an_empty_canvas_gets_the_pages_around_the_origin_and_empty_decks_are_refused() {
    let mut ed = Editor::default();
    assert!(ed.import_deck(&ImportedDeck::default(), "x".into(), Origin::Human).is_err());
    assert!(!ed.can_undo());
    ed.import_deck(&deck(4), "x".into(), Origin::Human).unwrap();
    let b = ed.doc.bounds().unwrap();
    assert!(b.x0 < 0.0 && b.x1 > 0.0 && b.y0 < 0.0 && b.y1 > 0.0, "{b:?}");
    assert_eq!(import::columns(4), 2);
    assert_eq!(import::columns(27), 6);
}

/// A PDF page: one picture filling it.
fn picture_deck(pages: usize) -> ImportedDeck {
    let png = {
        let mut v = vec![];
        image::RgbaImage::new(4, 3).write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
        v
    };
    let (key, asset) = import::asset_from(png).unwrap();
    let mut d = ImportedDeck { title: "slides".into(), ..Default::default() };
    d.assets.insert(key.clone(), asset);
    let (pdf, asset) = import::asset_from(b"%PDF-1.4 test".to_vec()).unwrap();
    d.assets.insert(pdf.clone(), asset);
    for i in 0..pages {
        let pic = Element {
            id: "x".into(),
            transform: Transform { x: 480.0, y: 360.0, scale: 1.0, rotation: 0.0 },
            size: Size { w: 960.0, h: 720.0 },
            locked: false,
            name: String::new(),
            placeholder: false,
            group: None,
            kind: ElementKind::Image(ImageData { asset: key.clone(), alt: String::new(), crop: None, mask: Default::default(), symbol: None, page: Some(PageRef { pdf: pdf.clone(), index: i as u32 }) }),
        };
        let mut p = ImportedPage { w: 960.0, h: 720.0, name: format!("Page {}", i + 1), notes: String::new(), ..Default::default() };
        p.items.push(ImportedItem { element: pic, fit: None });
        d.pages.push(p);
    }
    d
}

#[test]
fn picture_pages_are_their_own_steps_inside_one_frame_that_carries_them() {
    let mut ed = Editor::default();
    ed.import_deck(&picture_deck(4), "Import slides.pdf".into(), Origin::Human).unwrap();
    let frames: Vec<&Element> = ed.doc.ordered().filter(|e| e.is_frame()).collect();
    assert_eq!(frames.len(), 1, "one frame for the whole file");
    let outer = frames[0].id.clone();
    assert_eq!(frames[0].name, "slides");
    assert_eq!(ed.doc.path.len(), 4);
    let pages: Vec<String> = ed.doc.path.iter().map(|s| match &s.target {
        StepTarget::Element { id } => id.clone(),
        t => panic!("{t:?}"),
    }).collect();
    for (i, id) in pages.iter().enumerate() {
        let e = ed.doc.get(id).unwrap();
        assert!(matches!(e.kind, ElementKind::Image(_)));
        assert_eq!(e.name, format!("Page {}", i + 1));
        assert_eq!(ed.doc.parent_frame(id).as_deref(), Some(outer.as_str()));
        assert_eq!(ed.parent_frame(id).as_deref(), Some(outer.as_str()));
    }
    // Turning the frame with its contents turns the pages with it: each
    // page keeps its place relative to the frame.
    let rel = |ed: &Editor, id: &str| {
        let (f, p) = (ed.doc.get(&outer).unwrap().transform, ed.doc.get(id).unwrap().transform);
        (Vec2::new(p.x - f.x, p.y - f.y).rotate_deg(-f.rotation), p.rotation - f.rotation)
    };
    let before: Vec<_> = pages.iter().map(|p| rel(&ed, p)).collect();
    let ids = ed.expand(&[outer.clone()], true);
    assert_eq!(ids.len(), 5);
    let c = ed.doc.get(&outer).unwrap().transform;
    ed.apply(Command::Rotate { ids, degrees: 30.0, about: Some([c.x, c.y]), with_contents: false }, Origin::Human).unwrap();
    for (p, (d0, r0)) in pages.iter().zip(before) {
        let (d1, r1) = rel(&ed, p);
        assert!((d1 - d0).len() < 1e-6 && (r1 - r0).abs() < 1e-9, "{p}: {d0:?} -> {d1:?}");
    }
    // A page step fills the screen, like a slide.
    let e = ed.doc.get(&pages[0]).unwrap();
    assert!(e.is_page());
    assert_eq!(e.fit_margin(), 0.0);
}

/// Notes that name slides list them as ranges, at most 8 groups, then
/// "and N more"; each slide is counted once.
#[test]
fn notes_name_their_slides() {
    use zebra_core::import::{slide_groups, ImportNote};
    assert_eq!(slide_groups(&[]), (vec![], 0));
    assert_eq!(slide_groups(&[2, 5, 6, 7, 8, 9, 12]), (vec!["2".to_string(), "5\u{2013}9".into(), "12".into()], 0));
    let many: Vec<usize> = (1..=40).map(|i| i * 3).collect();
    let (groups, more) = slide_groups(&many);
    assert_eq!(groups.len(), 8);
    assert_eq!(more, 32);
    let (groups, more) = slide_groups(&(2..=9999).collect::<Vec<_>>());
    assert_eq!((groups, more), (vec!["2\u{2013}9999".to_string()], 0));
    let mut d = ImportedDeck::default();
    for s in [7, 3, 7, 3, 12] {
        d.note_slide(ImportNote::TextCut, s);
    }
    assert_eq!(d.slides[&ImportNote::TextCut], [3, 7, 12]);
    assert_eq!(d.notes[&ImportNote::TextCut], 3);
    assert_eq!(ImportNote::PageSkipped.english_with(2, &[2, 5]), "2 unreadable slides left out (2, 5)");
    assert_eq!(ImportNote::PageSkipped.english_with(2, &[]), "2 unreadable pages left out");
    assert_eq!(ImportNote::TextCut.english_with(1, &[4]), "1 slide with text cut to 100,000 characters (4)");
}
