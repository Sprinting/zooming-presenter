//! The PDF helper against good, empty, broken, huge and hostile PDFs, and
//! against a helper that hangs, grows, lies or dies: the app gets an
//! answer or an error in bounded time and memory, never a crash, and
//! never starts helpers without end.
mod common;

use std::time::{Duration, Instant};

use common::*;
use zebra_core::AssetBytes;
use zebra_pdf::{Lane, Limits, PdfError, Pool};

fn pool(limits: Limits) -> Pool {
    Pool::new(limits, Some(helper()))
}

fn no() -> bool {
    false
}

fn alive(pid: u32) -> bool {
    // A zombie still answers kill(0); the pool always waits for its
    // helpers, so a pid that answers is a helper still running.
    // SAFETY: signal 0 only checks the pid.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[test]
fn a_pdf_is_opened_measured_and_drawn() {
    if !engine() {
        return;
    }
    let p = pool(Limits::default());
    let b = AssetBytes::from(many_pages(3));
    let t = Instant::now();
    assert_eq!(p.open(Lane::Draw, "k", &b, &no), Ok(3));
    eprintln!("start + open: {:?}", t.elapsed());
    assert_eq!(p.sizes(Lane::Draw, "k", &b, 0, 2, &no).unwrap(), vec![(200.0, 200.0); 3]);
    let pv = p.preview(Lane::Draw, "k", &b, 1, 64, &no).unwrap();
    assert_eq!(pv.px, (64, 64));
    assert_eq!((pv.w_pt, pv.h_pt), (200.0, 200.0));
    // Blue square at the bottom left, white elsewhere.
    let at = |x: usize, y: usize| &pv.rgb[(y * 64 + x) * 3..][..3];
    assert_eq!(at(10, 55), &[0, 0, 255]);
    assert_eq!(at(55, 10), &[255, 255, 255]);
    let px = p.rect(Lane::Draw, "k", &b, 0, (400, 400), (0, 200, 200, 200), &no).unwrap();
    assert_eq!(px.len(), 200 * 200 * 4);
    assert_eq!(&px[(150 * 200 + 50) * 4..][..4], &[0, 0, 255, 255]);
    // No such page: an error, the helper carries on.
    assert_eq!(p.rect(Lane::Draw, "k", &b, 3, (64, 64), (0, 0, 64, 64), &no), Err(PdfError::Page));
    assert_eq!(p.preview(Lane::Draw, "k", &b, 0, 0, &no).err(), Some(PdfError::Page));
    assert_eq!(p.rect(Lane::Draw, "k", &b, 0, (64, 64), (0, 0, 5000, 64), &no), Err(PdfError::Page));
    assert_eq!(p.stats().spawns, 1);
    assert_eq!(p.stats().deaths, 0);
}

/// Audit C2: the 3.5 KB PDF PDFium spends 12 GB on. The helper is killed
/// at the budget; the app's own memory doesn't move; the page is given up
/// on (asked again: at once); other PDFs carry on in a new helper.
#[test]
fn the_slow_page_is_stopped_at_the_memory_budget() {
    if !engine() {
        return;
    }
    let p = pool(Limits::default());
    let b = AssetBytes::from(slow_page());
    let before = heap();
    let t = Instant::now();
    assert_eq!(p.open(Lane::Import, "slow", &b, &no), Ok(1));
    let r = p.preview(Lane::Import, "slow", &b, 0, 512, &no);
    let took = t.elapsed();
    let s = p.stats();
    eprintln!("slow page: {r:?} after {took:?}; helper peak {} MB; app heap +{} MB", s.peak >> 20, heap().saturating_sub(before) >> 20);
    assert_eq!(r.err(), Some(PdfError::TooDemanding));
    assert!(took < Duration::from_secs(12), "{took:?}");
    assert_eq!(s.deaths, 1);
    assert!(s.peak <= zebra_pdf::limits::BUDGET + (200 << 20), "peak {} MB", s.peak >> 20);
    assert!(heap().saturating_sub(before) < 100 << 20);
    assert!(p.given_up("slow", Some(0)));
    let t = Instant::now();
    assert_eq!(p.rect(Lane::Draw, "slow", &b, 0, (64, 64), (0, 0, 64, 64), &no), Err(PdfError::TooDemanding));
    assert!(t.elapsed() < Duration::from_millis(50));
    // Drawn on the Draw lane: once reported for the note.
    assert_eq!(p.take_draw_failures(|k| k == "slow"), vec!["slow".to_string()]);
    let _ = p.rect(Lane::Draw, "slow", &b, 0, (64, 64), (0, 0, 64, 64), &no);
    assert!(p.take_draw_failures(|_| true).is_empty());
    // The next PDF gets a new helper at once (one death is no streak).
    let good = AssetBytes::from(many_pages(1));
    assert_eq!(p.open(Lane::Import, "good", &good, &no), Ok(1));
    assert!(p.preview(Lane::Import, "good", &good, 0, 64, &no).is_ok());
}

/// Audit C2's failing test: Cancel while PDFium is deep in the page stops
/// it at once, and the helper is gone.
#[test]
fn cancel_kills_the_helper_at_once() {
    if !engine() {
        return;
    }
    let p = pool(Limits::default());
    let b = AssetBytes::from(slow_page());
    assert_eq!(p.open(Lane::Import, "slow", &b, &no), Ok(1));
    let pid = p.pid(Lane::Import).unwrap();
    let t = Instant::now();
    let cancel = || t.elapsed() > Duration::from_secs(2);
    let r = p.preview(Lane::Import, "slow", &b, 0, 512, &cancel);
    let took = t.elapsed();
    eprintln!("cancelled after {took:?}");
    assert_eq!(r.err(), Some(PdfError::Cancelled));
    assert!(took < Duration::from_millis(2300), "{took:?}");
    assert!(!alive(pid));
    assert_eq!(p.pid(Lane::Import), None);
    // Cancel isn't the PDF's fault.
    assert!(!p.given_up("slow", Some(0)));
    assert_eq!(p.stats().deaths, 0);
}

#[test]
fn a_hung_or_growing_helper_is_killed() {
    if !engine() {
        return;
    }
    let p = pool(Limits { page: Duration::from_secs(1), rect: Duration::from_secs(1), budget: 300 << 20, ..Limits::default() });
    let b = AssetBytes::from(many_pages(5));
    for (i, what) in ["hang", "spin", "grow"].iter().enumerate() {
        assert_eq!(p.open(Lane::Draw, "k", &b, &no), Ok(5));
        p.misbehave(Lane::Draw, what).unwrap();
        let t = Instant::now();
        let r = p.rect(Lane::Draw, "k", &b, i as u32, (64, 64), (0, 0, 64, 64), &no);
        eprintln!("{what}: {r:?} after {:?}, peak {} MB", t.elapsed(), p.stats().peak >> 20);
        assert_eq!(r, Err(PdfError::TooDemanding));
        assert!(t.elapsed() < Duration::from_millis(1500));
    }
    assert!(p.stats().peak < (300 + 200) << 20);
    // Three pages of one PDF given up on: the PDF's other pages are still
    // drawn, as Preview keeps showing the pages it can.
    assert!(!p.given_up("k", Some(4)));
    assert_eq!(p.open(Lane::Draw, "k", &b, &no), Ok(5));
    assert!(p.rect(Lane::Draw, "k", &b, 4, (64, 64), (0, 0, 64, 64), &no).is_ok());
}

/// A helper that answers wrongly (a compromised one could): garbage, a
/// frame claiming 2 GB, too few pixels, an exit or an abort. Each is an
/// error and the next request gets a new helper.
#[test]
fn a_lying_or_dying_helper_is_an_error() {
    if !engine() {
        return;
    }
    let p = pool(Limits::default());
    for (i, what) in ["garbage", "oversize", "lie", "exit", "abort"].iter().enumerate() {
        let key = format!("k{i}");
        let b = AssetBytes::from(many_pages(2));
        assert_eq!(p.open(Lane::Draw, &key, &b, &no), Ok(2));
        p.misbehave(Lane::Draw, what).unwrap();
        let t = Instant::now();
        let r = p.rect(Lane::Draw, &key, &b, 0, (64, 64), (0, 0, 64, 64), &no);
        assert_eq!(r, Err(PdfError::TooDemanding), "{what}");
        assert!(t.elapsed() < Duration::from_secs(1), "{what}: {:?}", t.elapsed());
        // The other page of that PDF still draws, in a new helper.
        assert_eq!(p.rect(Lane::Draw, &key, &b, 1, (64, 64), (0, 0, 64, 64), &no).map(|v| v.len()), Ok(64 * 64 * 4), "{what}");
    }
    assert_eq!(p.stats().deaths, 5);
}

/// One hostile file can't make the app start helpers without end: after
/// deaths in a row, each new helper waits longer (0, 0.5, 1, 2… s).
#[test]
fn helpers_that_keep_dying_are_restarted_slower_and_slower() {
    if !engine() {
        return;
    }
    let p = pool(Limits::default());
    let t = Instant::now();
    let mut asked = 0;
    while t.elapsed() < Duration::from_secs(5) {
        // A different PDF each time, so nothing is given up on and only
        // the backoff holds it back.
        let key = format!("k{asked}");
        let b = AssetBytes::from(many_pages(1));
        if p.misbehave(Lane::Draw, "exit").is_ok() {
            let _ = p.rect(Lane::Draw, &key, &b, 0, (8, 8), (0, 0, 8, 8), &no);
        }
        asked += 1;
    }
    let s = p.stats();
    eprintln!("5 s of dying helpers: {asked} requests, {} helpers started", s.spawns);
    // 0 + 0 + 0.5 + 1 + 2 s of waits fit in 5 s: about 6 starts.
    assert!(s.spawns <= 8, "{} spawns", s.spawns);
}

/// The helper never outlives the app: with the socket closed mid-request
/// it ends by itself (the app's parent-death and idle checks are its).
#[test]
fn the_helper_leaves_when_the_app_does() {
    if !engine() {
        return;
    }
    use std::io::Write;
    use std::os::fd::OwnedFd;
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    let mut child = std::process::Command::new(helper()).env("ZEBRA_PDFIUM", std::env::var_os("ZEBRA_PDFIUM").unwrap()).stdin(std::process::Stdio::from(OwnedFd::from(theirs))).spawn().unwrap();
    let mut s = &ours;
    s.write_all(&zebra_pdf::wire::Request::Misbehave("hang".into()).encode()).unwrap();
    s.write_all(&zebra_pdf::wire::Request::Sizes { first: 0, last: 0 }.encode()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    drop(ours);
    let t = Instant::now();
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            eprintln!("helper left {:?} after the app: {st}", t.elapsed());
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(2), "helper still running");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn empty_broken_and_odd_pdfs_are_errors_not_panics() {
    if !engine() {
        return;
    }
    let p = pool(Limits::default());
    let open = |k: &str, v: Vec<u8>| p.open(Lane::Import, k, &AssetBytes::from(v), &no);
    assert_eq!(open("e", vec![]), Err(PdfError::Damaged));
    assert_eq!(open("g", b"%PDF-1.7\n garbage".to_vec()), Err(PdfError::Damaged));
    let good = many_pages(2);
    for cut in [10, 100, good.len() / 2, good.len() - 30] {
        let k = format!("cut{cut}");
        let b = AssetBytes::from(good[..cut].to_vec());
        if let Ok(n) = p.open(Lane::Import, &k, &b, &no) {
            for i in 0..n.min(3) {
                let _ = p.preview(Lane::Import, &k, &b, i, 64, &no);
            }
        }
    }
    // Zero, negative and NaN, and absurdly big page boxes.
    for (k, b) in [("zero", "[0 0 0 0]"), ("neg", "[0 0 -500 NaN]"), ("huge", "[0 0 1e9 1e9]"), ("long", "[0 0 1 100000]")] {
        let b = AssetBytes::from(boxed(b));
        let r = p.open(Lane::Import, k, &b, &no).and_then(|_| p.sizes(Lane::Import, k, &b, 0, 0, &no));
        let pv = p.preview(Lane::Import, k, &b, 0, 512, &no);
        let px = p.rect(Lane::Import, k, &b, 0, (4096, 4096), (0, 0, 512, 512), &no);
        eprintln!("{k}: sizes {r:?}, preview {:?}, rect {:?}", pv.as_ref().map(|v| v.px), px.as_ref().map(|v| v.len()));
        if let Ok(v) = pv {
            assert!(v.px.0.max(v.px.1) <= 512 && v.px.0 >= 1 && v.px.1 >= 1);
        }
    }
    // Asking for too many sizes at once.
    let b = AssetBytes::from(many_pages(2));
    assert_eq!(p.sizes(Lane::Import, "two", &b, 0, 200_000, &no), Err(PdfError::Page));
    assert_eq!(p.stats().deaths, 0);
}

/// 30,000 pages: counted quickly, sizes for a range, a page from the end.
#[test]
fn a_30000_page_pdf_counts_quickly() {
    if !engine() {
        return;
    }
    let p = pool(Limits::default());
    let b = AssetBytes::from(many_pages(30_000));
    let t = Instant::now();
    assert_eq!(p.open(Lane::Import, "big", &b, &no), Ok(30_000));
    let open = t.elapsed();
    let sizes = p.sizes(Lane::Import, "big", &b, 0, 1999, &no).unwrap();
    assert_eq!(sizes.len(), 2000);
    assert!(p.preview(Lane::Import, "big", &b, 29_999, 64, &no).is_ok());
    eprintln!("30,000 pages: open {open:?}, then 2,000 sizes and the last page in {:?}", t.elapsed() - open);
    assert!(t.elapsed() < Duration::from_secs(10));
}

/// The PDF reaches the helper as the file the app mapped, also when that
/// file has no name any more (a document's clone or private copy), and
/// a 4 GB file (sparse) neither enters the app's memory nor hangs it.
#[test]
fn mapped_pdfs_go_over_as_files() {
    if !engine() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("zebra-pdf-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = pool(Limits::default());
    // Some bytes before the PDF: it is a slice of the file, as an asset
    // in a .zebra is.
    let path = dir.join("doc.bin");
    let mut file = b"PK-not-a-pdf-prefix".to_vec();
    let start = file.len();
    let pdf = many_pages(4);
    file.extend_from_slice(&pdf);
    std::fs::write(&path, &file).unwrap();
    let map = std::sync::Arc::new(zebra_core::media::MappedFile::open_unlinked(&path).unwrap());
    std::fs::remove_file(&path).unwrap();
    let b = AssetBytes::Mapped { map, start, len: pdf.len() };
    assert_eq!(p.open(Lane::Draw, "m", &b, &no), Ok(4));
    assert!(p.rect(Lane::Draw, "m", &b, 3, (64, 64), (0, 0, 64, 64), &no).is_ok());

    let big = dir.join("big.pdf");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(4 << 30).unwrap();
    std::os::unix::fs::FileExt::write_all_at(&f, b"%PDF-1.7\n", 0).unwrap();
    drop(f);
    let before = heap();
    let b = AssetBytes::from(zebra_core::media::MappedFile::open(&big).unwrap());
    let t = Instant::now();
    let r = p.open(Lane::Import, "big", &b, &no);
    eprintln!("4 GB sparse: {r:?} after {:?}; app heap +{} MB, helper peak {} MB", t.elapsed(), heap().saturating_sub(before) >> 20, p.stats().peak >> 20);
    assert!(r.is_err());
    assert!(heap().saturating_sub(before) < 64 << 20);
    drop(b);
    let _ = std::fs::remove_dir_all(&dir);
}
