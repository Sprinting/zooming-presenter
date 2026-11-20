//! Present: a step on an imported PDF page shows only that page, like a
//! slide deck; every other step keeps the canvas around it.
use zebra_core::import::{self, ImportedDeck, ImportedItem, ImportedPage};
use zebra_core::*;

fn deck(pages: usize, pdf: bool) -> ImportedDeck {
    let mut png = vec![];
    image::RgbaImage::new(4, 3).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let (key, asset) = import::asset_from(png).unwrap();
    let mut d = ImportedDeck { title: "slides".into(), ..Default::default() };
    d.assets.insert(key.clone(), asset);
    let (pdf_key, asset) = import::asset_from(b"%PDF-1.4 test".to_vec()).unwrap();
    d.assets.insert(pdf_key.clone(), asset);
    for i in 0..pages {
        let page = pdf.then(|| PageRef { pdf: pdf_key.clone(), index: i as u32 });
        let pic = Element {
            id: "x".into(),
            transform: Transform { x: 480.0, y: 360.0, scale: 1.0, rotation: 0.0 },
            size: Size { w: 960.0, h: 720.0 },
            locked: false,
            name: String::new(),
            placeholder: false,
            group: None,
            kind: ElementKind::Image(ImageData { asset: key.clone(), alt: String::new(), crop: None, mask: Default::default(), symbol: None, page }),
        };
        let mut p = ImportedPage { w: 960.0, h: 720.0, name: format!("Page {}", i + 1), ..Default::default() };
        p.items.push(ImportedItem { element: pic, fit: None });
        d.pages.push(p);
    }
    d
}

fn editor(pdf: bool) -> Editor {
    let mut ed = Editor::default();
    ed.view(ViewCommand::SetViewport { width: 1440.0, height: 900.0 }, Origin::Human).unwrap();
    ed.apply(serde_json::from_value(serde_json::json!({"op":"add_frame","id":"f1","x":-3000,"y":0,"w":800,"h":600,"add_to_path":true})).unwrap(), Origin::Human).unwrap();
    ed.import_deck(&deck(2, pdf), "Import slides.pdf".into(), Origin::Human).unwrap();
    ed
}

fn settle(ed: &mut Editor) {
    let t = ed.view.now + 10.0;
    ed.view.tick(t);
}

#[test]
fn a_pdf_page_step_shows_only_the_page() {
    let mut ed = editor(true);
    ed.view(ViewCommand::Present { step: Some(1) }, Origin::Human).unwrap();
    settle(&mut ed);
    let (corners, a) = zebra_app::present::deck_page(&ed).expect("page step hides its surroundings");
    assert_eq!(a, 1.0);
    let page = match &ed.doc.path[1].target {
        StepTarget::Element { id } => ed.doc.get(id).unwrap().clone(),
        t => panic!("{t:?}"),
    };
    assert_eq!(corners, page.corners());
    // The page fills the screen: no margin around it.
    assert_eq!(page.fit_margin(), 0.0);
}

#[test]
fn other_steps_overview_and_off_path_keep_the_canvas() {
    let mut ed = editor(true);
    ed.view(ViewCommand::Present { step: Some(0) }, Origin::Human).unwrap();
    settle(&mut ed);
    assert!(zebra_app::present::deck_page(&ed).is_none(), "a frame step shows its surroundings");
    ed.view(ViewCommand::Overview, Origin::Human).unwrap();
    settle(&mut ed);
    assert!(zebra_app::present::deck_page(&ed).is_none(), "overview");
    ed.view(ViewCommand::GotoStep { step: 2 }, Origin::Human).unwrap();
    settle(&mut ed);
    assert!(zebra_app::present::deck_page(&ed).is_some());
    let _ = ed.view(ViewCommand::Pan { dx: 50.0, dy: 0.0 }, Origin::Human);
    settle(&mut ed);
    assert!(ed.view.present.off_path);
    assert!(zebra_app::present::deck_page(&ed).is_none(), "off the path");
    // Not presenting: never.
    let mut ed = editor(true);
    ed.view.camera = ed.camera_for_step(1);
    assert!(zebra_app::present::deck_page(&ed).is_none());
}

#[test]
fn a_plain_picture_step_keeps_the_canvas() {
    let mut ed = editor(false);
    ed.view(ViewCommand::Present { step: Some(1) }, Origin::Human).unwrap();
    settle(&mut ed);
    assert!(zebra_app::present::deck_page(&ed).is_none());
}

#[test]
fn the_page_mask_fades_in_at_the_end_of_the_flight() {
    let mut ed = editor(true);
    ed.view(ViewCommand::Present { step: Some(0) }, Origin::Human).unwrap();
    settle(&mut ed);
    ed.view(ViewCommand::Next, Origin::Human).unwrap();
    let f = ed.view.flight.clone().expect("flies to the page");
    ed.view.now = f.start + f.duration * 0.5;
    assert!(zebra_app::present::deck_page(&ed).is_none(), "not yet, mid-flight");
    ed.view.now = f.start + f.duration * 0.85;
    let (_, a) = zebra_app::present::deck_page(&ed).unwrap();
    assert!(a > 0.0 && a < 1.0, "{a}");
}

/// Two PowerPoint slides: a slide of one picture is still framed, and the
/// frame (marked as a slide) is the step, shown deck-style.
#[test]
fn a_powerpoint_slide_step_shows_only_the_slide() {
    let mut d = deck(2, false);
    for p in &mut d.pages {
        p.slide = true;
    }
    let mut ed = Editor::default();
    ed.view(ViewCommand::SetViewport { width: 1440.0, height: 900.0 }, Origin::Human).unwrap();
    ed.import_deck(&d, "Import slides.pptx".into(), Origin::Human).unwrap();
    ed.view(ViewCommand::Present { step: Some(1) }, Origin::Human).unwrap();
    settle(&mut ed);
    let slide = match &ed.doc.path[1].target {
        StepTarget::Element { id } => ed.doc.get(id).unwrap().clone(),
        t => panic!("{t:?}"),
    };
    assert!(matches!(&slide.kind, ElementKind::Frame(f) if f.slide), "{:?}", slide.kind);
    assert_eq!(slide.name, "Page 2");
    assert!(slide.is_page());
    assert_eq!(slide.fit_margin(), 0.0);
    let (corners, a) = zebra_app::present::deck_page(&ed).expect("slide step hides its surroundings");
    assert_eq!(a, 1.0);
    assert_eq!(corners, slide.corners());
}
