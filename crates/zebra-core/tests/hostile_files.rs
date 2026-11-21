//! Files made to hurt: zip bombs in a `.zebra`, a document rewritten
//! underneath Zebra, and paths that never end when read. Own test binary:
//! heap is measured, and the spill folder is process-wide.
use std::io::Write;

use serde_json::json;
use zebra_core::*;

fn heap() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("RssAnon:"))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb * 1024)
}

/// The spill folder is process-wide: tests that set it run one at a time.
static SPILL: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn spill_lock() -> std::sync::MutexGuard<'static, ()> {
    SPILL.lock().unwrap_or_else(|e| e.into_inner())
}

fn cmd(v: serde_json::Value) -> Command {
    serde_json::from_value(v).unwrap()
}

fn root(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zebra-hostile-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
    let mut v = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
    v
}

/// A `.zebra` whose entries are written by `f`, deflated.
fn zip_with(f: impl FnOnce(&mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>)) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    f(&mut z);
    z.finish().unwrap().into_inner()
}

fn deflated() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true)
}

fn write_n(z: &mut impl Write, byte: u8, n: usize) {
    let chunk = vec![byte; 1 << 20];
    let mut left = n;
    while left > 0 {
        let k = left.min(chunk.len());
        z.write_all(&chunk[..k]).unwrap();
        left -= k;
    }
}

fn real_json() -> String {
    io::to_json(&Document::default())
}

/// A document with a picture for each asset key in `keys`.
fn json_using(keys: &[&str]) -> String {
    use base64::Engine;
    let mut ed = Editor::new(Document::default());
    let b64 = base64::engine::general_purpose::STANDARD.encode(png(4, 4));
    for k in keys {
        let out = ed.apply(cmd(json!({"op": "add_image", "data_base64": b64, "x": 0, "y": 0})), Origin::Human).unwrap();
        let id = out.created[0].clone();
        if let Some(e) = ed.doc.elements.get_mut(&id) {
            if let ElementKind::Image(i) = &mut e.kind {
                i.asset = k.to_string();
            }
        }
    }
    io::to_json(&ed.doc)
}

