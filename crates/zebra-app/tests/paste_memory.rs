//! M22 (Mac pass, 21 Nov): pasting an 8,700 x 8,700 picture took memory
//! from 290 MB to 2.4 GB. The pixels were held as a PNG in memory, then
//! as base64 on the window's thread, then decoded again before they were
//! spilled. Now the PNG is written straight to a scratch file and added by
//! path, like a dropped file: only the pixels are ever held, once.
//!
//! Its own test binary: it counts every allocation.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};

struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Past this the test can't pass; stop before the machine starts swapping.
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

fn data_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("zebra-paste-memory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: set once, before this binary's only test starts a window.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    dir
}

/// Noise, so the PNG is about as big as the pixels (as a photo or a
/// screenshot of one is).
fn noise(w: u32, h: u32) -> Vec<u8> {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    (0..w as usize * h as usize * 4)
        .map(|i| {
            if i % 4 == 3 {
                return 255;
            }
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

#[test]
fn a_pasted_picture_holds_its_pixels_once() {
    let dir = data_dir();
    let mut h = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);

    let (w, hh) = (4000u32, 4000u32);
    let rgba = noise(w, hh);
    let pixels = rgba.len();
    // What the paste worker does, then what the window does with it, with
    // no frame drawn in between (drawing the picture decodes it for the
    // screen, which isn't the paste's doing).
    let base = NOW.load(Relaxed);
    PEAK.store(base, Relaxed);
    let file = zebra_app::media_ui::pasted_png_file(w, hh, rgba).expect("a picture");
    let at = h.state().canvas_center_world();
    h.state_mut().insert_pasted_file(&file, at, "Pasted image".into());
    let extra = PEAK.load(Relaxed).saturating_sub(base);
    let app = h.state();
    assert_eq!(app.editor.doc.elements.len(), 1, "the picture landed");
    let a = app.editor.doc.assets.values().next().unwrap();
    assert_eq!((a.width, a.height), (w, hh), "at its full size");
    // Over what was held before (which counts the pixels): the PNG, its
    // base64 and its decoded copy were each about the pixels' size again
    // (3.5x in all). Now only the encoder's buffers are, a few MB.
    assert!(extra < pixels / 4, "paste held {} MB over the {} MB of pixels", extra / MB, pixels / MB);
    // The old way, for the record: the PNG in memory, then its base64 and
    // a decoded copy (with today's encoder; the old one also held about 3x
    // the pixels while it encoded).
    let rgba = noise(w, hh);
    let base = NOW.load(Relaxed);
    PEAK.store(base, Relaxed);
    let png = zebra_app::media_ui::pasted_png(w, hh, rgba).unwrap();
    h.state_mut().insert_image_bytes(png, at, "Pasted image".into());
    let old = PEAK.load(Relaxed).saturating_sub(base);
    assert!(old > pixels, "the old way held {} MB over", old / MB);
    // The scratch file is gone once the picture is in.
    let left: Vec<_> = walk(&dir).into_iter().filter(|p| p.to_string_lossy().contains("-paste-")).collect();
    assert!(left.is_empty(), "scratch files left: {left:?}");
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
