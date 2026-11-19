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
        p.items.push(ImportedItem { element: text(&format!("Title {i}\nsecond line"), Align::Left), fit: Some(SourceBox { x0: 100.0, y0: 50.0, x1: 900.0, y1: 150.0 }) });
        p.items.push(ImportedItem { element: text("centred", Align::Center), fit: Some(SourceBox { x0: 400.0, y0: 300.0, x1: 560.0, y1: 340.0 }) });
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
    let frames: Vec<&Element> = ed.doc.ordered().filter(|e| e.is_frame() && e.id != "f1").collect();
    assert_eq!(frames.len(), 5);
    assert_eq!(out.created.len(), 15);
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
    let f = ed.doc.ordered().find(|e| e.is_frame()).unwrap().bounds();
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