#[test]
fn zip_bombs_are_refused_without_the_memory() {
    let _spill = spill_lock();
    let _ = std::fs::create_dir_all(root("bomb"));
    media::set_spill_dir(Some(root("bomb-spill")));

    // document.json: 512 MB of spaces before a real document. That is a
    // real document (JSON allows the spaces): it opens, in constant
    // memory, as there is no size limit.
    let bomb = zip_with(|z| {
        z.start_file("document.json", deflated()).unwrap();
        write_n(z, b' ', 512 << 20);
        z.write_all(real_json().as_bytes()).unwrap();
    });
    assert!(bomb.len() < 2 << 20, "{}", bomb.len());
    let before = heap();
    io::load_bytes(&bomb).unwrap();
    if let (Some(a), Some(b)) = (before, heap()) {
        assert!(b.saturating_sub(a) < 64 << 20, "heap grew {} MB", (b - a) >> 20);
    }
    // Spaces and nothing else: not a document, found in constant memory.
    let bomb = zip_with(|z| {
        z.start_file("document.json", deflated()).unwrap();
        write_n(z, b' ', 512 << 20);
    });
    let before = heap();
    let e = io::load_bytes(&bomb).unwrap_err();
    assert_eq!((e.code, e.about), ("bad_file", None), "{e:?}");
    if let (Some(a), Some(b)) = (before, heap()) {
        assert!(b.saturating_sub(a) < 64 << 20, "heap grew {} MB", (b - a) >> 20);
    }

    // An asset of 256 MB of zeros named .png is dropped from its first
    // bytes. One that starts as a real PNG and goes on with zeros is read
    // whole, as nothing is ever cut (B8), to a spill file, not memory.
    let mut head = png(4, 4);
    head.truncate(head.len() - 12); // no IEND: zeros follow the header
    let bomb = zip_with(|z| {
        z.start_file("document.json", deflated()).unwrap();
        z.write_all(json_using(&["zeros", "fakepng"]).as_bytes()).unwrap();
        z.start_file("assets/zeros.png", deflated()).unwrap();
        write_n(z, 0, 256 << 20);
        z.start_file("assets/fakepng.png", deflated()).unwrap();
        z.write_all(&head).unwrap();
        write_n(z, 0, 256 << 20);
    });
    assert!(bomb.len() < 2 << 20, "{}", bomb.len());
    let before = heap();
    let doc = io::load_bytes(&bomb).unwrap();
    assert_eq!(doc.assets.keys().collect::<Vec<_>>(), ["fakepng"]);
    assert_eq!(doc.assets["fakepng"].bytes.len(), head.len() + (256 << 20), "read whole");
    assert!(doc.assets["fakepng"].bytes.is_mapped(), "spilled, not in memory");
    if let (Some(a), Some(b)) = (before, heap()) {
        assert!(b.saturating_sub(a) < 64 << 20, "heap grew {} MB", (b - a) >> 20);
    }
    drop(doc);
    // With the disk nearly full it stops before filling it, and the
    // person is told which one is missing and why.
    media::set_free_space_for_tests(Some(media::SPACE_MARGIN - 1));
    let doc = io::load_bytes(&bomb).unwrap();
    media::set_free_space_for_tests(None);
    assert!(doc.assets.is_empty());
    assert_eq!(doc.warnings.0, vec![warnings::Warning::AssetNotRead { key: "fakepng".into(), kind: warnings::MediaWord::Picture, disk_full: true }]);
    assert!(doc.warnings[0].english().contains("disk is full"));

    // A real document with a real deflated picture still opens.
    let pic = png(64, 64);
    let ok = zip_with(|z| {
        z.start_file("document.json", deflated()).unwrap();
        z.write_all(json_using(&["abc"]).as_bytes()).unwrap();
        z.start_file("assets/abc.png", deflated()).unwrap();
        z.write_all(&pic).unwrap();
    });
    let doc = io::load_bytes(&ok).unwrap();
    assert_eq!(doc.assets["abc"].bytes.len(), pic.len());
}

/// bomb-picture.zebra: a picture nothing in the document uses, which
/// unpacks to 200 MB. Opening it reads none of it: nothing is spilled to
/// disk (it went into the autosave's media before), and the document
/// opens without it.
#[test]
fn assets_nothing_uses_are_not_read() {
    let _spill = spill_lock();
    let spill = root("unused-spill");
    media::set_spill_dir(Some(spill.clone()));
    let mut head = png(4, 4);
    head.truncate(head.len() - 12);
    let pic = png(8, 8);
    let file = root("unused").join("bomb-picture.zebra");
    std::fs::write(
        &file,
        zip_with(|z| {
            z.start_file("document.json", deflated()).unwrap();
            z.write_all(json_using(&["used"]).as_bytes()).unwrap();
            z.start_file("assets/bomb.png", deflated()).unwrap();
            z.write_all(&head).unwrap();
            write_n(z, 0, 256 << 20);
            z.start_file("assets/used.png", deflated()).unwrap();
            z.write_all(&pic).unwrap();
        }),
    )
    .unwrap();
    let doc = io::load(&file).unwrap();
    assert_eq!(doc.assets.keys().collect::<Vec<_>>(), ["used"], "the unused asset isn't read");
    let spilled: u64 = std::fs::read_dir(&spill).into_iter().flatten().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum();
    assert!(spilled < 1 << 20, "{spilled} bytes spilled for an asset nothing uses");
    // Saved again: the same document, without it.
    let out = file.with_file_name("again.zebra");
    io::write_file(&doc, &out).unwrap();
    assert_eq!(io::load(&out).unwrap().assets.keys().collect::<Vec<_>>(), ["used"]);
    media::set_spill_dir(None);
}

