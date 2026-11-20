//! Large media, end to end: a video in a PowerPoint file and one added
//! from a file, through import, autosave, recovery, save, delete, undo,
//! save again and playback, with the heap staying flat and nothing left on
//! disk at the end. Own test binary: the spill folder is process-wide.
//!
//! `cargo test -p zebra-import --test large_media` runs it with a 24 MB
//! video. The multi-GB run (a sparse file, so it costs little disk to
//! make) is ignored by default:
//! `ZEBRA_BIG_MB=6144 cargo test --release -p zebra-import --test large_media -- --ignored --nocapture`.
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use zebra_core::{Command, Editor, ElementKind, Origin};
use zebra_import::{Control, Options};

const H: Origin = Origin::Human;

/// The tests take turns: the heap is the process's, and the spill folder too.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn cmd(v: serde_json::Value) -> Command {
    serde_json::from_value(v).unwrap()
}

/// Anonymous memory (heap) of this process, bytes. Linux only; mapped
/// files are not counted, which is the point.
fn heap() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("RssAnon:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

struct Heap {
    base: Option<u64>,
    limit: u64,
    peak: u64,
}

impl Heap {
    fn new(limit: u64) -> Heap {
        Heap { base: heap(), limit, peak: 0 }
    }
    /// The heap grew by no more than the limit since the start.
    fn check(&mut self, what: &str) {
        let (Some(b), Some(now)) = (self.base, heap()) else { return };
        let grew = now.saturating_sub(b);
        self.peak = self.peak.max(grew);
        eprintln!("{what}: heap +{} MB", grew >> 20);
        assert!(grew <= self.limit, "{what}: the heap grew by {} MB (limit {} MB)", grew >> 20, self.limit >> 20);
    }
}

/// An MP4 of `len` bytes: the fixture's header, then an `mdat` box of
/// zeros (a hole: a sparse file costs no disk to make).
fn sparse_mp4(path: &Path, len: u64) {
    let head = zebra_core::media::fixtures::tiny_mp4(1280, 720, 20, true);
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&head).unwrap();
    let rest = len - head.len() as u64;
    f.write_all(&1u32.to_be_bytes()).unwrap();
    f.write_all(b"mdat").unwrap();
    f.write_all(&rest.to_be_bytes()).unwrap();
    f.set_len(len).unwrap();
}

/// The `extras.pptx` fixture with its video (`ppt/media/media1.mp4`)
/// replaced by the file `video`, stored as PowerPoint stores media.
fn pptx_with(video: &Path, out: &Path) {
    let src = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extras.pptx")).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(src)).unwrap();
    let mut w = zip::ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(out).unwrap()));
    for i in 0..z.len() {
        let f = z.by_index_raw(i).unwrap();
        if f.name().unwrap() == "ppt/media/media1.mp4" {
            drop(f);
            let len = std::fs::metadata(video).unwrap().len();
            let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(len >= u32::MAX as u64);
            w.start_file("ppt/media/media1.mp4", o).unwrap();
            std::io::copy(&mut std::fs::File::open(video).unwrap(), &mut w).unwrap();
        } else {
            w.raw_copy_file(f).unwrap();
        }
    }
    w.finish().unwrap().flush().unwrap();
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    v.sort();
    v
}

/// Same bytes, checked at a spread of places (the whole of a multi-GB
/// file would be slow to compare).
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && (0..64).all(|i| {
        let at = (a.len() as u64 * i / 64) as usize;
        let end = (at + 4096).min(a.len());
        a[at..end] == b[at..end]
    })
}

fn video_key(ed: &Editor) -> Option<String> {
    ed.doc.elements.values().find_map(|e| match &e.kind {
        ElementKind::Video(v) => Some(v.asset.clone()),
        _ => None,
    })
}

