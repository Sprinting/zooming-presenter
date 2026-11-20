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

/// Run `f` (a big test picture being made) while nothing is measured: the
/// counter is process-wide, so its allocations would land in another
/// test's peak.
fn quietly<T>(f: impl FnOnce() -> T) -> T {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    f()
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

/// A `side` square grey PNG of zeros, written row by row (never held whole).
fn big_grey_png(side: u32) -> Vec<u8> {
    let mut out = vec![];
    {
        let mut e = png::Encoder::new(&mut out, side, side);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().unwrap();
        let mut s = w.stream_writer().unwrap();
        let row = vec![0u8; side as usize];
        for _ in 0..side {
            std::io::Write::write_all(&mut s, &row).unwrap();
        }
        s.finish().unwrap();
    }
    out
}

/// H3: a 1.2 MB 16000x16000 PNG took 3.2 GB (three full copies) and 38 s.
/// Now its rows stream into the level drawn, so even level 0 (8192 px,
/// 268 MB) stays under 400 MB.
#[test]
fn huge_png_streams_into_its_level() {
    let bytes: zebra_core::AssetBytes = quietly(|| big_grey_png(16000).into());
    let cache = zebra_render::images::ImageCache::default();
    let (img, peak) = measure(|| cache.get("p", "png", &bytes, (16000, 16000), 0, true));
    assert_eq!(img.map(|d| d.width()), Some(8192));
    eprintln!("16000x16000 PNG at level 0: peak {} MB", peak / MB);
    assert!(peak < 400 * MB, "level 0 peak {} MB", peak / MB);
}

/// H3: JPEG and WebP decode under the limits and shrink from their own
/// pixel format (no RGBA copy of the full picture).
#[test]
fn jpeg_shrinks_without_an_rgba_copy() {
    let bytes: zebra_core::AssetBytes = quietly(|| {
        let img = image::RgbImage::from_pixel(3000, 2000, image::Rgb([200, 100, 50]));
        let mut jpg = std::io::Cursor::new(Vec::new());
        img.write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
        jpg.into_inner().into()
    });
    let cache = zebra_render::images::ImageCache::default();
    let (d, peak) = measure(|| cache.get("j", "jpg", &bytes, (3000, 2000), 2, true));
    let d = d.unwrap();
    assert_eq!(d.width(), 750);
    // The RGB decode (18 MB) and the levels, not an RGBA copy on top (24 MB).
    assert!(peak < 30 * MB, "peak {} MB", peak / MB);
    let zebra_render::images::Decoded::Still(l) = d else { panic!() };
    let px = &l.data.data.data()[..4];
    assert!((px[0] as i32 - 200).abs() < 6 && (px[1] as i32 - 100).abs() < 6, "{px:?}");
}

/// Leftover of the last fix branch: a CPU capture copied a decoded level
/// into vello_cpu's pixmap, so an 8000² picture was held twice (515 MB).
/// The pixmap and the GPU's image data are now the same buffer.
#[test]
fn a_captured_level_is_held_once() {
    let bytes: zebra_core::AssetBytes = big_grey_png(8000).into();
    let cache = zebra_render::images::ImageCache::default();
    let (pixmap, peak) = measure(|| {
        let d = cache.get("p8k", "png", &bytes, (8000, 8000), 0, true).unwrap();
        let zebra_render::images::Decoded::Still(l) = d else { panic!() };
        assert_eq!((l.width, l.height), (8000, 8000));
        assert_eq!(l.bytes(), 8000 * 8000 * 4);
        // What the CPU renderer draws from is what the GPU draws from.
        let p = l.pixmap();
        assert_eq!(p.data_as_u8_slice().as_ptr(), l.data.data.data().as_ptr());
        p
    });
    eprintln!("8000x8000 PNG level 0 and its CPU pixmap: peak {} MB", peak / MB);
    assert_eq!(pixmap.width(), 8000);
    // One 256 MB level, not two.
    assert!(peak < 300 * MB, "peak {} MB", peak / MB);
}

/// Odd buffers still make a level (the pixmap needs whole 4-byte pixels).
#[test]
fn levels_from_odd_buffers() {
    let mut v = Vec::with_capacity(4 * 9 + 3);
    v.resize(4 * 9, 7u8);
    let l = zebra_render::display::ImageLevel::from_premultiplied("odd", 3, 3, v);
    assert_eq!(l.pixmap().data_as_u8_slice(), &[7u8; 36][..]);
    let l = zebra_render::display::ImageLevel::video_frame("v", 2, 1, vec![1; 8]);
    assert!(l.moving);
}

/// Kartik's decision (21 Nov): no size cap on pictures. On the Mac a JPEG
/// too big to decode whole (here wider than 16,384) is drawn by ImageIO
/// at the level's size, as Keynote does, never at full size in Zebra.
#[test]
#[cfg(target_os = "macos")]
fn a_big_jpeg_decodes_scaled() {
    let (w, h) = (20000u32, 1200u32);
    let bytes: zebra_core::AssetBytes = quietly(|| {
        let img = image::RgbImage::from_fn(w, h, |x, _| if x < w / 2 { image::Rgb([200, 100, 50]) } else { image::Rgb([30, 60, 220]) });
        let mut jpg = std::io::Cursor::new(Vec::new());
        img.write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
        jpg.into_inner().into()
    });
    let cache = zebra_render::images::ImageCache::default();
    let (d, peak) = measure(|| cache.get("j", "jpg", &bytes, (w, h), 1, true));
    let zebra_render::images::Decoded::Still(l) = d.expect("decoded") else { panic!() };
    assert_eq!((l.width, l.height), zebra_render::images::level_size(w, h, 1));
    // Whole, it would be 72 MB of RGB; at 1/4 it is 4.5 MB (twice).
    assert!(peak < 24 * MB, "peak {} MB", peak / MB);
    let px = l.data.data.data();
    assert!((px[0] as i32 - 200).abs() < 8 && (px[1] as i32 - 100).abs() < 8, "{:?}", &px[..4]);
    let end = &px[px.len() - 4..];
    assert!((end[2] as i32 - 220).abs() < 8 && (end[0] as i32 - 30).abs() < 8, "{end:?}");
}

/// A PNG of 160 megapixels (more than any picture decoded whole) streams
/// into its level whatever its size.
#[test]
fn a_png_over_the_whole_decode_budget_streams() {
    let (w, h) = (40000u32, 4000u32);
    let bytes: zebra_core::AssetBytes = quietly(|| {
        let mut out = vec![];
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Grayscale);
        let mut wr = e.write_header().unwrap();
        let mut s = wr.stream_writer().unwrap();
        let row = vec![77u8; w as usize];
        for _ in 0..h {
            std::io::Write::write_all(&mut s, &row).unwrap();
        }
        s.finish().unwrap();
        drop(wr);
        out.into()
    });
    assert!(w as u64 * h as u64 > zebra_render::images::MAX_WHOLE_PIXELS);
    let cache = zebra_render::images::ImageCache::default();
    let (d, peak) = measure(|| cache.get("p", "png", &bytes, (w, h), 2, true));
    let zebra_render::images::Decoded::Still(l) = d.expect("decoded") else { panic!() };
    assert_eq!((l.width, l.height), zebra_render::images::level_size(w, h, 2));
    assert_eq!(l.data.data.data()[..4], [77, 77, 77, 255]);
    // The decoder's own buffers and the level: about 20 MB, whatever the
    // size. Whole, it would be 160 MB of grey and 640 MB as RGBA.
    assert!(peak < 40 * MB, "peak {} MB", peak / MB);
}

