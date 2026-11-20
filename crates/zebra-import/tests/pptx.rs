//! PowerPoint import: the fixtures (made by fixtures/gen_pptx.py with
//! python-pptx) come in as frames of Zebra's own text, shapes and pictures,
//! with a path step per shown slide, notes on the steps, and notes on what
//! came in differently; damaged files give an error, never a panic.
use std::collections::BTreeMap;

use zebra_core::import::{BoxAnchor, ImportedDeck};
use zebra_core::{Align, Editor, Element, ElementKind, ListKind, Origin, ShapeKind, StepTarget, TextData, TextStyle};
use zebra_import::{Control, ImportError, ImportNote, Options};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn editor() -> Editor {
    let mut ed = Editor::default();
    ed.measure = zebra_render::Renderer::new().measure();
    ed
}

fn read(name: &str, ed: &Editor) -> ImportedDeck {
    let r = zebra_import::open_bytes(fixture(name), name.trim_end_matches(".pptx").into()).unwrap();
    assert_eq!(r.kind(), "PowerPoint");
    r.read(&Options::all(ed.theme()), &Control::default()).unwrap()
}

fn texts(items: &[&Element]) -> Vec<String> {
    items.iter().filter_map(|e| if let ElementKind::Text(t) = &e.kind { Some(t.text.clone()) } else { None }).collect()
}

fn text<'a>(deck: &'a ImportedDeck, page: usize, starts: &str) -> &'a TextData {
    deck.pages[page]
        .items
        .iter()
        .find_map(|it| match &it.element.kind {
            ElementKind::Text(t) if t.text.starts_with(starts) => Some(t),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text {starts:?} on page {page}"))
}

/// Element kinds on a page, counted ("shape:rectangle", "text", "image").
fn kinds(deck: &ImportedDeck, page: usize) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for it in &deck.pages[page].items {
        let k = match &it.element.kind {
            ElementKind::Shape(s) => format!("shape:{:?}", s.shape),
            k => k.type_name().to_string(),
        };
        *m.entry(k).or_default() += 1;
    }
    m
}

