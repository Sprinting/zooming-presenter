//! M1: File › Import… of a PDF of more than 200 pages asks first, with
//! pages 1 to 200 filled in; the range can be changed, up to 2,000 pages
//! at a time. Cancel while a page is being drawn kills the PDF helper at
//! once.
//!
//! Needs PDFium and the PDF helper (`cargo build -p zebra-pdf --bin
//! zebra-pdf-helper`); without PDFium it says so and passes.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use zebra_app::import_ui::Flow;
use zebra_app::{AppOptions, ZebraApp};
use zebra_pdf::samples::*;

fn engine() -> bool {
    if std::env::var_os("ZEBRA_PDFIUM").is_none() {
        eprintln!("SKIPPED: PDFium not found (set ZEBRA_PDFIUM)");
        return false;
    }
    assert!(zebra_pdf::helper_path().is_some(), "build the PDF helper first: cargo build -p zebra-pdf --bin zebra-pdf-helper");
    true
}

fn harness() -> Harness<'static, ZebraApp> {
    Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }))
}

fn file(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zebra-import-sheet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// Step until `done` says so (the import's workers run on threads).
fn until(h: &mut Harness<'static, ZebraApp>, secs: u64, done: impl Fn(&ZebraApp) -> bool) {
    let t = Instant::now();
    while !done(h.state()) {
        assert!(t.elapsed() < Duration::from_secs(secs), "timed out");
        std::thread::sleep(Duration::from_millis(20));
        h.step();
    }
}

fn range(app: &ZebraApp) -> Option<(f64, f64, bool)> {
    match &app.import {
        Some(Flow::Choose { from, to, all, .. }) => Some((*from, *to, *all)),
        _ => None,
    }
}

#[test]
fn a_30000_page_pdf_asks_with_1_to_200_filled_in() {
    if !engine() {
        return;
    }
    let path = file("Atlas.pdf", &many_pages(30_000));
    let mut h = harness();
    h.run_steps(3);
    zebra_app::import_ui::start(h.state_mut(), path.clone());
    until(&mut h, 30, |a| range(a).is_some());
    h.run_steps(3);
    assert_eq!(range(h.state()), Some((1.0, 200.0, false)));
    h.get_by_label("This PDF has 30,000 pages.");
    h.get_by_label("Import pages");
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        h.render().unwrap().save(Path::new(&dir).join("import-many-pages.png")).unwrap();
    }

    // More than 2,000: said under the range, and Import does nothing.
    if let Some(Flow::Choose { from, to, .. }) = &mut h.state_mut().import {
        (*from, *to) = (250.0, 2300.0);
    }
    h.run_steps(3);
    h.get_by_label("At most 2,000 pages at a time.");
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        h.render().unwrap().save(Path::new(&dir).join("import-many-pages-too-many.png")).unwrap();
    }
    h.get_by_label(&zebra_app::i18n::tr("sheet-import")).click();
    h.run_steps(2);
    assert!(range(h.state()).is_some(), "still asking");

    // A range the person typed: those pages come in, one undo step.
    if let Some(Flow::Choose { from, to, .. }) = &mut h.state_mut().import {
        (*from, *to) = (250.0, 300.0);
    }
    h.run_steps(2);
    assert!(h.query_by_label("At most 2,000 pages at a time.").is_none());
    h.get_by_label(&zebra_app::i18n::tr("sheet-import")).click();
    until(&mut h, 120, |a| a.import.is_none());
    let pages: Vec<String> = h.state().editor.doc.elements.values().filter(|e| matches!(&e.kind, zebra_core::ElementKind::Image(i) if i.page.is_some())).map(|e| e.id.clone()).collect();
    assert_eq!(pages.len(), 51);
    assert_eq!(h.state().editor.doc.path.len(), 51);
}

#[test]
fn a_small_pdf_still_offers_all_pages() {
    if !engine() {
        return;
    }
    let path = file("Short.pdf", &many_pages(12));
    let mut h = harness();
    h.run_steps(3);
    zebra_app::import_ui::start(h.state_mut(), path);
    until(&mut h, 30, |a| range(a).is_some());
    assert_eq!(range(h.state()), Some((1.0, 12.0, true)));
    h.run_steps(2);
    h.get_by_label(&zebra_app::i18n::tr("import-pages-all"));
    assert!(h.query_by_label("Import pages").is_none());
}

/// Cancel while the helper is deep in the 12 GB page: it is killed at
/// once, nothing is added.
#[test]
fn cancel_kills_the_helper_mid_page() {
    if !engine() {
        return;
    }
    let mut slow = slow_page();
    slow.extend_from_slice(b"% sheet\n");
    let path = file("Slow.pdf", &slow);
    let mut h = harness();
    h.run_steps(3);
    zebra_app::import_ui::start(h.state_mut(), path);
    until(&mut h, 30, |a| range(a).is_some());
    h.run_steps(2);
    h.get_by_label(&zebra_app::i18n::tr("sheet-import")).click();
    h.run_steps(2);
    assert!(matches!(h.state().import, Some(Flow::Running { .. })));
    until(&mut h, 10, |_| zebra_pdf::Pool::global().pid(zebra_pdf::Lane::Import).is_some());
    let pid = zebra_pdf::Pool::global().pid(zebra_pdf::Lane::Import).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let before = h.state().editor.doc.elements.len();
    h.get_by_label(&zebra_app::i18n::tr("sheet-cancel")).click();
    h.run_steps(1);
    let t = Instant::now();
    // SAFETY: signal 0 only asks whether the pid is there.
    while unsafe { libc::kill(pid as i32, 0) } == 0 {
        assert!(t.elapsed() < Duration::from_millis(500), "helper still running");
        std::thread::sleep(Duration::from_millis(5));
    }
    eprintln!("helper gone {:?} after Cancel", t.elapsed());
    assert!(h.state().import.is_none());
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.elements.len(), before);
}
