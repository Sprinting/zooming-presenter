//! PowerPoint files that are huge, broken or hostile (the 20 Nov audit):
//! each comes in as far as it can, in bounded memory and time, and says
//! what it left out. Own test binary: heap is measured per process, so the
//! tests take turns.
//!
//! The decks are made here from the fixtures, as the audit made its own.
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zebra_core::import::{ImportedDeck, MAX_IMPORT_SPANS};
use zebra_core::rich::MAX_INSERT_CHARS;
use zebra_core::{Editor, ElementKind, Origin, TextData};
use zebra_import::{Control, ImportError, ImportNote, Options};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
const MB: u64 = 1 << 20;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

/// Anonymous memory (heap) of this process, bytes (Linux; 0 elsewhere).
fn heap() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines().find(|l| l.starts_with("RssAnon:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * 1024
}

/// Run `f`, and the most the heap grew while it ran (sampled every 2 ms).
fn peak<T>(f: impl FnOnce() -> T) -> (T, u64) {
    let base = heap();
    let top = Arc::new(AtomicU64::new(base));
    let stop = Arc::new(AtomicBool::new(false));
    let (t, s) = (top.clone(), stop.clone());
    let sampler = std::thread::spawn(move || {
        while !s.load(Ordering::Relaxed) {
            t.fetch_max(heap(), Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    let out = f();
    top.fetch_max(heap(), Ordering::Relaxed);
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    (out, top.load(Ordering::Relaxed).saturating_sub(base))
}

/// A zip of `parts`, deflated.
fn pack(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in parts {
        z.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn unpack(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut b = vec![];
            f.read_to_end(&mut b).unwrap();
            (f.name().unwrap().to_string(), b)
        })
        .collect()
}

/// `parts` with `set` added or replaced and `remove` taken out.
fn patch(parts: &[(String, Vec<u8>)], set: &[(&str, Vec<u8>)], remove: &[&str]) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = parts.iter().filter(|(n, _)| !remove.contains(&n.as_str()) && !set.iter().any(|(s, _)| s == n)).cloned().collect();
    out.extend(set.iter().map(|(n, b)| (n.to_string(), b.clone())));
    out
}

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A slide holding `shapes` (spTree children).
fn slide(shapes: &str) -> String {
    format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld {NS}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{shapes}</p:spTree></p:cSld></p:sld>"#)
}

/// A text box `id` holding `paras` (`a:p` elements).
fn text_box(id: u32, paras: &str) -> String {
    format!(r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="T{id}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="100000" y="100000"/><a:ext cx="6000000" cy="1000000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{paras}</p:txBody></p:sp>"#)
}

fn words(id: u32, text: &str) -> String {
    text_box(id, &format!("<a:p><a:r><a:rPr lang=\"en-GB\" sz=\"2000\"/><a:t>{text}</a:t></a:r></a:p>"))
}

/// One slide of a made deck: its XML (None: the part is missing) and more
/// relationships for it.
struct S {
    xml: Option<Vec<u8>>,
    rels: String,
}

fn s(xml: impl Into<Vec<u8>>) -> S {
    S { xml: Some(xml.into()), rels: String::new() }
}

/// A deck of `slides` on deck16x9's master, layouts and theme, with
/// `extra` parts.
fn deck(slides: &[S], extra: &[(&str, Vec<u8>)], remove: &[&str]) -> Vec<u8> {
    let base = unpack(&fixture("deck16x9.pptx"));
    let mut parts: Vec<(String, Vec<u8>)> = base
        .into_iter()
        .filter(|(n, _)| !["ppt/slides/", "ppt/notesSlides/", "ppt/charts/", "ppt/embeddings/", "ppt/media/"].iter().any(|p| n.starts_with(p)))
        .filter(|(n, _)| n != "ppt/presentation.xml" && n != "ppt/_rels/presentation.xml.rels")
        .collect();
    let pres = String::from_utf8(fixture_part("deck16x9.pptx", "ppt/presentation.xml")).unwrap();
    let (a, rest) = pres.split_once("<p:sldIdLst>").unwrap();
    let (_, b) = rest.split_once("</p:sldIdLst>").unwrap();
    let ids: String = (0..slides.len()).map(|i| format!("<p:sldId id=\"{}\" r:id=\"rIdS{i}\"/>", 256 + i)).collect();
    parts.push(("ppt/presentation.xml".into(), format!("{a}<p:sldIdLst>{ids}</p:sldIdLst>{b}").into_bytes()));
    let rels = String::from_utf8(fixture_part("deck16x9.pptx", "ppt/_rels/presentation.xml.rels")).unwrap();
    let kept: String = rels.split("<Relationship ").skip(1).map(|r| r.trim_end_matches("</Relationships>")).filter(|r| !r.contains("/slide\"")).map(|r| format!("<Relationship {r}")).collect();
    let slide_rels: String = (0..slides.len()).map(|i| format!("<Relationship Id=\"rIdS{i}\" Type=\"{REL}/slide\" Target=\"slides/slide{}.xml\"/>", i + 1)).collect();
    parts.push(("ppt/_rels/presentation.xml.rels".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{kept}{slide_rels}</Relationships>").into_bytes()));
    for (i, sl) in slides.iter().enumerate() {
        if let Some(x) = &sl.xml {
            parts.push((format!("ppt/slides/slide{}.xml", i + 1), x.clone()));
        }
        parts.push((
            format!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
            {
                // Layout 7 unless the slide names its own.
                let layout = if sl.rels.contains("/slideLayout") { String::new() } else { format!("<Relationship Id=\"rIdL\" Type=\"{REL}/slideLayout\" Target=\"../slideLayouts/slideLayout7.xml\"/>") };
                format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{layout}{}</Relationships>", sl.rels).into_bytes()
            },
        ));
    }
    pack(&patch(&parts, extra, remove))
}

fn fixture_part(name: &str, part: &str) -> Vec<u8> {
    unpack(&fixture(name)).into_iter().find(|(n, _)| n == part).unwrap().1
}

fn read_with(bytes: Vec<u8>, opts: &Options) -> Result<ImportedDeck, ImportError> {
    zebra_import::open_bytes(bytes, "x".into())?.read(opts, &Control::default())
}

fn read(bytes: Vec<u8>) -> Result<ImportedDeck, ImportError> {
    read_with(bytes, &Options::all(Editor::default().theme()))
}

fn texts(deck: &ImportedDeck) -> Vec<&TextData> {
    deck.pages.iter().flat_map(|p| &p.items).filter_map(|i| if let ElementKind::Text(t) = &i.element.kind { Some(t) } else { None }).collect()
}

/// The deck goes on a canvas (text measured as the app measures it).
fn import(deck: &ImportedDeck) {
    let mut ed = Editor::default();
    ed.measure = zebra_render::Renderer::new().measure();
    ed.import_deck(deck, "Import".into(), Origin::Human).unwrap();
}

/// T-C1: a SmartArt drawing whose graphic frame names the drawing itself
/// overflowed the stack (and so did two drawings naming each other).
#[test]
fn smartart_drawings_that_name_themselves_are_read_once() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let frame = |uri_ns: &str| format!(r#"<dsp:graphicFrame><dsp:nvGraphicFramePr><dsp:cNvPr id="4" name="Diagram 1"/><dsp:cNvGraphicFramePr/><dsp:nvPr/></dsp:nvGraphicFramePr><dsp:xfrm><a:off x="914400" y="914400"/><a:ext cx="7315200" cy="3657600"/></dsp:xfrm><a:graphic><a:graphicData uri="{uri_ns}"><dgm:relIds xmlns:dgm="{uri_ns}" r:dm="rId2" r:lo="rId3"/></a:graphicData></a:graphic></dsp:graphicFrame>"#);
    let drawing = |inner: &str| {
        format!(r#"<dsp:drawing xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><dsp:spTree><dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/><dsp:sp modelId="{{A}}"><dsp:nvSpPr><dsp:cNvPr id="5" name=""/><dsp:cNvSpPr/></dsp:nvSpPr><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100000" cy="100000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="999999"/></a:solidFill></dsp:spPr></dsp:sp>{inner}</dsp:spTree></dsp:drawing>"#)
            .into_bytes()
    };
    let rels = |to: &str| format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="{REL}/diagramData" Target="{to}"/><Relationship Id="rId4" Type="http://schemas.microsoft.com/office/2007/relationships/diagramDrawing" Target="{to}"/></Relationships>"#).into_bytes();
    let dgm = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
    let on_slide = format!(r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Diagram 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="914400" y="914400"/><a:ext cx="7315200" cy="3657600"/></p:xfrm><a:graphic><a:graphicData uri="{dgm}"><dgm:relIds xmlns:dgm="{dgm}" r:dm="rId2" r:lo="rId3"/></a:graphicData></a:graphic></p:graphicFrame>"#);
    let slide_rels = format!(r#"<Relationship Id="rId2" Type="{REL}/diagramData" Target="../diagrams/drawing1.xml"/><Relationship Id="rId4" Type="http://schemas.microsoft.com/office/2007/relationships/diagramDrawing" Target="../diagrams/drawing1.xml"/>"#);
    let one = S { xml: Some(slide(&on_slide).into_bytes()), rels: slide_rels.clone() };
    let after = s(slide(&words(2, "After")));
    // The audit's case (the frame's namespace typo kept), the drawing naming
    // itself with the real namespace, and two drawings naming each other.
    for (d1, r1, extra) in [
        (drawing(&frame("httdsp://schemas.openxmlformats.org/drawingml/2006/diagram")), rels("drawing1.xml"), None),
        (drawing(&frame(dgm)), rels("drawing1.xml"), None),
        (drawing(&frame(dgm)), rels("drawing2.xml"), Some((drawing(&frame(dgm)), rels("drawing1.xml")))),
    ] {
        let mut parts = vec![("ppt/diagrams/drawing1.xml", d1), ("ppt/diagrams/_rels/drawing1.xml.rels", r1)];
        if let Some((d2, r2)) = extra {
            parts.push(("ppt/diagrams/drawing2.xml", d2));
            parts.push(("ppt/diagrams/_rels/drawing2.xml.rels", r2));
        }
        let one = S { xml: one.xml.clone(), rels: one.rels.clone() };
        let after = S { xml: after.xml.clone(), rels: String::new() };
        let d = read(deck(&[one, after], &parts, &[])).unwrap();
        assert_eq!(d.pages.len(), 2);
        assert_eq!(texts(&d).last().map(|t| t.text.as_str()), Some("After"));
        import(&d);
    }
}

/// R1: a run's link comes in only for http, https and mailto; any other
/// (`javascript:` reached the web player) leaves its words plain.
#[test]
fn only_web_and_mail_links_come_in() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    use zebra_import::pptx::web_link;
    for bad in ["javascript:alert(1)", "JAVASCRIPT:alert(1)", " javascript:alert(1)", "java\tscript:alert(1)", "java\nscript:x", "\u{0}javascript:x", "file:///etc/passwd", "data:text/html,<b>", "vbscript:x", "http:x", "https://", "mailto:", "slide2.xml", "", "  "] {
        assert_eq!(web_link(bad), None, "{bad:?}");
    }
    assert_eq!(web_link("HTTPS://Example.com/A?b=1").as_deref(), Some("https://Example.com/A?b=1"));
    assert_eq!(web_link(" http://e.x/ ").as_deref(), Some("http://e.x/"));
    assert_eq!(web_link("MailTo:a@b.c").as_deref(), Some("mailto:a@b.c"));
    // End to end: the fixture's link, then the same run linking elsewhere.
    let parts = unpack(&fixture("deck16x9.pptx"));
    let rels = String::from_utf8(fixture_part("deck16x9.pptx", "ppt/slides/_rels/slide3.xml.rels")).unwrap();
    for (target, want) in [("https://example.com/", Some("https://example.com/")), ("javascript:alert(1)", None), ("HTTP://example.com/", Some("http://example.com/")), ("file:///etc/passwd", None)] {
        let r = rels.replace("https://example.com/", &target.replace('&', "&amp;"));
        let d = read(pack(&patch(&parts, &[("ppt/slides/_rels/slide3.xml.rels", r.into_bytes())], &[]))).unwrap();
        let t = texts(&d).into_iter().find(|t| t.text.starts_with("Bold italic")).unwrap();
        assert!(t.text.contains("link"), "the words stay");
        let links: Vec<Option<&str>> = t.spans.iter().filter_map(|s| s.format.link.as_deref()).map(Some).collect();
        assert_eq!(links.first().copied().flatten(), want, "{target}");
    }
}

/// T-W3: a missing, broken or hostile part used to make the whole file
/// "damaged". Now the slide it belongs to is left out and named, or a
/// design part is left out and noted, and the rest comes in.
#[test]
fn a_broken_slide_is_left_out_and_the_rest_comes_in() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let ok = |t: &str| s(slide(&words(2, t)));
    let mut outside = ok("Five");
    outside.rels = format!(r#"<Relationship Id="rId8" Type="{REL}/image" Target="../../../../../../etc/passwd"/>"#);
    let mut no_layout = ok("Six");
    no_layout.rels = format!(r#"<Relationship Id="rIdL" Type="{REL}/slideLayout" Target="../slideLayouts/slideLayout99.xml"/>"#);
    let slides = vec![
        ok("One"),
        S { xml: None, rels: String::new() },
        ok("Three"),
        s(b"<p:sld><p:cSld><p:spTree>".to_vec()),
        outside,
        no_layout,
        // Over BetterOffice's 4 MB attribute limit, well formed: only the
        // parser finds it, and the batch is halved to find it.
        s(slide(&format!(r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="{}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/></p:sp>"#, "x".repeat(5 << 20)))),
        // Over the 8 MB a slide may be.
        s(slide(&words(2, &"y".repeat(9 << 20)))),
        ok("Nine"),
    ];
    let bytes = deck(&slides, &[], &[]);
    let d = read(bytes.clone()).unwrap();
    let got: Vec<&str> = texts(&d).iter().map(|t| t.text.as_str()).collect();
    assert_eq!(got, ["One", "Three", "Five", "Six", "Nine"]);
    assert_eq!(d.slides[&ImportNote::PageSkipped], [2, 4, 7, 8]);
    assert_eq!(d.notes[&ImportNote::PageSkipped], 4);
    // The missing layout of slide 6.
    assert_eq!(d.notes.get(&ImportNote::Design), Some(&1));
    let names: Vec<&str> = d.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Slide 1", "Slide 3", "Slide 5", "Slide 6", "Slide 9"]);
    import(&d);
    // A range reads only what it asks for.
    let mut opts = Options::all(Editor::default().theme());
    (opts.first, opts.last) = (3, 5);
    let d = read_with(bytes, &opts).unwrap();
    assert_eq!(d.pages.len(), 2);
    assert_eq!(d.slides[&ImportNote::PageSkipped], [4]);

    // A missing theme or master, or a broken layout: the slides come in
    // without that design, noted once.
    for (set, remove) in [
        (vec![], vec!["ppt/theme/theme1.xml"]),
        (vec![], vec!["ppt/slideMasters/slideMaster1.xml"]),
        (vec![("ppt/slideLayouts/slideLayout7.xml", b"<p:sldLayout>".to_vec())], vec![]),
    ] {
        let d = read(deck(&[ok("A"), ok("B")], &set, &remove)).unwrap();
        assert_eq!(texts(&d).iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["A", "B"], "{remove:?}");
        assert_eq!(d.notes.get(&ImportNote::Design), Some(&1), "{remove:?}");
        assert!(!d.notes.contains_key(&ImportNote::PageSkipped));
    }
    // Nothing readable: damaged, at open when no slide part is there.
    let broken = s(b"<oops".to_vec());
    assert_eq!(read(deck(&[s(b"<a>".to_vec()), broken], &[], &[])).err(), Some(ImportError::Damaged("PowerPoint")));
    assert_eq!(zebra_import::open_bytes(deck(&[S { xml: None, rels: String::new() }], &[], &[]), "x".into()).err(), Some(ImportError::Damaged("PowerPoint")));
    assert_eq!(zebra_import::open_bytes(deck(&[ok("A")], &[], &["ppt/_rels/presentation.xml.rels"]), "x".into()).err(), Some(ImportError::Damaged("PowerPoint")));
    // Content types broken or missing: a default is used.
    let d = read(deck(&[ok("A")], &[("[Content_Types].xml", b"<Types".to_vec())], &[])).unwrap();
    assert_eq!(d.pages.len(), 1);
    let d = read(deck(&[ok("A")], &[], &["[Content_Types].xml"])).unwrap();
    assert_eq!(d.pages.len(), 1);
}

/// A slide of alike runs packs far tighter than a picture may (20,000 runs
/// of one letter, 266 to 1, were cut short and the slide left out): a
/// part's own size cap is its bound, not its packing ratio.
#[test]
fn a_tightly_packed_slide_comes_in() {
    let run = r#"<a:r><a:rPr lang="en-GB" sz="1800"/><a:t>x</a:t></a:r>"#;
    let one = s(slide(&text_box(2, &format!("<a:p>{}</a:p>", run.repeat(40_000)))));
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in unpack(&deck(&[one], &[], &[])) {
        z.start_file(name.as_str(), zip::write::SimpleFileOptions::default().compression_level(Some(9))).unwrap();
        z.write_all(&bytes).unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let f = zip.by_name("ppt/slides/slide1.xml").unwrap();
    let ratio = f.size() / f.compressed_size().max(1);
    drop(f);
    assert!(ratio > 200, "packs only {ratio} to 1");
    let d = read(bytes).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert!(d.slides.is_empty(), "{:?}", d.slides);
    assert_eq!(texts(&d)[0].text.chars().count(), 40_000);
}

/// From the cloud pass: a slide over a limit (shapes100000.pptx) said the
/// file was damaged. Left out with the others, it is named like any slide
/// left out; when nothing else comes in, the error says it is too big.
#[test]
fn a_slide_over_a_limit_is_too_big_not_damaged() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let ok = |t: &str| s(slide(&words(2, t)));
    // Over 8 MB of XML.
    let huge = || s(slide(&format!("<!-- {} -->", "x".repeat(9 << 20))));
    // Under 8 MB but more shapes than the parser reads (100,000).
    let crowded = || s(slide(&"<p:sp/>".repeat(100_001)));
    for (name, big) in [("XML", huge as fn() -> S), ("shapes", crowded)] {
        assert_eq!(read(deck(&[big()], &[], &[])).err(), Some(ImportError::TooBig), "{name}");
        let d = read(deck(&[ok("A"), big(), ok("C")], &[], &[])).unwrap();
        assert_eq!(d.slides[&ImportNote::PageSkipped], [2], "{name}");
        assert_eq!(texts(&d).len(), 2, "{name}");
    }
    // Too big and broken together: damaged.
    assert_eq!(read(deck(&[huge(), s(b"<oops".to_vec())], &[], &[])).err(), Some(ImportError::Damaged("PowerPoint")));
}

/// T-W3: 10,000 slides were "damaged" (BetterOffice reads at most 5000
/// zip entries, and its budgets were the whole deck's). They come in, a
/// batch at a time, the heap not growing with the deck.
#[test]
fn ten_thousand_slides_come_in() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let n = 10_000;
    let slides: Vec<S> = (0..n).map(|i| s(slide(&words(2, &format!("Slide text {}", i + 1))))).collect();
    let bytes = deck(&slides, &[], &[]);
    drop(slides);
    let r = zebra_import::open_bytes(bytes.clone(), "x".into()).unwrap();
    assert_eq!(r.page_count(), n);
    let t = Instant::now();
    let ctl = Control::default();
    let (d, grew) = peak(|| r.read(&Options::all(Editor::default().theme()), &ctl).unwrap());
    eprintln!("10,000 slides: read in {:.1} s, heap +{} MB", t.elapsed().as_secs_f64(), grew / MB);
    assert_eq!(d.pages.len(), n);
    assert_eq!(ctl.done(), n);
    assert_eq!(texts(&d).last().map(|t| t.text.as_str()), Some("Slide text 10000"));
    assert!(d.notes.is_empty(), "{:?}", d.notes);
    let t = Instant::now();
    import(&d);
    eprintln!("10,000 slides: on the canvas in {:.1} s", t.elapsed().as_secs_f64());
    // Cancel stops between slides.
    let ctl = Control::default();
    ctl.cancel();
    assert_eq!(r.read(&Options::all(Editor::default().theme()), &ctl).err(), Some(ImportError::Cancelled));
}

/// T-C5: a box with more runs than text layout can take (65,536 styles
/// crashed parley) keeps 60,000 spans and says so; a box longer than paste
/// allows is cut there and says so. All-bold runs were quadratic to read.
#[test]
fn long_text_is_cut_and_its_formatting_simplified() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let run = |b: bool, t: &str| format!(r#"<a:r><a:rPr b="{}"/><a:t>{t}</a:t></a:r>"#, b as u8);
    // Bold, italic, underlined, plain, again: 75,000 spans in 100,000
    // characters (no neighbours alike to merge).
    let styles = [r#"b="1""#, r#"i="1""#, r#"u="sng""#, ""];
    let alternating: String = (0..100_000).map(|i| format!(r#"<a:r><a:rPr {}/><a:t>a</a:t></a:r>"#, styles[i % 4])).collect();
    let bold: String = (0..150_000).map(|_| run(true, "abc")).collect();
    assert!(alternating.len() < 8 << 20 && bold.len() < 8 << 20);
    let slides = [s(slide(&words(3, "First"))), s(slide(&text_box(2, &format!("<a:p>{alternating}</a:p>")))), s(slide(&text_box(2, &format!("<a:p>{bold}</a:p>"))))];
    let t = Instant::now();
    let (d, grew) = peak(|| read(deck(&slides, &[], &[])).unwrap());
    eprintln!("100,000 + 150,000 runs: read in {:.1} s, heap +{} MB", t.elapsed().as_secs_f64(), grew / MB);
    assert!(t.elapsed() < Duration::from_secs(60), "{:?}", t.elapsed());
    let tx = texts(&d);
    assert_eq!(tx[1].spans.len(), MAX_IMPORT_SPANS);
    assert_eq!(tx[1].text.chars().count(), 100_000);
    assert_eq!(tx[2].text.chars().count(), MAX_INSERT_CHARS);
    assert!(tx[2].bold);
    assert_eq!(d.slides[&ImportNote::Simplified], [2]);
    assert_eq!(d.slides[&ImportNote::TextCut], [3]);
    // Laid out on a canvas without a panic.
    let t = Instant::now();
    import(&d);
    eprintln!("on the canvas in {:.1} s", t.elapsed().as_secs_f64());
}

/// T-M4, T-M5: a SmartArt data part of 1.5M empty points (34 MB) took a
/// gigabyte; a 300 MB one 600 MB; 500 MB of parts nothing uses 580 MB.
/// None of them is unpacked now.
#[test]
fn big_smartart_and_unused_parts_cost_little_memory() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let base = unpack(&fixture("smartart.pptx"));
    let streamed = |name: &str, head: &[u8], chunk: &[u8], times: usize, tail: &[u8]| {
        // Written chunk by chunk: the test itself never holds it.
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, b) in base.iter().filter(|(n, _)| n != name) {
            z.start_file(n.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(head).unwrap();
        for _ in 0..times {
            z.write_all(chunk).unwrap();
        }
        z.write_all(tail).unwrap();
        z.finish().unwrap().into_inner()
    };
    let head = br#"<?xml version="1.0"?><dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:ptLst>"#;
    let points: Vec<u8> = (0..1000).flat_map(|i| format!("<dgm:pt modelId=\"{i}\"/>").into_bytes()).collect();
    let huge = streamed("ppt/diagrams/data1.xml", head, &points, 1500, b"</dgm:ptLst></dgm:dataModel>");
    let attr = streamed("ppt/diagrams/data1.xml", br#"<dgm:dataModel xmlns:dgm="d" x=""#, &[b'x'; 1 << 20], 300, br#""/>"#);
    // Over the 2 MB a SmartArt part may be (6.6 MB, 300,000 points: 200 MB
    // in the parser's tree), and just under it (1.9 MB).
    let many = streamed("ppt/diagrams/data1.xml", head, &points, 300, b"</dgm:ptLst></dgm:dataModel>");
    let under = streamed("ppt/diagrams/data1.xml", head, &points, 86, b"</dgm:ptLst></dgm:dataModel>");
    let junk = {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, b) in &base {
            z.start_file(n.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        for j in 0..5 {
            z.start_file(format!("ppt/junk{j}.bin"), zip::write::SimpleFileOptions::default()).unwrap();
            for _ in 0..100 {
                z.write_all(&[0u8; 1 << 20]).unwrap();
            }
            z.start_file(format!("ppt/junk{j}.xml"), zip::write::SimpleFileOptions::default()).unwrap();
            for _ in 0..100 {
                z.write_all(&[b' '; 1 << 20]).unwrap();
            }
        }
        z.finish().unwrap().into_inner()
    };
    for (what, bytes) in [("1.5M points", huge), ("300 MB attribute", attr), ("300k points", many), ("86k points", under), ("1 GB unused", junk)] {
        let ((d, pages), grew) = peak(|| {
            let d = read(bytes).unwrap();
            let pages = d.pages.len();
            (d, pages)
        });
        eprintln!("{what}: heap +{} MB", grew / MB);
        assert_eq!(pages, 4, "{what}");
        assert!(grew < 64 * MB, "{what}: heap +{} MB", grew / MB);
        drop(d);
    }
}

/// T-W10: an MP4 saying it lasts 0xfffffffe seconds (or "unknown", all
/// ones) comes in with its length unknown, not 136 years.
#[test]
fn a_video_with_a_lying_length_comes_in_with_it_unknown() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let parts = unpack(&fixture("extras.pptx"));
    for said in [u32::MAX, u32::MAX - 1, 10] {
        let mut mp4 = zebra_core::media::fixtures::tiny_mp4(1280, 720, 10, true);
        let at = mp4.windows(4).position(|w| w == b"mvhd").unwrap() + 4;
        // version/flags 4, created 4, modified 4, timescale 4, duration 4.
        mp4[at + 12..at + 16].copy_from_slice(&1u32.to_be_bytes());
        mp4[at + 16..at + 20].copy_from_slice(&said.to_be_bytes());
        let d = read(pack(&patch(&parts, &[("ppt/media/media1.mp4", mp4)], &[]))).unwrap();
        let durations: Vec<f64> = d.pages.iter().flat_map(|p| &p.items).filter_map(|i| if let ElementKind::Video(v) = &i.element.kind { Some(v.duration) } else { None }).collect();
        assert_eq!(durations, [if said == 10 { 10.0 } else { 0.0 }], "{said}");
        import(&d);
    }
}

/// A17 of the cloud pass: a video part that unpacks to far more than it
/// takes in the file (a bomb: the MP4's first bytes, then zeros) was cut
/// at 200 times its packed size and came in, cut, as slide 4's sound. It
/// is left out now, noted, and nothing else takes its place.
#[test]
fn a_bomb_video_is_left_out_not_cut_short() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let extras = fixture("extras.pptx");
    let seed = unpack(&extras).into_iter().find(|(n, _)| n == "ppt/media/media1.mp4").unwrap().1;
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(&extras)).unwrap();
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let f = src.by_index_raw(i).unwrap();
        if f.name().unwrap() == "ppt/media/media1.mp4" {
            drop(f);
            w.start_file("ppt/media/media1.mp4", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&seed[..64]).unwrap();
            let zeros = vec![0u8; 1 << 20];
            for _ in 0..256 {
                w.write_all(&zeros).unwrap();
            }
        } else {
            w.raw_copy_file(f).unwrap();
        }
    }
    let bytes = w.finish().unwrap().into_inner();
    assert!(bytes.len() < 4 << 20, "{} bytes", bytes.len());
    let (d, grew) = peak(|| read(bytes).unwrap());
    eprintln!("256 MB bomb video: heap +{} MB", grew / MB);
    let slide4 = &d.pages[3];
    assert!(!slide4.items.iter().any(|i| matches!(i.element.kind, ElementKind::Video(_))));
    assert!(slide4.voiceover.is_none(), "came in as a sound");
    assert!(d.assets.values().all(|a| a.bytes.len() < 1 << 20), "a cut part came in");
    // One more poster-only video than the deck as it was.
    let before = read(extras).unwrap().notes.get(&ImportNote::Media).copied().unwrap_or(0);
    assert_eq!(d.notes.get(&ImportNote::Media).copied().unwrap_or(0), before + 1, "{:?}", d.notes);
    assert!(grew < 64 * MB, "heap +{} MB", grew / MB);
}

/// Big files: a 2 GB file named .pptx that is nothing is refused without
/// being read (it is mapped, never on the heap).
#[test]
fn a_two_gigabyte_non_deck_is_refused_without_reading_it() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("zebra-pptxh-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.pptx");
    let f = std::fs::File::create(&path).unwrap();
    f.set_len(2 << 30).unwrap();
    drop(f);
    let t = Instant::now();
    let (r, grew) = peak(|| zebra_import::open(&path).err());
    eprintln!("2 GB sparse non-deck: {:.2} s, heap +{} MB", t.elapsed().as_secs_f64(), grew / MB);
    assert_eq!(r, Some(ImportError::NotSupported));
    assert!(grew < 64 * MB);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn notes_are_one_summary() {
    // Every note an import can give has English for the API.
    let _ = BTreeMap::<ImportNote, usize>::new();
    for n in ImportNote::ALL {
        assert!(!n.english(2).is_empty());
    }
}

/// The biggest slide that is read (just under 8 MB of XML, as many runs as
/// fit): what one slide can cost.
#[test]
fn the_biggest_slide_read_costs_a_bounded_heap() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let run = r#"<a:r><a:rPr b="1"/><a:t>ab</a:t></a:r>"#;
    let n = ((8 << 20) - 4096) / run.len();
    let big = s(slide(&text_box(2, &format!("<a:p>{}</a:p>", run.repeat(n)))));
    let bytes = deck(&[big], &[], &[]);
    let t = Instant::now();
    let (d, grew) = peak(|| read(bytes).unwrap());
    eprintln!("one slide of {} KB ({n} runs): read in {:.1} s, heap +{} MB", (n * run.len()) >> 10, t.elapsed().as_secs_f64(), grew / MB);
    assert_eq!(d.pages.len(), 1);
    assert!(grew < 512 * MB, "heap +{} MB", grew / MB);
}
