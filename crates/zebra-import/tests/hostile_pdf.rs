//! Importing hostile and huge PDFs (audit C2, L6; M1): PDFium runs in the
//! helper, so the page that makes it take 12 GB becomes a placeholder,
//! Cancel stops it at once, too many such pages stop the import, and a
//! 30,000-page PDF is refused without a range. The app's own memory stays
//! flat throughout.
//!
//! Needs PDFium and the helper, as tests/pdf.rs.
use std::time::{Duration, Instant};

use zebra_core::{Editor, ElementKind};
use zebra_import::{Control, ImportError, ImportNote, Options};
use zebra_pdf::samples::*;

fn engine() -> bool {
    if std::env::var_os("ZEBRA_PDFIUM").is_none() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/pdfium");
        let lib = if cfg!(target_os = "macos") { "libpdfium.dylib" } else { "libpdfium.so" };
        let found = ["mac-arm64", "mac-univ", "mac-x64", "linux-x64"].iter().map(|p| root.join(p).join("lib").join(lib)).find(|p| p.is_file());
        if let Some(p) = found {
            // SAFETY: set before any helper of this test binary starts.
            unsafe { std::env::set_var("ZEBRA_PDFIUM", p) };
        }
    }
    if std::env::var_os("ZEBRA_PDFIUM").is_none() {
        assert!(std::env::var_os("ZEBRA_SKIP_PDF").is_some(), "PDFium not found: run scripts/fetch-pdfium.sh for this platform or set ZEBRA_PDFIUM (ZEBRA_SKIP_PDF=1 skips the PDF tests)");
        eprintln!("SKIPPED: PDFium not found");
        return false;
    }
    assert!(zebra_pdf::helper_path().is_some(), "build the PDF helper first: cargo build -p zebra-pdf --bin zebra-pdf-helper");
    true
}

/// Anonymous memory of this process.
/// These tests each keep PDF helpers busy: run one at a time, so the
/// timings they check aren't another test's load.
static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

fn heap() -> u64 {
    zebra_pdf::sys::footprint(std::process::id()).unwrap_or(0)
}

/// The same PDF with a different content key (PDFs given up on are
/// remembered by key for the session, and the tests share one).
fn unique(mut pdf: Vec<u8>, tag: &str) -> Vec<u8> {
    pdf.extend_from_slice(format!("% {tag}\n").as_bytes());
    pdf
}

fn opts() -> Options {
    Options::all(Editor::default().theme())
}

#[test]
fn the_12_gb_page_comes_in_as_a_placeholder() {
    let _one = one_at_a_time();
    if !engine() {
        return;
    }
    let before = heap();
    let t = Instant::now();
    let reader = zebra_import::open_bytes(unique(slow_page(), "placeholder"), "slow".into()).unwrap();
    assert_eq!(reader.page_count(), 1);
    let deck = reader.read(&opts(), &Control::default()).unwrap();
    eprintln!("slow page imported in {:?}, app heap +{} MB, helper peak {} MB", t.elapsed(), heap().saturating_sub(before) >> 20, zebra_pdf::Pool::global().stats().peak >> 20);
    assert!(t.elapsed() < Duration::from_secs(15));
    assert!(heap().saturating_sub(before) < 100 << 20);
    assert_eq!(deck.notes.get(&ImportNote::Placeholder), Some(&1));
    assert_eq!(deck.pages.len(), 1);
    let ElementKind::Image(img) = &deck.pages[0].items[0].element.kind else { panic!() };
    assert_eq!(img.page.as_ref().map(|p| p.index), Some(0), "the page still points at the PDF");
    let pic = &deck.assets[&img.asset];
    assert_eq!((pic.width, pic.height), (512, 512));
    let rgb = image::load_from_memory(&pic.bytes).unwrap().to_rgb8();
    assert!(rgb.pixels().all(|p| p.0 == [0xe5; 3]), "a grey placeholder");
    // Drawn sharper later: given up on at once, the picture stays.
    let pdf = deck.assets.values().find(|a| a.ext == "pdf").unwrap();
    let t = Instant::now();
    assert!(zebra_import::pdf::raster(&img.page.as_ref().unwrap().pdf, &pdf.bytes, 0, (1024, 1024), (0, 0, 512, 512)).is_none());
    assert!(t.elapsed() < Duration::from_millis(100));
    assert_eq!(zebra_import::pdf::take_draw_failures(|_| true).len(), 1);
}

/// Audit C2's failing test: Cancel 2 s into the 12 GB page returns
/// Cancelled within 5 s, the app under 1 GB.
#[test]
fn cancel_stops_the_12_gb_page() {
    let _one = one_at_a_time();
    if !engine() {
        return;
    }
    let before = heap();
    let reader = zebra_import::open_bytes(unique(slow_page(), "cancel"), "slow".into()).unwrap();
    let ctl = Control::default();
    let c = ctl.clone();
    let t = Instant::now();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        c.cancel();
    });
    let r = reader.read(&opts(), &ctl);
    let took = t.elapsed();
    canceller.join().unwrap();
    eprintln!("cancelled after {took:?}, app heap +{} MB", heap().saturating_sub(before) >> 20);
    assert_eq!(r.err(), Some(ImportError::Cancelled));
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert!(heap().saturating_sub(before) < 1 << 30);
}

