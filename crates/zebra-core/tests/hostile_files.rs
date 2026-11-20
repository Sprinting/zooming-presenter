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

#[test]
fn zip_bombs_are_refused_without_the_memory() {
    let _ = std::fs::create_dir_all(root("bomb"));
    media::set_spill_dir(Some(root("bomb-spill")));

    // document.json: 512 MB of spaces before a real document.
    let bomb = zip_with(|z| {
        z.start_file("document.json", deflated()).unwrap();
        write_n(z, b' ', 512 << 20);
        z.write_all(real_json().as_bytes()).unwrap();
    });
    assert!(bomb.len() < 2 << 20, "{}", bomb.len());
    let before = heap();
    let e = io::load_bytes(&bomb).unwrap_err();
    assert_eq!(e.code, "bad_file");
    if let (Some(a), Some(b)) = (before, heap()) {
        assert!(b.saturating_sub(a) < 64 << 20, "heap grew {} MB", (b - a) >> 20);
    }

    // An asset of 256 MB of zeros named .png is dropped from its first
    // bytes. One that starts as a real PNG and goes on with zeros is read
    // only as far as 100x its compressed size, to a spill file: it is
    // kept as a broken picture (drawn as missing), never 256 MB.
    let mut head = png(4, 4);
    head.truncate(head.len() - 12); // no IEND: zeros follow the header
    let bomb = zip_with(|z| {
        z.start_file("document.json", deflated()).unwrap();
        z.write_all(real_json().as_bytes()).unwrap();
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
    assert!(doc.assets["fakepng"].bytes.len() < 64 << 20, "{}", doc.assets["fakepng"].bytes.len());
    if let (Some(a), Some(b)) = (before, heap()) {
        assert!(b.saturating_sub(a) < 64 << 20, "heap grew {} MB", (b - a) >> 20);
    }

    // A real document with a real deflated picture still opens.
    let pic = png(64, 64);
    let ok = zip_with(|z| {
        z.start_file("document.json", deflated()).unwrap();
        z.write_all(real_json().as_bytes()).unwrap();
        z.start_file("assets/abc.png", deflated()).unwrap();
        z.write_all(&pic).unwrap();
    });
    let doc = io::load_bytes(&ok).unwrap();
    assert_eq!(doc.assets["abc"].bytes.len(), pic.len());
}

#[test]
fn a_document_rewritten_underneath_still_reads_and_saves() {
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

#[test]
fn a_huge_picture_is_refused_before_it_is_copied() {
    let spill = root("huge-spill");
    media::set_spill_dir(Some(spill.clone()));
    let dir = root("huge");
    // A real PNG header, then a sparse 300 MB: costs no disk.
    let p = dir.join("poster.png");
    let f = std::fs::File::create(&p).unwrap();
    (&f).write_all(&png(8, 8)).unwrap();
    f.set_len(media::MAX_PICTURE + (44 << 20)).unwrap();
    let mut ed = Editor::default();
    let e = ed.apply(cmd(json!({"op":"add_image","path":p,"x":0,"y":0})), Origin::Human).unwrap_err();
    assert_eq!(e.code, "too_big", "{e:?}");
    // Nothing copied.
    assert_eq!(std::fs::read_dir(&spill).map(|d| d.count()).unwrap_or(0), 0);

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
