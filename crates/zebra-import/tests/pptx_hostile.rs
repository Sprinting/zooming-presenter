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
use zebra_core::{Editor, ElementKind, Origin, TextData};
use zebra_import::{Control, ImportError, ImportNote, Options};

/// The tests here measure this process's heap or change the memory
/// budget, so they run one at a time (all of them: any test running
/// beside one that measures would count in its heap).
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
        // 9 MB of text: comes in whole (no size limit, as in Keynote).
        s(slide(&words(2, &"y".repeat(9 << 20)))),
        ok("Nine"),
    ];
    let bytes = deck(&slides, &[], &[]);
    let d = read(bytes.clone()).unwrap();
    let got: Vec<&str> = texts(&d).iter().map(|t| t.text.as_str()).collect();
    assert_eq!(got.len(), 6);
    assert_eq!(got[4].len(), 9 << 20);
    assert_eq!([got[0], got[1], got[2], got[3], got[5]], ["One", "Three", "Five", "Six", "Nine"]);
    assert_eq!(d.slides[&ImportNote::PageSkipped], [2, 4, 7]);
    assert_eq!(d.notes[&ImportNote::PageSkipped], 3);
    // The missing layout of slide 6.
    assert_eq!(d.notes.get(&ImportNote::Design), Some(&1));
    let names: Vec<&str> = d.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Slide 1", "Slide 3", "Slide 5", "Slide 6", "Slide 8", "Slide 9"]);
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
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
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
    // 9 MB of XML comes in: the only bound is what its parse takes against
    // the memory budget (half the Mac's memory), as Keynote has no limit.
    let huge = || s(slide(&format!("<!-- {} -->", "x".repeat(9 << 20))));
    assert!(read(deck(&[ok("A"), huge()], &[], &[])).unwrap().slides.is_empty());
    // Past the budget (here made 8 MB of XML's worth): left out.
    zebra_core::io::set_memory_budget(Some(8 << 20).map(|x: u64| x * 55));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            zebra_core::io::set_memory_budget(None);
        }
    }
    let _reset = Reset;
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
/// crashed parley) keeps 60,000 spans and says so; a long box comes in
/// whole, as in Keynote. All-bold runs were quadratic to read.
#[test]
fn long_text_comes_in_whole_and_its_formatting_simplified() {
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
    assert_eq!(tx[2].text.chars().count(), 450_000, "all the text, as in Keynote");
    assert!(tx[2].bold);
    assert_eq!(d.slides[&ImportNote::Simplified], [2]);
    // Laid out on a canvas without a panic.
    let t = Instant::now();
    import(&d);
    eprintln!("on the canvas in {:.1} s", t.elapsed().as_secs_f64());
}