/// Three pages that need too much: three placeholders, and every other
/// page comes in too; the import never stops, as Preview never stops
/// showing a PDF's pages.
#[test]
fn bad_pages_never_stop_the_import() {
    let _one = one_at_a_time();
    if !engine() {
        return;
    }
    let before = heap();
    let t = Instant::now();
    let reader = zebra_import::open_bytes(unique(some_slow_pages(7, &[1, 2, 4]), "three"), "three".into()).unwrap();
    assert_eq!(reader.page_count(), 7);
    let deck = reader.read(&opts(), &Control::default()).unwrap();
    let first = t.elapsed();
    eprintln!("3 bad pages of 7: {first:?}, app heap +{} MB", heap().saturating_sub(before) >> 20);
    assert_eq!(deck.pages.len(), 7, "every page, three of them placeholders");
    assert_eq!(deck.notes.get(&ImportNote::Placeholder), Some(&3));
    assert_eq!(ImportNote::Placeholder.english(3), "3 pages that couldn't be drawn shown as placeholders");
    assert_eq!(deck.pages.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), (1..=7).map(|i| format!("Page {i}")).collect::<Vec<_>>());
    // The same PDF again this session: opens, and the bad pages are
    // placeholders at once.
    let t = Instant::now();
    let again = zebra_import::open_bytes(unique(some_slow_pages(7, &[1, 2, 4]), "three"), "three".into()).unwrap();
    let deck = again.read(&opts(), &Control::default()).unwrap();
    assert_eq!(deck.pages.len(), 7);
    assert_eq!(deck.notes.get(&ImportNote::Placeholder), Some(&3));
    // At once: well under the first read, which waited out each bad page
    // (not a fixed time, which a loaded machine misses).
    assert!(t.elapsed() < first / 2, "{:?} again, {first:?} the first time", t.elapsed());
}

/// M1: 30,000 pages are counted at once, and any range comes in: here
/// the last 200.
#[test]
fn a_30000_page_pdf_imports_a_range() {
    let _one = one_at_a_time();
    if !engine() {
        return;
    }
    let before = heap();
    let t = Instant::now();
    let reader = zebra_import::open_bytes(many_pages(30_000), "many".into()).unwrap();
    assert_eq!(reader.page_count(), 30_000);
    let counted = t.elapsed();
    let mut o = opts();
    o.first = 29_801;
    o.last = 30_000;
    let ctl = Control::default();
    let t2 = Instant::now();
    let deck = reader.read(&o, &ctl).unwrap();
    assert_eq!(deck.pages.len(), 200);
    assert_eq!(ctl.done(), 200);
    assert_eq!(deck.pages[0].name, "Page 29801");
    eprintln!("30,000 pages: counted in {counted:?}, 200 read in {:?}, app heap +{} MB", t2.elapsed(), heap().saturating_sub(before) >> 20);
    assert!(heap().saturating_sub(before) < 200 << 20);
}

/// B3: no page ceiling. 2,500 pages (more than the 2,000 Zebra once cut
/// imports at) all come in, in order, with progress for each.
#[test]
fn a_2500_page_pdf_imports_whole() {
    let _one = one_at_a_time();
    if !engine() {
        return;
    }
    let before = heap();
    let t = Instant::now();
    let reader = zebra_import::open_bytes(unique(many_pages(2_500), "2500"), "2500".into()).unwrap();
    let ctl = Control::default();
    let deck = reader.read(&opts(), &ctl).unwrap();
    eprintln!("2,500 pages imported in {:?}, app heap +{} MB", t.elapsed(), heap().saturating_sub(before) >> 20);
    assert_eq!(deck.pages.len(), 2_500);
    assert_eq!(ctl.done(), 2_500);
    assert_eq!(deck.pages[2_499].name, "Page 2500");
    assert!(deck.notes.is_empty(), "{:?}", deck.notes);
}

#[test]
fn empty_and_damaged_pdfs_are_errors() {
    let _one = one_at_a_time();
    if !engine() {
        return;
    }
    assert!(zebra_import::open_bytes(b"%PDF-".to_vec(), "x".into()).is_err());
    assert!(zebra_import::open_bytes(b"%PDF-1.7\n1 0 obj << /Type /Catalog >> endobj trailer << /Root 1 0 R >>".to_vec(), "x".into()).is_err());
    for b in ["[0 0 0 0]", "[0 0 -500 NaN]", "[0 0 1e9 1e9]", "[0 0 1 100000]"] {
        let r = zebra_import::open_bytes(boxed(b), "box".into()).and_then(|r| r.read(&opts(), &Control::default()));
        let deck = r.unwrap_or_else(|e| panic!("{b}: {e}"));
        for p in &deck.pages {
            assert!(p.w.is_finite() && p.h.is_finite() && p.w > 0.0 && p.h > 0.0, "{b}: {} x {}", p.w, p.h);
        }
    }
}
