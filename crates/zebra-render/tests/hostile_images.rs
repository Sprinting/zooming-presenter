//! Hostile pictures (robustness audit C1, H3): each decodes in bounded
//! memory or becomes the "unreadable picture" placeholder. Its own test
//! binary: it counts every allocation.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;

use base64::Engine;
use zebra_core::{Editor, Origin};
use zebra_render::{CaptureTarget, Renderer};

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
/// One measurement at a time: the counter is process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

fn measure<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let base = NOW.load(Relaxed);
    PEAK.store(base, Relaxed);
    let t = f();
    (t, PEAK.load(Relaxed).saturating_sub(base))
}

fn with_picture(bytes: &[u8]) -> Editor {
    let mut ed = Editor::default();
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let r = ed.apply(serde_json::from_value(serde_json::json!({"op":"add_image","data_base64": b64})).unwrap(), Origin::Human);
    assert!(r.is_ok(), "{r:?}");
    ed
}

/// A GIF whose logical screen is `side` square, with three 1x1 frames.
fn gif_bomb(side: u16) -> Vec<u8> {
    let mut g = b"GIF89a".to_vec();
    g.extend_from_slice(&side.to_le_bytes());
    g.extend_from_slice(&side.to_le_bytes());
    g.extend_from_slice(&[0x80, 0, 0, 0, 0, 0, 255, 255, 255]);
    for _ in 0..3 {
        g.extend_from_slice(&[0x21, 0xF9, 4, 0, 10, 0, 0, 0, 0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 0x01, 0]);
    }
    g.push(0x3B);
    g
}

/// C1: the audit's 89-byte GIF asked for 17 GB and aborted the app.
#[test]
fn gif_with_giant_screen_does_not_allocate_it() {
    for bytes in [gif_bomb(0xffff), std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/gif-bomb-65535.gif")).unwrap()] {
        let ed = with_picture(&bytes);
        let (png, peak) = measure(|| Renderer::new().capture(&ed, &CaptureTarget::Overview, 320, 200));
        assert!(png.is_ok());
        assert!(peak < 64 * MB, "peak {} MB", peak / MB);
    }
}

/// C1: a screen inside the size limit but with many frames is decoded at a
/// level (and frame count) that fits the per-GIF budget, reusing one canvas.
#[test]
fn big_gif_screen_with_many_frames_stays_in_budget() {
    let ed = with_picture(&{
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&2000u16.to_le_bytes());
        g.extend_from_slice(&2000u16.to_le_bytes());
        g.extend_from_slice(&[0x80, 0, 0, 0, 0, 0, 255, 255, 255]);
        for _ in 0..30 {
            g.extend_from_slice(&[0x21, 0xF9, 4, 0, 10, 0, 0, 0, 0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 0x01, 0]);
        }
        g.push(0x3B);
        g
    });
    let (png, peak) = measure(|| Renderer::new().capture(&ed, &CaptureTarget::Overview, 320, 200));
    assert!(png.is_ok());
    // One 2000x2000 canvas (16 MB) plus the frames' budget (48 MB); all
    // 30 frames at full size would be 480 MB.
    assert!(peak < 80 * MB, "peak {} MB", peak / MB);
}