/// T-M4, T-M5: a SmartArt data part of 1.5M empty points (34 MB) took a
/// gigabyte; a 300 MB one 600 MB; 500 MB of parts nothing uses 580 MB.
/// Parts nothing uses are never unpacked; SmartArt past the memory budget
/// is left out and noted.
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
    // SmartArt has no size limit of its own (Keynote has none): a part
    // whose parse would pass the memory budget is left out, noted. Here
    // the budget is made what a 2 MB part takes, to keep the test small.
    zebra_core::io::set_memory_budget(Some((2 << 20) * 55));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            zebra_core::io::set_memory_budget(None);
        }
    }
    let reset = Reset;
    for (what, bytes) in [("1.5M points", huge), ("300 MB attribute", attr), ("300k points", many.clone()), ("86k points", under), ("1 GB unused", junk)] {
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
    // The presentation's own part past the budget: too big to read, said
    // so (was "damaged").
    let mut pres = fixture_part("deck16x9.pptx", "ppt/presentation.xml");
    pres.extend(vec![b' '; 3 << 20]);
    let big = pack(&patch(&unpack(&fixture("deck16x9.pptx")), &[("ppt/presentation.xml", pres)], &[]));
    assert_eq!(read(big).err(), Some(ImportError::NeedsMemory));
    // With the real budget the 6.6 MB part is read, at what its parse costs.
    drop(reset);
    let ((d, smart), grew) = peak(|| {
        let d = read(many).unwrap();
        let smart = d.notes.get(&ImportNote::SmartArt).copied();
        (d, smart)
    });
    eprintln!("300k points, read: heap +{} MB", grew / MB);
    assert_eq!(d.pages.len(), 4);
    let as_it_was = read(fixture("smartart.pptx")).unwrap().notes.get(&ImportNote::SmartArt).copied();
    assert_eq!(smart, as_it_was, "read, not left out: {:?}", d.notes);
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
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
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

// Tables and charts (plan-21nov/28): hostile spans, a big table, a chart
// whose part is missing, values that aren't numbers.

const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

/// A graphic frame `id` holding `data` (graphicData's child) of `uri`.
fn frame(id: u32, uri: &str, data: &str) -> String {
    format!(r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="F{id}"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="100000" y="100000"/><a:ext cx="6000000" cy="3000000"/></p:xfrm><a:graphic><a:graphicData uri="{uri}">{data}</a:graphicData></a:graphic></p:graphicFrame>"#)
}

fn table_frame(id: u32, grid: usize, rows: &str) -> String {
    let cols: String = (0..grid).map(|_| "<a:gridCol w=\"600000\"/>").collect();
    frame(id, "http://schemas.openxmlformats.org/drawingml/2006/table", &format!("<a:tbl><a:tblPr firstRow=\"1\" bandRow=\"1\"/><a:tblGrid>{cols}</a:tblGrid>{rows}</a:tbl>"))
}

fn cell(attrs: &str, text: &str) -> String {
    format!("<a:tc {attrs}><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-GB\" sz=\"1400\"/><a:t>{text}</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>")
}

fn tables_of(deck: &ImportedDeck) -> Vec<&zebra_core::table::TableData> {
    deck.pages.iter().flat_map(|p| &p.items).filter_map(|i| if let ElementKind::Table(t) = &i.element.kind { Some(t) } else { None }).collect()
}

fn charts_of(deck: &ImportedDeck) -> Vec<&zebra_core::chart::ChartData> {
    deck.pages.iter().flat_map(|p| &p.items).filter_map(|i| if let ElementKind::Chart(c) = &i.element.kind { Some(c) } else { None }).collect()
}

/// Spans past the grid, rows of too many or no cells, a merge inside a
/// merge, a style id nobody knows: the table comes in, its text whole,
/// and goes on a canvas.
#[test]
fn a_table_with_spans_outside_its_grid_comes_in() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let rows = [
        format!("<a:tr h=\"300000\">{}{}</a:tr>", cell("gridSpan=\"9\" rowSpan=\"40\"", "Wide"), cell("", "B")),
        format!("<a:tr h=\"300000\">{}{}{}{}</a:tr>", cell("hMerge=\"1\" vMerge=\"1\"", "x"), cell("rowSpan=\"-3\" gridSpan=\"0\"", "=1+1"), cell("gridSpan=\"4294967295\"", "C"), cell("", "D")),
        "<a:tr h=\"-5\"></a:tr>".to_string(),
        format!("<a:tr h=\"99999999999\">{}</a:tr>", cell("vMerge=\"1\"", "E")),
    ]
    .concat();
    let sl = slide(&table_frame(4, 2, &rows).replace("<a:tblPr firstRow=\"1\" bandRow=\"1\"/>", "<a:tblPr firstRow=\"1\" bandRow=\"1\"><a:tableStyleId>{00000000-DEAD-BEEF-0000-000000000000}</a:tableStyleId></a:tblPr>"));
    let deck = read(deck(&[s(sl)], &[], &[])).unwrap();
    let t = tables_of(&deck);
    assert_eq!(t.len(), 1, "{:?}", deck.notes);
    let t = t[0];
    assert!(t.n_rows() >= 4 && t.n_cols() >= 2, "{} x {}", t.n_rows(), t.n_cols());
    let all: Vec<&str> = t.cells.iter().flat_map(|r| r.iter().map(|c| c.input.as_str())).collect();
    for w in ["Wide", "B", "'=1+1"] {
        assert!(all.contains(&w), "{w} in {all:?}");
    }
    assert!(t.merges.iter().all(|m| m.r1 < t.n_rows() && m.c1 < t.n_cols()), "{:?}", t.merges);
    assert!(deck.notes.contains_key(&ImportNote::TableStyle));
    import(&deck);
}

/// A 1,000 x 100 table comes in whole, quickly.
#[test]
fn a_big_table_comes_in_whole() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let row: String = format!("<a:tr h=\"200000\">{}</a:tr>", (0..100).map(|c| cell("", &c.to_string())).collect::<String>());
    let rows = row.repeat(1000);
    let sl = slide(&table_frame(4, 100, &rows));
    let start = Instant::now();
    let deck = read(deck(&[s(sl)], &[], &[])).unwrap();
    let t = tables_of(&deck);
    assert_eq!(t.len(), 1, "{:?}", deck.notes);
    assert_eq!((t[0].n_rows(), t[0].n_cols()), (1000, 100));
    assert_eq!(t[0].cells[999][99].input, "99");
    import(&deck);
    assert!(start.elapsed() < Duration::from_secs(60), "{:?}", start.elapsed());
}