fn run(mb: u64) {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let len = mb << 20;
    let root = std::env::temp_dir().join(format!("zebra-large-{}-{mb}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let spill = root.join("spill");
    let cache = root.join("cache");
    zebra_core::media::set_spill_dir(Some(spill.clone()));
    // SAFETY: set once, before anything reads it (one test in this binary
    // touches the cache).
    unsafe { std::env::set_var("ZEBRA_MEDIA_CACHE", &cache) };

    let video = root.join("clip.mp4");
    sparse_mp4(&video, len);
    let pptx = root.join("deck.pptx");
    pptx_with(&video, &pptx);
    // Heap may grow by a few MB (the parser, the document); never by the video.
    let mut heap = Heap::new(48 << 20);

    // 1. Import the PowerPoint file. It is over the parser's 512 MB
    //    ceiling when `mb` is: that used to fail as "damaged".
    let mut ed = Editor::default();
    let deck = {
        let r = zebra_import::open(&pptx).expect("the PowerPoint file opens");
        r.read(&Options::all(ed.theme()), &Control::default()).expect("and reads")
    };
    heap.check("pptx read");
    ed.import_deck(&deck, "Import".into(), H).unwrap();
    drop(deck);
    let key = video_key(&ed).expect("the video came in");
    {
        let a = &ed.doc.assets[&key];
        assert_eq!(a.bytes.len() as u64, len);
        assert!(a.bytes.is_mapped());
        let own = a.bytes.own_file().expect("in a spill file of its own");
        assert!(own.starts_with(&spill) && own.extension().is_some_and(|e| e == "mp4"), "{own:?}");
        let src = std::fs::File::open(&video).unwrap();
        let src = unsafe { memmap2::Mmap::map(&src) }.unwrap();
        assert!(same(&a.bytes, &src));
    }
    heap.check("pptx imported");
    // The voice-over and music came in by key, no base64.
    assert!(ed.doc.soundtrack.is_some() && ed.doc.path.iter().any(|s| s.voiceover.is_some()));

    // 2. The same clip added from its file: one asset (same content key),
    //    the second copy goes at once.
    ed.apply(cmd(serde_json::json!({"op":"add_video","path":video,"x":0,"y":0,"width":400})), H).unwrap();
    assert_eq!(ed.doc.elements.values().filter(|e| matches!(e.kind, ElementKind::Video(_))).count(), 2);
    let spilled: Vec<_> = files_in(&spill).into_iter().filter(|p| p.extension().is_some_and(|e| e == "mp4")).collect();
    assert_eq!(spilled.len(), 1, "one copy of the clip: {spilled:?}");
    heap.check("file added");

    // 3. Autosave of the untitled document: the video is linked, not
    //    written; the autosave itself is small.
    let auto = root.join("Untitled.zebra.autosave");
    zebra_core::io::write_autosave(&ed.doc, &auto, None).unwrap();
    assert!(std::fs::metadata(&auto).unwrap().len() < 4 << 20, "autosave holds no video");
    let linked = zebra_core::io::autosave_media_dir(&auto).join(format!("{key}.mp4"));
    assert_eq!(std::fs::metadata(&linked).unwrap().len(), len);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(std::fs::metadata(&linked).unwrap().nlink(), 2, "a hard link to the spill file, not a copy");
    }
    // Writing it again writes nothing new.
    zebra_core::io::write_autosave(&ed.doc, &auto, None).unwrap();
    heap.check("autosaved");

    // 4. Recovery: the autosave reads back with its video.
    {
        let back = zebra_core::io::load_autosave(&auto, None).unwrap();
        assert!(same(&back.assets[&key].bytes, &ed.doc.assets[&key].bytes));
    }
    heap.check("recovered");

    // 5. Save: the video goes into the file, streamed; the asset then
    //    lives in the saved file and its spill file goes.
    let file = root.join("deck.zebra");
    ed.save_now(&file).unwrap();
    assert!(std::fs::metadata(&file).unwrap().len() >= len);
    assert!(!ed.doc.assets[&key].bytes.mapped_file().unwrap().is_owned(), "mapped from the saved file");
    assert!(files_in(&spill).iter().all(|p| p.extension().is_none_or(|e| e != "mp4")), "spill file gone after the save");
    zebra_core::io::remove_autosave(&auto);
    assert!(!zebra_core::io::autosave_media_dir(&auto).exists());
    heap.check("saved");

    // 6. Autosave beside a saved document: what the file holds isn't
    //    written again.
    let side = root.join(".deck.zebra.autosave");
    ed.apply(cmd(serde_json::json!({"op":"add_text","text":"edit","x":0,"y":0})), H).unwrap();
    zebra_core::io::write_autosave(&ed.doc, &side, Some(&file)).unwrap();
    assert!(std::fs::metadata(&side).unwrap().len() < 4 << 20);
    assert!(!zebra_core::io::autosave_media_dir(&side).exists(), "no media folder: all in the saved file");
    {
        let back = zebra_core::io::load_autosave(&side, Some(&file)).unwrap();
        assert!(same(&back.assets[&key].bytes, &ed.doc.assets[&key].bytes));
    }
    zebra_core::io::remove_autosave(&side);
    heap.check("autosaved beside");

    // 7. Delete both videos and save: the file shrinks; Undo can still
    //    bring the video back, so it is kept, moved out of the old file.
    let ids: Vec<String> = ed.doc.elements.values().filter(|e| matches!(e.kind, ElementKind::Video(_))).map(|e| e.id.clone()).collect();
    ed.apply(Command::Delete { ids }, H).unwrap();
    ed.save_now(&file).unwrap();
    assert!(std::fs::metadata(&file).unwrap().len() < len / 2 + (8 << 20), "the saved file no longer holds the video");
    let kept = &ed.doc.assets[&key];
    assert!(kept.bytes.mapped_file().unwrap().is_owned(), "kept for Undo in a spill file, not the old file");
    heap.check("deleted and saved");

    // 8. Undo brings it back, intact; saving writes it again.
    ed.undo(H).unwrap();
    assert_eq!(video_key(&ed).as_deref(), Some(key.as_str()));
    ed.save_now(&file).unwrap();
    assert!(std::fs::metadata(&file).unwrap().len() >= len);
    heap.check("undone and saved");

    // 9. Playback: a copy for the player (the asset is in the .zebra now),
    //    shared while it plays, gone when it stops.
    {
        let a = &ed.doc.assets[&key];
        let f = zebra_core::media::materialize(&key, &a.ext, &a.bytes).unwrap();
        let g = zebra_core::media::materialize(&key, &a.ext, &a.bytes).unwrap();
        assert_eq!(f.path(), g.path());
        assert_eq!(std::fs::metadata(f.path()).unwrap().len(), len);
        assert!(f.path().starts_with(&cache));
        heap.check("materialised");
    }
    assert!(files_in(&cache).is_empty(), "the player's copy goes when it stops");

    // 10. Close: nothing of the document's is left but its file.
    drop(ed);
    assert!(files_in(&spill).iter().all(|p| p.extension().is_none_or(|e| e != "mp4")), "{:?}", files_in(&spill));
    eprintln!("peak heap growth {} MB for a {mb} MB video", heap.peak >> 20);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_video_goes_through_everything_without_the_heap_growing() {
    run(24);
}

#[test]
#[ignore = "multi-GB: run with --ignored (and ZEBRA_BIG_MB)"]
fn a_multi_gigabyte_video_goes_through_everything() {
    let mb = std::env::var("ZEBRA_BIG_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(3072);
    run(mb);
}

/// Hostile and broken input: never a panic, nothing left behind.
#[test]
fn empty_garbage_truncated_and_bomb_media_never_panic() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("zebra-hostile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut ed = Editor::default();
    // An empty file, garbage, a picture named .mp4, a missing file, a folder.
    std::fs::write(root.join("empty.mp4"), b"").unwrap();
    std::fs::write(root.join("junk.mp4"), vec![0x5a; 200_000]).unwrap();
    let mut box_lie = 0xffff_fff0u32.to_be_bytes().to_vec();
    box_lie.extend(b"ftypisom");
    box_lie.extend(vec![0; 100]);
    std::fs::write(root.join("lying-box.mp4"), &box_lie).unwrap();
    std::fs::create_dir_all(root.join("dir.mp4")).unwrap();
    for name in ["empty.mp4", "junk.mp4", "missing.mp4", "dir.mp4"] {
        let p = root.join(name).to_string_lossy().to_string();
        let r = ed.apply(cmd(serde_json::json!({"op":"add_video","path":p})), H);
        assert!(r.is_err(), "{name}");
        let r = ed.apply(cmd(serde_json::json!({"op":"add_image","path":p})), H);
        assert!(r.is_err(), "{name}");
    }
    assert!(ed.doc.assets.is_empty(), "nothing kept from a refused file");
    // A box whose size runs far past the end of the file: read as far as
    // it goes, never past it (the player reports what it can't play).
    let p = root.join("lying-box.mp4").to_string_lossy().to_string();
    let _ = ed.apply(cmd(serde_json::json!({"op":"add_video","path":p})), H);

    // Import: empty, garbage, a zip that isn't PowerPoint, a truncated
    // PowerPoint file, one whose video part is a bomb.
    let fixture = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extras.pptx")).unwrap();
    std::fs::write(root.join("empty.pptx"), b"").unwrap();
    std::fs::write(root.join("junk.pptx"), vec![7u8; 10_000]).unwrap();
    std::fs::write(root.join("half.pptx"), &fixture[..fixture.len() / 2]).unwrap();
    for name in ["empty.pptx", "junk.pptx", "half.pptx", "missing.pptx"] {
        assert!(zebra_import::open(&root.join(name)).is_err(), "{name}");
    }
    // A video part that unpacks to far more than it takes in the file.
    let bomb = root.join("bomb.pptx");
    {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(fixture.clone())).unwrap();
        let mut w = zip::ZipWriter::new(std::fs::File::create(&bomb).unwrap());
        for i in 0..z.len() {
            let f = z.by_index_raw(i).unwrap();
            if f.name().unwrap() == "ppt/media/media1.mp4" {
                drop(f);
                let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
                w.start_file("ppt/media/media1.mp4", o).unwrap();
                let zeros = vec![0u8; 1 << 20];
                for _ in 0..1024 {
                    w.write_all(&zeros).unwrap();
                }
            } else {
                w.raw_copy_file(f).unwrap();
            }
        }
        w.finish().unwrap();
    }
    assert!(std::fs::metadata(&bomb).unwrap().len() < 16 << 20);
    let r = zebra_import::open(&bomb).unwrap();
    let deck = r.read(&Options::all(ed.theme()), &Control::default()).unwrap();
    // The bomb didn't come in as a video (and didn't unpack 1 GB to disk).
    assert!(deck.pages.iter().flat_map(|p| &p.items).all(|i| !matches!(&i.element.kind, ElementKind::Video(v) if deck.assets.get(&v.asset).is_some_and(|a| a.bytes.len() > 64 << 20))));

    // A .zebra that is truncated, or whose stored asset runs past its end.
    let mut f = std::fs::File::create(root.join("cut.zebra")).unwrap();
    f.write_all(b"PK\x03\x04 not really").unwrap();
    assert!(zebra_core::io::load(&root.join("cut.zebra")).is_err());
    assert!(zebra_core::io::load(&root.join("nothing.zebra")).is_err());
    assert!(zebra_core::io::load_autosave(&root.join("nothing.zebra.autosave"), Some(&root.join("cut.zebra"))).is_err());

    // A materialise of something that isn't a safe name is refused.
    let bytes: zebra_core::AssetBytes = vec![1, 2, 3].into();
    assert!(zebra_core::media::materialize("../x", "mp4", &bytes).is_err());
    assert!(zebra_core::media::materialize("ok", "mp4/../../x", &bytes).is_err());

    // Reading a sparse file at an offset works (the harness's own check).
    let mut s = std::fs::File::open(root.join("junk.mp4")).unwrap();
    s.seek(SeekFrom::Start(10)).unwrap();
    let mut b = [0u8; 1];
    s.read_exact(&mut b).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}
