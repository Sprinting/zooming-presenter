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

use zebra_core::import::ImportedDeck;
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
