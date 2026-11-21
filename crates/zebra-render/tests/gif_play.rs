//! B4: a GIF of any length plays every frame. A short one is decoded once
//! and kept; a long or big one is decoded as it plays, about a second
//! ahead, so what it holds doesn't grow with its length.

use std::sync::Arc;
use std::time::{Duration, Instant};

use zebra_core::AssetBytes;
use zebra_render::images::{Decoded, ImageCache, ImageLevel};

/// A `w`x`h` GIF of `n` frames, each `delay` hundredths of a second. Frame
/// k fills `rect(k)` (x, y, w, h) with palette colour k % 256, whose red
/// is k % 256.
fn gif(w: u16, h: u16, n: usize, delay: u16, rect: impl Fn(usize) -> (u16, u16, u16, u16), dispose: impl Fn(usize) -> gif::DisposalMethod) -> AssetBytes {
    let palette: Vec<u8> = (0..256u32).flat_map(|i| [i as u8, 255 - i as u8, 7]).collect();
    let mut out = vec![];
    {
        let mut e = gif::Encoder::new(&mut out, w, h, &palette).unwrap();
        for k in 0..n {
            let (x, y, fw, fh) = rect(k);
            let px = vec![(k % 256) as u8; fw as usize * fh as usize];
            let mut f = gif::Frame::from_indexed_pixels(fw, fh, px, None);
            f.left = x;
            f.top = y;
            f.delay = delay;
            f.dispose = dispose(k);
            e.write_frame(&f).unwrap();
        }
    }
    out.into()
}

/// The red of the pixel at (x, y) of a frame (opaque, so not changed by
/// premultiplying).
fn red(f: &ImageLevel, x: u32, y: u32) -> u8 {
    f.pixmap().data_as_u8_slice()[((y * f.width + x) * 4) as usize]
}

fn animation(c: &ImageCache, key: &str, bytes: &AssetBytes, size: (u32, u32), level: u8) -> Arc<zebra_render::images::Animation> {
    match c.get(key, "gif", bytes, size, level, true) {
        Some(Decoded::Animated(a)) => a,
        other => panic!("not animated: {other:?}"),
    }
}

/// The start of frame `k` (each lasts `d` seconds), nudged inside it.
fn at(k: usize, d: f64) -> f64 {
    k as f64 * d + d / 4.0
}