fn count(pairs: &[(&str, usize)]) -> BTreeMap<String, usize> {
    pairs.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

#[test]
fn a_16_by_9_deck_comes_in_as_slides_of_zebra_objects() {
    let ed = editor();
    let deck = read("deck16x9.pptx", &ed);
    assert_eq!(deck.title, "deck16x9");
    // 8 slides, one hidden.
    assert_eq!(deck.pages.len(), 7);
    let names: Vec<&str> = deck.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Zebra Import Test", "Bullets", "Slide 3", "Slide 4", "Slide 5", "Chart", "Dark background"]);
    for p in &deck.pages {
        assert!(p.slide);
        assert!((p.w - 1000.0).abs() < 0.01);
        assert!((p.w / p.h - 16.0 / 9.0).abs() < 0.01, "{} x {}", p.w, p.h);
    }
    let notes: Vec<&str> = deck.pages.iter().map(|p| p.notes.as_str()).collect();
    assert_eq!(notes, ["Opening notes", "Talk about bullets", "Runs and sizes", "Pictures and shapes", "", "", ""]);
    // What came in differently: the hidden slide, the chart and table, the
    // gear, and the theme's gradient fill on python-pptx's shapes.
    let want = BTreeMap::from([(ImportNote::HiddenSlide, 1), (ImportNote::Chart, 1), (ImportNote::Table, 1), (ImportNote::Shape, 1), (ImportNote::Gradient, 7)]);
    assert_eq!(deck.notes, want);

    // Every slide has its background at the back: white, or its own colour.
    for (i, p) in deck.pages.iter().enumerate() {
        let ElementKind::Shape(bg) = &p.items[0].element.kind else { panic!("{:?}", p.items[0].element.kind) };
        assert_eq!(bg.shape, ShapeKind::Rectangle);
        assert_eq!(bg.fill.as_deref(), Some(if i == 6 { "#102040" } else { "#ffffff" }));
        assert_eq!((p.items[0].element.size.w, p.items[0].element.size.h), (p.w, p.h));
    }

    // 1. Title and subtitle: placeholders placed by their layout, centred.
    assert_eq!(kinds(&deck, 0), count(&[("shape:Rectangle", 1), ("text", 2)]));
    let title = text(&deck, 0, "Zebra Import Test");
    assert_eq!(title.align, Align::Center);
    assert_eq!(title.color.as_deref(), Some("#000000"));
    let sub = text(&deck, 0, "A deck made by python-pptx");
    assert_eq!(sub.color.as_deref(), Some("#898989"), "the theme's text colour, tinted, as RGB");
    let fit = deck.pages[0].items[1].fit.unwrap();
    assert_eq!(fit.anchor, BoxAnchor::Middle);
    assert!(fit.x0 > 0.0 && fit.x1 < 1000.0 && fit.y0 > 100.0, "{fit:?}");

    // 2. Bullets by level from the master's body style, sizes relative.
    let b = text(&deck, 1, "Level one");
    assert_eq!(b.text, "Level one\nLevel two\nLevel three\nBack to one");
    let paras: Vec<(u8, Option<ListKind>)> = b.paras.iter().map(|p| (p.indent, p.list)).collect();
    assert_eq!(paras, [(0, Some(ListKind::Bullet)), (1, Some(ListKind::Bullet)), (2, Some(ListKind::Bullet)), (0, Some(ListKind::Bullet))]);
    let sizes: Vec<(usize, usize, f64)> = b.spans.iter().map(|s| (s.start, s.end, s.format.size.unwrap())).collect();
    assert_eq!(sizes, [(10, 19, 28.0 / 32.0), (20, 31, 24.0 / 32.0)]);
    assert!(text(&deck, 1, "Bullets").paras.iter().all(|p| p.list == Some(ListKind::None)), "titles have no bullets");

    // 3. Runs: bold, italic, colours, a link, underline; a centred line.
    let r = text(&deck, 2, "Bold ");
    assert_eq!(r.text, "Bold italic red accent link under\nCentred line");
    let f = |s: usize| &r.spans.iter().find(|x| x.start == s).unwrap().format;
    assert_eq!(f(0).bold, Some(true));
    assert_eq!(f(5).italic, Some(true));
    assert_eq!(f(12).color.as_deref(), Some("#ff0000"));
    assert_eq!(f(16).color.as_deref(), Some("#4f81bd"), "accent 1 of the deck's theme");
    assert_eq!(f(23).link.as_deref(), Some("https://example.com/"));
    assert_eq!(f(27).underline, Some(true));
    assert_eq!(r.paras[1].align, Some(Align::Center));
    assert!(!r.fixed_width, "a box that doesn't wrap sizes itself");
    let big = text(&deck, 2, "Big words");
    assert!(big.bold);
    assert_eq!(big.style, TextStyle::Title, "54 pt against a 32 pt body");

    // 4. A picture and shapes; text on the rectangle, grouped with it.
    assert_eq!(
        kinds(&deck, 3),
        count(&[("image", 1), ("shape:Rectangle", 2), ("shape:RoundedRect", 1), ("shape:Ellipse", 1), ("shape:Triangle", 1), ("shape:BlockArrowRight", 1), ("shape:Arrow", 1), ("text", 1)])
    );
    let items: Vec<&Element> = deck.pages[3].items.iter().map(|i| &i.element).collect();
    let box_ = items.iter().find(|e| matches!(&e.kind, ElementKind::Shape(s) if s.fill.as_deref() == Some("#c0504d"))).expect("the accent 2 box");
    let ElementKind::Shape(s) = &box_.kind else { unreachable!() };
    assert_eq!(s.stroke.as_deref(), Some("#000000"));
    assert!((s.stroke_width - 3.0 * 12700.0 * 1000.0 / 12_192_000.0).abs() < 0.01);
    let on = items.iter().find(|e| matches!(&e.kind, ElementKind::Text(t) if t.text == "Box")).unwrap();
    assert!(box_.group.is_some() && on.group == box_.group);
    let ElementKind::Text(t) = &on.kind else { unreachable!() };
    assert_eq!(t.color.as_deref(), Some("#ffffff"), "the shape style's font colour");
    let arrow = items.iter().find(|e| matches!(&e.kind, ElementKind::Shape(s) if s.shape == ShapeKind::Arrow)).unwrap();
    // From (8.5, 3.5) to (11.5, 5.5) inches.
    assert!((arrow.transform.rotation - (2.0f64).atan2(3.0).to_degrees()).abs() < 0.1, "{}", arrow.transform.rotation);
    assert!((arrow.size.w - 13f64.sqrt() * 75.0).abs() < 1.0, "{}", arrow.size.w);
    let ElementKind::Image(img) = &items[1].kind else { panic!() };
    assert_eq!(img.alt, "image.png");
    assert_eq!(deck.assets[&img.asset].ext, "png");

    // 5. A turned group stays a group, its parts turned with it; a flipped
    // triangle is turned upside down instead.
    let items: Vec<&Element> = deck.pages[4].items.iter().map(|i| &i.element).collect();
    let grouped: Vec<&&Element> = items.iter().filter(|e| e.group.is_some()).collect();
    assert_eq!(grouped.len(), 3);
    assert!(grouped.iter().all(|e| e.group == grouped[0].group && (e.transform.rotation - 30.0).abs() < 1e-6));
    assert_eq!(texts(&items), ["Grouped"]);
    let tri = items.iter().find(|e| matches!(&e.kind, ElementKind::Shape(s) if s.shape == ShapeKind::Triangle)).unwrap();
    assert_eq!(tri.transform.rotation, 180.0);
    assert!(items.iter().any(|e| (e.transform.rotation - 45.0).abs() < 1e-6));

    // 6. The chart and the table are left out; the title stays.
    assert_eq!(kinds(&deck, 5), count(&[("shape:Rectangle", 1), ("text", 1)]));
}

#[test]
fn text_takes_the_themes_styles_by_size() {
    let ed = editor();
    let deck = read("deck16x9.pptx", &ed);
    let theme = ed.theme();
    let k = 1000.0 / 12_192_000.0 * 12_700.0;
    for (page, starts, style, pt) in [(1, "Level one", TextStyle::Body, 32.0), (0, "Zebra Import Test", TextStyle::Subtitle, 44.0), (2, "Big words", TextStyle::Title, 54.0)] {
        let it = deck.pages[page].items.iter().find(|i| matches!(&i.element.kind, ElementKind::Text(t) if t.text.starts_with(starts))).unwrap();
        let ElementKind::Text(t) = &it.element.kind else { unreachable!() };
        assert_eq!(t.style, style, "{starts}");
        // The box keeps its size on the slide through its scale.
        let size = theme.text_style(style).size * it.element.transform.scale;
        assert!((size - pt * k).abs() < 0.01, "{starts}: {size} vs {}", pt * k);
    }
}

#[test]
fn a_4_by_3_deck_brings_pictures_kept_for_a_chart_and_a_video() {
    let ed = editor();
    let deck = read("deck4x3.pptx", &ed);
    assert_eq!(deck.pages.len(), 3);
    for p in &deck.pages {
        assert!((p.w - 1000.0).abs() < 0.01 && (p.h - 750.0).abs() < 0.01, "{} x {}", p.w, p.h);
    }
    assert_eq!(deck.pages[0].name, "Chart with a picture");
    assert_eq!(deck.pages[0].notes, "Chart notes");
    // The chart PowerPoint 2016 can't be drawn here: its picture comes in.
    assert_eq!(kinds(&deck, 0), count(&[("shape:Rectangle", 1), ("text", 1), ("image", 1)]));
    // The video: its poster.
    assert_eq!(kinds(&deck, 1), count(&[("shape:Rectangle", 1), ("image", 1)]));
    assert_eq!(deck.notes, BTreeMap::from([(ImportNote::Media, 1)]));
    assert_eq!(deck.pages[2].name, "Four by three");
}

#[test]
fn on_the_canvas_slides_are_frames_with_steps_and_one_undo_removes_them() {
    let mut ed = editor();
    ed.apply(serde_json::from_value(serde_json::json!({"op":"add_frame","id":"f1","x":0,"y":0,"w":800,"h":600,"add_to_path":true})).unwrap(), Origin::Human).unwrap();
    let before = ed.doc.clone();
    let deck = read("deck16x9.pptx", &ed);
    let items: usize = deck.pages.iter().map(|p| p.items.len()).sum();
    let out = ed.import_deck(&deck, "Import deck16x9.pptx".into(), Origin::Human).unwrap();
    // A frame around everything, a frame per slide, and each slide's objects.
    assert_eq!(out.created.len(), 1 + deck.pages.len() + items);
    assert_eq!(ed.doc.path.len(), 1 + deck.pages.len());
    for (i, step) in ed.doc.path.iter().skip(1).enumerate() {
        let StepTarget::Element { id } = &step.target else { panic!("{step:?}") };
        let f = ed.doc.get(id).unwrap();
        let ElementKind::Frame(fd) = &f.kind else { panic!("{:?}", f.kind) };
        assert!(fd.slide);
        // Shown deck-style in Present, filling the screen.
        assert!(f.is_page());
        assert_eq!(f.fit_margin(), 0.0);
        assert_eq!(f.name, deck.pages[i].name);
        assert_eq!(step.notes, deck.pages[i].notes);
        assert_eq!(ed.doc.frame_contents(id).len(), deck.pages[i].items.len(), "slide {}", i + 1);
    }
    // Groups stay groups (one per slide's group, not merged across slides).
    let groups: std::collections::BTreeSet<&str> = ed.doc.elements.values().filter_map(|e| e.group.as_deref()).collect();
    assert_eq!(groups.len(), 2);
    // Every text kept its place: inside its slide.
    for e in ed.doc.elements.values().filter(|e| matches!(e.kind, ElementKind::Text(_))) {
        let slide = ed.doc.elements.values().find(|f| matches!(&f.kind, ElementKind::Frame(fd) if fd.slide) && ed.doc.frame_contents(&f.id).contains(&e.id));
        assert!(slide.is_some(), "{:?} outside its slide", e.kind);
    }
    assert_eq!(ed.undo_label().as_deref(), Some("Import deck16x9.pptx (You)"));
    ed.undo(Origin::Human).unwrap();
    assert_eq!(ed.doc.elements, before.elements);
    assert_eq!(ed.doc.path, before.path);
}

#[test]
fn a_range_of_slides_and_their_names() {
    let ed = editor();
    let r = zebra_import::open_bytes(fixture("deck16x9.pptx"), "d".into()).unwrap();
    assert_eq!(r.page_count(), 8);
    let mut o = Options::all(ed.theme());
    (o.first, o.last) = (3, 6);
    o.slide_name = std::sync::Arc::new(|n| format!("Dia {n}"));
    let deck = r.read(&o, &Control::default()).unwrap();
    let names: Vec<&str> = deck.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Dia 3", "Dia 4", "Dia 5"]);
    assert_eq!(deck.notes.get(&ImportNote::HiddenSlide), Some(&1));
    // Only the hidden slide: nothing to bring in.
    (o.first, o.last) = (6, 6);
    assert_eq!(r.read(&o, &Control::default()).unwrap_err(), ImportError::AllHidden);
    // Cancelled.
    let ctl = Control::default();
    ctl.cancel();
    assert_eq!(r.read(&Options::all(ed.theme()), &ctl).unwrap_err(), ImportError::Cancelled);
    let ctl = Control::default();
    r.read(&Options::all(ed.theme()), &ctl).unwrap();
    assert_eq!(ctl.done(), 8);
}

/// A minimal zip holding the named (stored) files.
fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = vec![];
    let mut central = vec![];
    for (name, data) in files {
        let crc = crc32(data);
        let off = out.len() as u32;
        let le16 = |v: u16| v.to_le_bytes();
        let le32 = |v: u32| v.to_le_bytes();
        out.extend(le32(0x0403_4b50));
        out.extend(le16(20));
        out.extend(le16(0));
        out.extend(le16(0));
        out.extend(le32(0));
        out.extend(le32(crc));
        out.extend(le32(data.len() as u32));
        out.extend(le32(data.len() as u32));
        out.extend(le16(name.len() as u16));
        out.extend(le16(0));
        out.extend(name.as_bytes());
        out.extend(*data);
        central.extend(le32(0x0201_4b50));
        central.extend(le16(20));
        central.extend(le16(20));
        central.extend([0; 6]);
        central.extend(le32(0));
        central.extend(le32(crc));
        central.extend(le32(data.len() as u32));
        central.extend(le32(data.len() as u32));
        central.extend(le16(name.len() as u16));
        central.extend([0; 12]);
        central.extend(le32(off));
        central.extend(name.as_bytes());
    }
    let (cd_off, cd_len) = (out.len() as u32, central.len() as u32);
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend(cd_len.to_le_bytes());
    out.extend(cd_off.to_le_bytes());
    out.extend([0; 2]);
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for b in data {
        c ^= *b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

#[test]
fn damaged_and_other_files_give_an_error_never_a_panic() {
    let ed = editor();
    let good = fixture("deck16x9.pptx");
    let err = |bytes: Vec<u8>| zebra_import::open_bytes(bytes, "x".into()).err();
    assert_eq!(err(vec![]), Some(ImportError::NotSupported));
    assert_eq!(err(b"hello".to_vec()), Some(ImportError::NotSupported));
    // A zip that isn't a presentation.
    assert_eq!(err(zip(&[("word/document.xml", b"<w:document/>")])), Some(ImportError::NotSupported));
    // A zip that says it is one but isn't.
    assert_eq!(err(zip(&[("ppt/presentation.xml", b"<p:presentation")])), Some(ImportError::Damaged("PowerPoint")));
    assert_eq!(err(zip(&[("[Content_Types].xml", b"<Types/>"), ("ppt/presentation.xml", b"\xff\xfe garbage")])), Some(ImportError::Damaged("PowerPoint")));
    // Old PowerPoint, and a .pptx with a password: both OLE files.
    let ole = |stream: &str| {
        let mut b = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        b.resize(512, 0);
        b.extend(stream.encode_utf16().flat_map(u16::to_le_bytes));
        b
    };
    assert_eq!(err(ole("PowerPoint Document")), Some(ImportError::OldPowerPoint));
    assert_eq!(err(ole("EncryptedPackage")), Some(ImportError::Encrypted));
    assert_eq!(err(ole("WordDocument")), Some(ImportError::NotSupported));
    assert!(ImportError::OldPowerPoint.to_string().contains("save it as .pptx"));
    // Every truncation of a real deck, and flipped bytes through it.
    let step = (good.len() / 150).max(1);
    for n in (0..good.len()).step_by(step) {
        if let Ok(r) = zebra_import::open_bytes(good[..n].to_vec(), "x".into()) {
            let _ = r.read(&Options::all(ed.theme()), &Control::default());
        }
        let mut bad = good.clone();
        for b in bad.iter_mut().skip(n).take(64) {
            *b ^= 0x5a;
        }
        if let Ok(r) = zebra_import::open_bytes(bad, "x".into()) {
            let _ = r.read(&Options::all(ed.theme()), &Control::default());
        }
    }
}

/// Each XML part of a deck cut short in turn, the zip itself sound: the
/// parser and the reader see broken and partial markup.
#[test]
fn broken_parts_give_an_error_or_a_partial_deck_never_a_panic() {
    use std::io::{Read, Write};
    let ed = editor();
    for name in ["deck16x9.pptx", "deck4x3.pptx"] {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(fixture(name))).unwrap();
        let mut parts = vec![];
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let mut b = vec![];
            f.read_to_end(&mut b).unwrap();
            parts.push((f.name().unwrap().to_string(), b));
        }
        for (target, bytes) in parts.iter().filter(|(n, _)| n.ends_with(".xml") || n.ends_with(".rels")) {
            for cut in [0, bytes.len() / 3, bytes.len() / 2, bytes.len().saturating_sub(7)] {
                let mut out = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
                for (n, b) in &parts {
                    out.start_file(n.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
                    out.write_all(if n == target { &b[..cut] } else { b }).unwrap();
                }
                let bytes = out.finish().unwrap().into_inner();
                match zebra_import::open_bytes(bytes, "x".into()) {
                    Ok(r) => match r.read(&Options::all(ed.theme()), &Control::default()) {
                        Ok(deck) => {
                            // Whatever came in goes on a canvas.
                            let mut ed = editor();
                            ed.import_deck(&deck, "Import".into(), Origin::Human).unwrap();
                        }
                        Err(e) => assert!(matches!(e, ImportError::AllHidden), "{target} cut at {cut}: {e:?}"),
                    },
                    Err(e) => assert!(matches!(e, ImportError::Damaged(_) | ImportError::NotSupported), "{target} cut at {cut}: {e:?}"),
                }
            }
        }
    }
}

#[test]
fn powerpoint_is_offered() {
    assert!(zebra_import::EXTENSIONS.contains(&"pptx"));
    assert!(zebra_import::pptx::is_pptx(&fixture("deck4x3.pptx")));
    assert!(!zebra_import::pptx::is_pptx(b"%PDF-1.7"));
}

/// With ZEBRA_PPTX_PNG_DIR set: draw every step of both fixtures and the
/// overview there, to look at.
#[test]
fn render_for_a_look() {
    let Some(dir) = std::env::var_os("ZEBRA_PPTX_PNG_DIR") else { return };
    for f in ["deck16x9.pptx", "deck4x3.pptx"] {
        let r = zebra_render::Renderer::new();
        let mut ed = Editor::default();
        ed.measure = r.measure();
        let deck = read(f, &ed);
        ed.import_deck(&deck, "Import".into(), Origin::Human).unwrap();
        for i in 0..ed.doc.path.len() {
            let png = r.capture(&ed, &zebra_render::CaptureTarget::Step { index: i, build: None }, 1280, 800).unwrap();
            std::fs::write(std::path::Path::new(&dir).join(format!("{f}-step{}.png", i + 1)), png).unwrap();
        }
        let png = r.capture(&ed, &zebra_render::CaptureTarget::Overview, 1600, 1000).unwrap();
        std::fs::write(std::path::Path::new(&dir).join(format!("{f}-overview.png")), png).unwrap();
    }
}