/// Measure a real big picture (`ZEBRA_BIG_PICTURE=path`): the time and the
/// heap peak of a fit-to-screen level and of level 0.
#[test]
#[ignore]
fn big_picture_from_env() {
    let Ok(p) = std::env::var("ZEBRA_BIG_PICTURE") else { return };
    let path = std::path::Path::new(&p);
    let bytes: zebra_core::AssetBytes = zebra_core::media::MappedFile::open(path).unwrap().into();
    let (ext, w, h) = zebra_core::editor::image_info(&bytes[..bytes.len().min(256 << 10)]).unwrap();
    eprintln!("{p}: {ext} {w}x{h}, {} MB", bytes.len() >> 20);
    for level in [zebra_render::images::level_for(w, h, 1600.0), 0] {
        let cache = zebra_render::images::ImageCache::default();
        let t = std::time::Instant::now();
        let (d, peak) = measure(|| cache.get("big", &ext, &bytes, (w, h), level, true));
        let d = d.expect("decoded");
        eprintln!("level {level}: {}px wide in {:.1} s, heap peak {} MB", d.width(), t.elapsed().as_secs_f64(), peak / MB);
        if let Ok(out) = std::env::var("ZEBRA_BIG_PICTURE_OUT") {
            if let zebra_render::images::Decoded::Still(l) = &d {
                if level != 0 {
                    let img = image::RgbaImage::from_raw(l.width, l.height, l.data.data.data().to_vec()).unwrap();
                    img.save(&out).unwrap();
                }
            }
        }
    }
}