/// Ask for time `t` until the frame shown is the one at `t` (a worker
/// decodes it), as the screen's repaints would.
fn wait_for(c: &ImageCache, a: &Arc<zebra_render::images::Animation>, t: f64, red_wanted: u8) -> Arc<ImageLevel> {
    let t0 = Instant::now();
    loop {
        let (f, _) = c.frame(a, t, false);
        if red(&f, 0, 0) == red_wanted {
            return f;
        }
        assert!(t0.elapsed() < Duration::from_secs(20), "frame at {t} never came (shows red {})", red(&f, 0, 0));
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// 3,000 frames: more than every old cap (600 frames, 1 G screen pixels,
/// 48 MB). Every one plays, the last too, and it loops.
#[test]
fn a_long_gif_plays_every_frame_and_loops() {
    let n = 3000;
    let d = 0.02;
    let bytes = gif(200, 200, n, 2, |_| (0, 0, 200, 200), |_| gif::DisposalMethod::Keep);
    let c = ImageCache::default();
    let a = animation(&c, "long", &bytes, (200, 200), 0);
    assert_eq!(a.len(), n, "every frame is counted");
    assert!(a.is_played(), "3,000 frames of 200x200 are decoded as they play");
    assert!((a.total() - n as f64 * d).abs() < 1e-6);
    assert_eq!(a.first.width, 200, "the level drawn, not a smaller one");
    // As a capture would ask: each frame at once.
    for k in [0, 1, 599, 600, 2999] {
        let (f, _) = c.frame(&a, at(k, d), true);
        assert_eq!(red(&f, 0, 0), (k % 256) as u8, "frame {k}");
    }
    // Looping: past the end it starts again.
    let (f, _) = c.frame(&a, a.total() + at(3, d), true);
    assert_eq!(red(&f, 0, 0), 3);
    // Played on the workers, frame by frame, across the loop.
    for k in (2950..3000).chain(0..50) {
        wait_for(&c, &a, at(k, d), (k % 256) as u8);
    }
    let frame = 200 * 200 * 4;
    assert!(a.bytes() < 200 * 200 * 8 + 32 * (1 << 20) + 3 * frame, "holds {} bytes", a.bytes());
}

/// A 1080p GIF of 1,000 frames (each changes a small part, as most do)
/// holds the same memory at its end as at its start: its screen and about
/// a second of frames.
#[test]
fn a_big_gif_keeps_memory_flat_while_playing() {
    let n = 1000;
    let d = 0.04;
    let bytes = gif(1920, 1080, n, 4, |k| if k == 0 { (0, 0, 1920, 1080) } else { (0, 0, 64, 64) }, |_| gif::DisposalMethod::Keep);
    let c = ImageCache::default();
    // Level 1 (960x540): all 1,000 frames would be 2 GB.
    let a = animation(&c, "big", &bytes, (1920, 1080), 1);
    assert!(a.is_played());
    assert_eq!(a.first.width, 960);
    let mut most = 0;
    for k in (0..n).step_by(25).chain([n - 1]) {
        let f = wait_for(&c, &a, at(k, d), (k % 256) as u8);
        // The rest of the screen is still the first frame.
        assert_eq!(red(&f, 959, 539), 0);
        most = most.max(a.bytes());
    }
    let screen = 1920 * 1080 * 8;
    assert!(most < screen + 40 * (1 << 20), "held up to {} MB", most >> 20);
}

/// "Restore previous" and "background" frames composite the same whether
/// a GIF is kept whole or played as it decodes, and whether frames are
/// asked for in turn or out of order.
#[test]
fn played_frames_composite_as_kept_frames_do() {
    use gif::DisposalMethod::*;
    let n = 40;
    let rect = |k: usize| if k == 0 { (0, 0, 64, 64) } else { ((k % 8 * 8) as u16, (k / 8 * 8) as u16, 8, 8) };
    let dispose = |k: usize| [Keep, Previous, Background, Any][k % 4];
    let small = gif(64, 64, n, 5, rect, dispose);
    let c = ImageCache::default();
    let kept = animation(&c, "kept", &small, (64, 64), 0);
    assert!(!kept.is_played());
    // The same frames on a screen big enough that they are played: the
    // first 64x64 pixels of each frame are the same picture.
    let big = gif(2048, 2048, n, 5, rect, dispose);
    let played = animation(&c, "played", &big, (2048, 2048), 0);
    assert!(played.is_played());
    let d = 0.05;
    for k in (0..n).chain([7, 3, 30, 1]) {
        let (a, _) = c.frame(&kept, at(k, d), true);
        let (b, _) = c.frame(&played, at(k, d), true);
        for y in 0..64 {
            for x in 0..64 {
                let i = ((y * a.width + x) * 4) as usize;
                let j = ((y * b.width + x) * 4) as usize;
                assert_eq!(a.pixmap().data_as_u8_slice()[i..i + 4], b.pixmap().data_as_u8_slice()[j..j + 4], "frame {k} at {x},{y}");
            }
        }
    }
}

/// A GIF cut off part way plays the frames it has and stays on the last
/// one it could draw; it never stops the screen or spins.
#[test]
fn a_cut_off_gif_plays_what_it_has() {
    let bytes = gif(400, 400, 500, 2, |_| (0, 0, 400, 400), |_| gif::DisposalMethod::Keep);
    let cut: AssetBytes = bytes[..bytes.len() * 2 / 3].to_vec().into();
    let c = ImageCache::default();
    let a = animation(&c, "cut", &cut, (400, 400), 0);
    assert!(a.is_played());
    let n = a.len();
    assert!(n > 100 && n < 500, "{n} frames");
    for k in [0, 10, n - 1] {
        let (f, _) = c.frame(&a, at(k, 0.02), true);
        assert!(red(&f, 0, 0) == (k % 256) as u8 || k == n - 1, "frame {k}");
    }
    for k in 0..n.min(60) {
        let (f, _) = c.frame(&a, at(k, 0.02), false);
        assert!(f.width == 400);
    }
}