fn chart_part(values: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="{CHART_NS}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{REL}"><c:chart><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>S</c:v></c:tx><c:cat><c:strLit><c:ptCount val="4"/><c:pt idx="0"><c:v>a</c:v></c:pt><c:pt idx="1"><c:v>b</c:v></c:pt><c:pt idx="2"><c:v>c</c:v></c:pt><c:pt idx="3"><c:v>d</c:v></c:pt></c:strLit></c:cat><c:val><c:numLit><c:ptCount val="4"/>{values}</c:numLit></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:crossAx val="1"/><c:scaling><c:min val="NaN"/><c:max val="-1e400"/></c:scaling><c:majorUnit val="0"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#).into_bytes()
}

fn chart_frame(id: u32, rid: &str) -> String {
    frame(id, CHART_NS, &format!("<c:chart xmlns:c=\"{CHART_NS}\" r:id=\"{rid}\"/>"))
}

/// A chart whose part is missing is noted (it has no picture to fall back
/// on); one whose values are NaN, text, past f64 or at a far index comes
/// in with what reads as numbers, and draws.
#[test]
fn charts_with_missing_parts_or_bad_values_come_in() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let vals = r#"<c:pt idx="0"><c:v>NaN</c:v></c:pt><c:pt idx="1"><c:v>abc</c:v></c:pt><c:pt idx="2"><c:v>1e400</c:v></c:pt><c:pt idx="3"><c:v>7</c:v></c:pt><c:pt idx="4000000000"><c:v>5</c:v></c:pt>"#;
    let rels = format!("<Relationship Id=\"rIdC\" Type=\"{REL}/chart\" Target=\"../charts/chart1.xml\"/><Relationship Id=\"rIdM\" Type=\"{REL}/chart\" Target=\"../charts/chart9.xml\"/>");
    let sl = S { xml: Some(slide(&format!("{}{}", chart_frame(4, "rIdC"), chart_frame(5, "rIdM"))).into_bytes()), rels };
    let deck = read(deck(&[sl], &[("ppt/charts/chart1.xml", chart_part(vals))], &[])).unwrap();
    let c = charts_of(&deck);
    assert_eq!(c.len(), 1, "{:?}", deck.notes);
    assert_eq!(c[0].chart_type, zebra_core::chart::ChartType::Column);
    assert!(deck.notes.contains_key(&ImportNote::Chart), "{:?}", deck.notes);
    let opts = (c[0].axis_min, c[0].axis_max, c[0].axis_step);
    assert!([opts.0, opts.1, opts.2].iter().all(|v| v.is_none_or(|v| v.is_finite())), "{opts:?}");
    let r = zebra_render::Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.import_deck(&deck, "Import".into(), Origin::Human).unwrap();
    let id = ed.doc.elements.values().find(|e| e.kind.type_name() == "chart").unwrap().id.clone();
    assert!(r.capture(&ed, &zebra_render::CaptureTarget::Element { id }, 400, 300).is_ok());
}

