//! Regression: every element kind, zoomed to extreme scales (and the camera
//! inside huge text and pictures), builds and renders in bounded memory;
//! and a PDF export of many big clip-art symbols doesn't decode each one at
//! full size. Its own test binary: it counts every allocation.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

mod common;
use common::*;
use zebra_core::*;
use zebra_render::cpu::render_rgba;
use zebra_render::{frame_cost, CaptureTarget, Renderer, MAX_FRAME_COST};

struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Past this the test can't pass; fail before the machine starts swapping.
const ABORT_AT: usize = 3 << 30;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let n = NOW.fetch_add(l.size(), Relaxed) + l.size();
        if n > ABORT_AT {
            NOW.fetch_sub(l.size(), Relaxed);
            return std::ptr::null_mut();
        }
        PEAK.fetch_max(n, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        NOW.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static A: Counting = Counting;

const MB: usize = 1 << 20;

fn measure<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = NOW.load(Relaxed);
    PEAK.store(base, Relaxed);
    let t = f();
    (t, PEAK.load(Relaxed).saturating_sub(base))
}

/// One of everything: each frame style, each shape kind (some dashed), text
/// with an emoji, an icon, ink, a picture, a connector; once normal-sized
/// around the origin and once enormous, all around `P`.
fn deck(r: &Renderer) -> Editor {
    let mut ed = sample(r, "night");
    for (i, st) in FrameStyle::ALL.iter().enumerate() {
        add(&mut ed, &format!("hf{i}"), 3.0 * i as f64, 2.0, 4e5, 3e5, 1.0, 7.0 * i as f64, frame(*st));
    }
    for (i, k) in ShapeKind::ALL.iter().enumerate() {
        let mut s = shape(*k, Some("theme:shape1"), Some("theme:title"), 6.0);
        if let ElementKind::Shape(d) = &mut s {
            d.dash = if i % 2 == 0 { Dash::Dashed } else { Dash::Dotted };
        }
        add(&mut ed, &format!("hs{i}"), 50.0 * i as f64, -30.0, 2e5, 1.5e5, 1.0, 3.0 * i as f64, s);
    }
    add(&mut ed, "ht", 0.0, 0.0, 0.0, 0.0, 2e3, 5.0, text("Huge words 🦓 here", TextStyle::Title));
    let key = ed.doc.elements.values().find_map(|e| match &e.kind {
        ElementKind::Image(i) => Some(i.asset.clone()),
        _ => None,
    });
    if let Some(key) = key {
        add(&mut ed, "hi", 10.0, 10.0, 3e5, 2e5, 1.0, 11.0, ElementKind::Image(ImageData { asset: key, alt: "big".into(), crop: None, mask: ImageMask::Circle, symbol: None, page: None }));
    }
    add(&mut ed, "hk", 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, ElementKind::Icon(IconData { name: "lightbulb".into(), color: "theme:accent".into(), stroke_width: 2.0 }));
    if let Some(e) = ed.doc.elements.get_mut("hk") {
        e.size = Size { w: 24.0, h: 24.0 };
        e.transform.scale = 1e4;
    }
    ed
}

#[test]
fn every_kind_at_extreme_zoom_stays_bounded() {
    let r = Renderer::new();
    let ed = deck(&r);
    // Points to stand on: inside the huge text's glyphs, the huge picture,
    // the huge shapes' outlines, and empty space between them.
    let spots = [(0.0, 0.0), (120.0, -10.0), (2e5, 1.0), (-1e5, 7.5e4), (37.0, 41.0)];
    let mut worst = 0usize;
    for (x, y) in spots {
        let mut zoom = 1e-3;
        while zoom <= 1e12 {
            let cam = Camera { cx: x, cy: y, zoom, rotation: 33.0 };
            let (list, peak) = measure(|| {
                let list = r.capture_display_list(&ed, &CaptureTarget::Camera { camera: cam }, 640, 400).unwrap();
                let _ = render_rgba(&list);
                list
            });
            let cost = frame_cost(&list.items);
            assert!(cost <= MAX_FRAME_COST, "({x},{y}) zoom {zoom:e}: frame cost {cost}");
            assert!(peak < 400 * MB, "({x},{y}) zoom {zoom:e}: used {} MB", peak / MB);
            worst = worst.max(peak);
            zoom *= 100.0;
        }
    }
    eprintln!("worst frame: {} MB", worst / MB);
}

#[test]
fn pdf_export_of_big_symbols_is_small() {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(1280.0, 800.0);
    // Like clip-art from the library: an SVG stored at 4096 px.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4096" height="4096" viewBox="0 0 32 32"><circle cx="16" cy="16" r="12" fill="#e83"/><rect x="8" y="8" width="8" height="8" fill="#38e"/></svg>"##;
    let mut keys = vec![];
    for i in 0..60 {
        // Different symbols (different bytes), as a clip-art grid has.
        keys.push(ed.add_svg_asset(svg.replace("#e83", &format!("#{:06x}", 0x101010 * (i % 15) + i)).into_bytes()));
    }
    add(&mut ed, "f", 0.0, 0.0, 2000.0, 1250.0, 1.0, 0.0, frame(FrameStyle::Rectangle));
    for (i, k) in keys.iter().enumerate() {
        let (x, y) = (-900.0 + 150.0 * (i % 12) as f64, -500.0 + 200.0 * (i / 12) as f64);
        add(&mut ed, &format!("s{i}"), x, y, 120.0, 120.0, 1.0, 0.0, ElementKind::Image(ImageData { asset: k.clone(), alt: String::new(), crop: None, mask: Default::default(), symbol: None, page: None }));
    }
    ed.doc.path = vec![PathStep { target: StepTarget::Element { id: "f".into() }, builds: vec![], notes: String::new(), duration: None, dwell: None, hide_builds: false, voiceover: None }];
    let (pdf, peak) = measure(|| r.export_pdf(&ed).unwrap());
    eprintln!("pdf {} KB, peak {} MB", pdf.len() / 1024, peak / MB);
    // 60 symbols at 4096 px would be 60 x 64 MB of pixels.
    assert!(peak < 300 * MB, "PDF export used {} MB", peak / MB);
    assert!(pdf.len() < 20 * MB, "PDF is {} MB", pdf.len() / MB);
}