#[test]
fn a_document_rewritten_underneath_still_reads_and_saves() {
    let _spill = spill_lock();
    let dir = root("rewrite");
    media::set_spill_dir(Some(root("rewrite-spill")));
    let src = dir.join("pic.png");
    // Big enough to be mapped from the saved file, not held in memory.
    std::fs::write(&src, png(600, 600)).unwrap();
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"add_image","path":src,"x":0,"y":0})), Origin::Human).unwrap();
    let deck = dir.join("Deck.zebra");
    io::save(&mut ed.doc, &deck).unwrap();

    let doc = io::load(&deck).unwrap();
    assert!(!doc.assets.is_empty() && doc.assets.values().all(|a| a.bytes.is_mapped()));
    // Another program truncates and rewrites the file in place (cp over
    // it, a sync client).
    std::fs::OpenOptions::new().write(true).truncate(true).open(&deck).unwrap().write_all(b"not a zebra any more").unwrap();
    // Reading every asset (decode, export, copy) must not crash (SIGBUS)…
    let sum: u64 = doc.assets.values().map(|a| a.bytes.iter().map(|&b| b as u64).sum::<u64>()).sum();
    assert!(sum > 0);
    // …and Save As elsewhere must work.
    let out = dir.join("Rescued.zebra");
    io::write_file(&doc, &out).unwrap();
    assert_eq!(io::load(&out).unwrap().assets.len(), doc.assets.len());
}

#[test]
fn paths_that_never_end_are_refused() {
    let _spill = spill_lock();
    media::set_spill_dir(Some(root("endless-spill")));
    let dir = root("endless");
    let fifo = dir.join("pipe.png");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success());
    let mut paths = vec![std::path::PathBuf::from("/dev/zero"), std::path::PathBuf::from("/dev/urandom"), dir.clone()];
    if made {
        paths.push(fifo);
    }
    let t = std::time::Instant::now();
    for p in &paths {
        let mut ed = Editor::default();
        for op in ["add_image", "add_video"] {
            assert!(ed.apply(cmd(json!({"op":op,"path":p,"x":0,"y":0})), Origin::Human).is_err(), "{op} {}", p.display());
        }
        assert!(ed.apply(cmd(json!({"op":"set_soundtrack","path":p})), Origin::Human).is_err());
        assert!(media::spill_file(p, "png").is_err());
        assert!(io::map_document(p).is_err());
        assert!(io::load(p).is_err());
    }
    assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
}

/// A PNG header claiming `w` x `h` (interlaced or not), with no pixels.
fn lying_png(w: u32, h: u32, interlaced: bool) -> Vec<u8> {
    let mut v = png(8, 8);
    v[16..20].copy_from_slice(&w.to_be_bytes());
    v[20..24].copy_from_slice(&h.to_be_bytes());
    v[28] = interlaced as u8;
    v
}

/// A baseline (C0) or progressive (C2) JPEG frame header claiming w x h.
fn lying_jpeg(w: u16, h: u16, progressive: bool) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0];
    v.extend([0xFF, if progressive { 0xC2 } else { 0xC0 }, 0, 17, 8]);
    v.extend(h.to_be_bytes());
    v.extend(w.to_be_bytes());
    v.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    v.extend([0xFF, 0xD9]);
    v
}