/// H5: speaker notes that can't be read (damaged, missing, or more than
/// memory holds) were left out without a word. The slide comes in, and
/// a note names it; big notes that fit come in whole.
#[test]
fn notes_that_cant_be_read_are_named() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let parts = unpack(&fixture("deck16x9.pptx"));
    let notes4 = parts.iter().find(|(n, _)| n == "ppt/notesSlides/notesSlide4.xml").unwrap().1.clone();
    let at = notes4.windows(2).position(|w| w == b"?>").unwrap() + 2;
    // A comment of 9 MB in slide 4's notes: valid, big.
    let big = [&notes4[..at], format!("<!-- {} -->", "x".repeat(9 << 20)).as_bytes(), &notes4[at..]].concat();
    let bytes = pack(&patch(&parts, &[("ppt/notesSlides/notesSlide2.xml", b"<p:notes><oops".to_vec()), ("ppt/notesSlides/notesSlide4.xml", big)], &["ppt/notesSlides/notesSlide3.xml"]));
    let d = read(bytes.clone()).unwrap();
    let notes: Vec<&str> = d.pages.iter().map(|p| p.notes.as_str()).collect();
    assert_eq!(notes[..4], ["Opening notes", "", "", "Pictures and shapes"]);
    assert_eq!(d.slides.get(&ImportNote::Notes), Some(&vec![2, 3]));
    assert_eq!(ImportNote::Notes.english(2), "2 slides' speaker notes couldn't be read");
    // Past what memory holds (here made 8 MB of XML's worth): named too.
    zebra_core::io::set_memory_budget(Some((8 << 20) * 55));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            zebra_core::io::set_memory_budget(None);
        }
    }
    let _reset = Reset;
    let d = read(bytes).unwrap();
    assert_eq!(d.pages.len(), 7, "every slide still comes in");
    assert_eq!(d.pages[3].notes, "");
    assert_eq!(d.slides.get(&ImportNote::Notes), Some(&vec![2, 3, 4]));
}

/// B9: EMF and WMF pictures over 64 MB were left out. One of 70 MB (a
/// big comment record before its drawing) comes in drawn, as a small one
/// does; only what memory can't hold is left out, with a note.
#[test]
fn a_big_emf_comes_in_drawn() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let parts = unpack(&fixture("extras.pptx"));
    let emf = parts.iter().find(|(n, _)| n == "ppt/media/image4.emf").unwrap().1.clone();
    let pad = 70usize << 20;
    let mut comment = Vec::with_capacity(pad + 12);
    comment.extend_from_slice(&70u32.to_le_bytes()); // EMR_COMMENT
    comment.extend_from_slice(&((pad + 12) as u32).to_le_bytes());
    comment.extend_from_slice(&(pad as u32).to_le_bytes());
    comment.resize(pad + 12, 0);
    let mut big = emf[..88].to_vec();
    let size = u32::from_le_bytes(big[48..52].try_into().unwrap()) + comment.len() as u32;
    let records = u32::from_le_bytes(big[52..56].try_into().unwrap()) + 1;
    big[48..52].copy_from_slice(&size.to_le_bytes());
    big[52..56].copy_from_slice(&records.to_le_bytes());
    big.extend_from_slice(&comment);
    big.extend_from_slice(&emf[88..]);
    let bytes = pack(&patch(&parts, &[("ppt/media/image4.emf", big)], &[]));
    let d = read(bytes.clone()).unwrap();
    let svg = d.pages[5].items.iter().find_map(|i| match &i.element.kind {
        ElementKind::Image(img) => Some(String::from_utf8_lossy(&d.assets[&img.asset].bytes).to_ascii_lowercase()),
        _ => None,
    });
    assert!(svg.is_some_and(|s| s.contains("#e0c040")), "the big EMF is drawn");
    assert_eq!(d.notes.get(&ImportNote::Metafile), Some(&1), "only the broken one is noted");
    // Past what memory holds (here 1 GB of budget: 64 MB to convert): noted.
    zebra_core::io::set_memory_budget(Some(1 << 30));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            zebra_core::io::set_memory_budget(None);
        }
    }
    let _reset = Reset;
    let d = read(bytes).unwrap();
    assert_eq!(d.notes.get(&ImportNote::Metafile), Some(&2));
}
