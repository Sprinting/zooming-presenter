//! A PDF helper that hangs on a page (up to its 10 s deadline) never
//! holds up the window: the canvas asks for the page's sharper tiles on a
//! worker and shows the page's picture meanwhile; exports, captures, path
//! thumbnails and presenter slides draw on their own threads. No frame
//! waits on the helper, and none takes more than about 100 ms.
//!
//! Needs PDFium and the PDF helper (`cargo build -p zebra-pdf --bin
//! zebra-pdf-helper`) and a GPU (software Vulkan will do); without PDFium
//! it fails (ZEBRA_SKIP_PDF=1 skips it).
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Camera, ElementKind, Origin};
use zebra_pdf::{Lane, Pool};

fn engine() -> bool {
    if std::env::var_os("ZEBRA_PDFIUM").is_none() {
        assert!(std::env::var_os("ZEBRA_SKIP_PDF").is_some(), "PDFium not found: run scripts/fetch-pdfium.sh for this platform or set ZEBRA_PDFIUM (ZEBRA_SKIP_PDF=1 skips the PDF tests)");
        eprintln!("SKIPPED: PDFium not found (set ZEBRA_PDFIUM)");
        return false;
    }
    assert!(zebra_pdf::helper_path().is_some(), "build the PDF helper first: cargo build -p zebra-pdf --bin zebra-pdf-helper");
    true
}

#[test]
fn a_hung_pdf_helper_never_holds_up_the_window() {
    if !engine() {
        return;
    }
    // On the GPU, as in the app: only its canvas draws pages sharper.
    let mut h = Harness::builder()
        .wgpu()
        .with_size(egui::vec2(1100.0, 720.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, cc.wgpu_render_state.clone(), None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    assert!(h.state().gpu_canvas.is_some(), "no GPU canvas");

    // Three PDF pages on the canvas, each with a path step.
    let reader = zebra_import::open_bytes(zebra_pdf::samples::many_pages(3), "Hang".into()).unwrap();
    let theme = h.state().editor.theme();
    let deck = reader.read(&zebra_import::Options::all(theme), &zebra_import::Control::default()).unwrap();
    h.state_mut().editor.import_deck(&deck, "Hang".into(), Origin::Human).unwrap();
    h.run_steps(3);

    // Every draw from the PDF, on whatever thread: none may be on this one.
    let ui = std::thread::current().id();
    let on_ui = Arc::new(AtomicBool::new(false));
    let draws = Arc::new(AtomicU64::new(0));
    {
        let (on_ui, draws) = (on_ui.clone(), draws.clone());
        h.state().renderer.set_page_raster(Some(Arc::new(move |r: &zebra_render::pages::PageRequest| {
            if std::thread::current().id() == ui {
                on_ui.store(true, Ordering::SeqCst);
            }
            draws.fetch_add(1, Ordering::SeqCst);
            zebra_import::pdf::raster(&r.pdf, &r.bytes, r.index, r.page_px, r.rect)
        })));
    }

    // The helper's next request hangs (until it is killed at the deadline).
    Pool::global().misbehave(Lane::Draw, "hang").unwrap();

    // Zoom in on the first page, well past its 512 px picture.
    let ed = &h.state().editor;
    let page = ed
        .doc
        .elements
        .values()
        .find(|e| matches!(&e.kind, ElementKind::Image(i) if i.page.is_some()))
        .map(|e| e.id.clone())
        .expect("an imported page");
    let b = ed.bounds_of(&[page]);
    let c = b.center();
    h.state_mut().editor.view.camera = Camera { cx: c.x, cy: c.y, zoom: 3000.0 / b.width(), rotation: 0.0 };

    // Meanwhile: a picture export (drawn from the PDF on its worker).
    let out = std::env::temp_dir().join(format!("zebra-pdf-hang-{}.png", std::process::id()));
    zebra_app::files::export_png_to(h.state_mut(), out.clone());

    let (mut worst, mut hung, mut waiting, mut frames) = (Duration::ZERO, false, 0, 0);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(3) {
        let t = Instant::now();
        h.step();
        let took = t.elapsed();
        frames += 1;
        worst = worst.max(took);
        hung |= Pool::global().busy(Lane::Draw);
        // Tiles the canvas asked for and shows the picture in place of.
        waiting = waiting.max(h.state().renderer.images().stats().pending);
        std::thread::sleep(Duration::from_millis(10));
    }
    eprintln!("{frames} frames, worst {:.1} ms, {} draws started, {waiting} tiles waiting", worst.as_secs_f64() * 1000.0, draws.load(Ordering::SeqCst));
    assert!(!on_ui.load(Ordering::SeqCst), "the window's thread drew from the PDF");
    assert!(worst < Duration::from_millis(100), "a frame took {worst:?} while the PDF helper hung");
    assert!(draws.load(Ordering::SeqCst) > 0, "nothing was drawn from the PDF: the test zoomed in too little");
    assert!(hung, "the helper never hung: the test tests nothing");
    assert!(waiting > 0, "the canvas asked for no tiles: it drew no PDF page sharper");
    let _ = std::fs::remove_file(out);
}