/// Kartik, 21 Nov: no file-size cap on pictures (like Keynote). A picture
/// of any file size comes in; only one whose header says its pixels can't
/// be scaled down within bounds is refused, before anything is copied.
#[test]
fn a_huge_picture_comes_in_and_only_unscalable_ones_are_refused() {
    let _spill = spill_lock();
    let spill = root("huge-spill");
    media::set_spill_dir(Some(spill.clone()));
    let dir = root("huge");
    // A real PNG header, then a sparse 1.2 GB: it comes in.
    let p = dir.join("poster.png");
    let f = std::fs::File::create(&p).unwrap();
    (&f).write_all(&png(8, 8)).unwrap();
    f.set_len(1200 << 20).unwrap();
    drop(f);
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"add_image","path":p,"x":0,"y":0})), Origin::Human).unwrap();
    drop(ed);

    // Headers that lie about their size: a PNG 150,000 pixels square (any
    // height streams) comes in; rows wider than the stream bound, a huge
    // progressive JPEG, are refused at once, nothing copied.
    let mut ed = Editor::default();
    let ok = dir.join("lying-150000.png");
    std::fs::write(&ok, lying_png(150_000, 150_000, false)).unwrap();
    ed.apply(cmd(json!({"op":"add_image","path":ok,"x":0,"y":0})), Origin::Human).unwrap();
    let base = std::fs::read_dir(&spill).map(|d| d.count()).unwrap_or(0);
    // (On the Mac ImageIO draws any JPEG scaled, progressive or not.)
    let mut refused = vec![("wide.png", lying_png(u32::MAX / 2, 10, false))];
    if !cfg!(target_os = "macos") {
        refused.push(("progressive.jpg", lying_jpeg(60_000, 60_000, true)));
    }
    for (name, bytes) in refused {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        let e = ed.apply(cmd(json!({"op":"add_image","path":p,"x":0,"y":0})), Origin::Human).unwrap_err();
        assert_eq!(e.code, "too_big", "{name}: {e:?}");
        assert_eq!(e.about, Some(error::About::Picture));
    }
    assert_eq!(std::fs::read_dir(&spill).map(|d| d.count()).unwrap_or(0), base, "nothing copied");
    // The header rules themselves.
    let h = |b: &[u8]| media::picture_header(b).unwrap();
    // On the Mac ImageIO reads a JPEG scaled at any size; elsewhere only
    // what Zebra's own decoder can hold whole.
    assert_eq!(h(&lying_jpeg(65_000, 65_000, false)).fits(), cfg!(target_os = "macos"), "a baseline JPEG is read scaled at any size on the Mac");
    assert_eq!(h(&lying_jpeg(65_000, 65_000, true)).fits(), cfg!(target_os = "macos"));
    assert!(h(&lying_png(70_000, 70_000, true)).fits(), "interlaced: drawn from its first pass");
    assert_eq!(h(&lying_png(3, 4, true)), media::PictureHeader { format: media::PictureFormat::Png, width: 3, height: 4, interlaced: true, orientation: 1 });
    // (A shadowed editor lives to the end of the scope: its copy of the
    // 150,000-pixel picture would still be spilled.)
    drop(ed);
    let mut ed = Editor::default();

    // A disk image named .png: turned away from its first bytes.
    let dmg = dir.join("Installer.png");
    let f = std::fs::File::create(&dmg).unwrap();
    (&f).write_all(b"koly not a picture").unwrap();
    f.set_len(1 << 30).unwrap();
    let t = std::time::Instant::now();
    assert!(ed.apply(cmd(json!({"op":"add_image","path":dmg,"x":0,"y":0})), Origin::Human).is_err());
    assert!(t.elapsed().as_millis() < 2000);
    assert_eq!(std::fs::read_dir(&spill).map(|d| d.count()).unwrap_or(0), 0);
}

#[test]
fn the_wrong_kind_of_file_is_refused_before_it_is_copied() {
    let _spill = spill_lock();
    let spill = root("kind-spill");
    media::set_spill_dir(Some(spill.clone()));
    let dir = root("kind");
    // A 1 GB (sparse) video given where a picture is asked for.
    let p = dir.join("clip.mp4");
    let f = std::fs::File::create(&p).unwrap();
    (&f).write_all(&media::fixtures::tiny_mp4(640, 360, 5, true)).unwrap();
    f.set_len(1 << 30).unwrap();
    let mut ed = Editor::default();
    let t = std::time::Instant::now();
    assert!(ed.apply(cmd(json!({"op":"add_image","path":p,"x":0,"y":0})), Origin::Human).is_err());
    // And a picture where a sound is asked for.
    let pic = dir.join("pic.png");
    std::fs::write(&pic, png(8, 8)).unwrap();
    assert!(ed.apply(cmd(json!({"op":"set_soundtrack","path":pic})), Origin::Human).is_err());
    assert!(t.elapsed().as_millis() < 500, "{:?}", t.elapsed());
    assert_eq!(std::fs::read_dir(&spill).map(|d| d.count()).unwrap_or(0), 0);
}
