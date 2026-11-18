//! Video and audio playback for Zebra.
//!
//! A small player trait with three backends:
//! * **AVFoundation** on macOS (`avf`): hardware decoding through `AVPlayer`,
//!   frames pulled from an `AVPlayerItemVideoOutput` as BGRA pixel buffers and
//!   handed to the renderer as RGBA images; audio through `AVAudioPlayer`;
//!   posters from `AVAssetImageGenerator`.
//! * **Null** elsewhere: nothing plays; the editor shows posters.
//! * **Fake** (`ZEBRA_FAKE_MEDIA=1`): synthetic frames and a wall-clock
//!   position, so the app's playback logic can be tested and screenshotted
//!   on Linux.
//!
//! Players open plain files (see `zebra_core::media::materialize`). Looping
//! and trimming are done by the caller from `position()`, so every backend
//! behaves the same. Memory: a player keeps at most one converted frame; the
//! platform's own decoder queue is bounded by AVFoundation (a few frames).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

#[cfg(target_os = "macos")]
mod avf;

/// One video frame: premultiplied RGBA8 rows, `width * height * 4` bytes.
#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{})", self.width, self.height)
    }
}

/// A playing (or paused) video. All times are seconds from the start of
/// the file.
pub trait VideoPlayer: Send {
    fn play(&mut self);
    fn pause(&mut self);
    fn is_playing(&self) -> bool;
    /// Jump to `secs` (frame-accurate where the platform allows).
    fn seek(&mut self, secs: f64);
    fn position(&self) -> f64;
    /// None until the platform knows.
    fn duration(&self) -> Option<f64>;
    fn set_muted(&mut self, muted: bool);
    /// 0..1.
    fn set_volume(&mut self, volume: f32);
    /// The newest frame, if a new one is ready since the last call.
    /// `max_dim` caps the returned size (the frame is scaled down when the
    /// video shows smaller on screen than its pixel size).
    fn next_frame(&mut self, max_dim: u32) -> Option<Frame>;
    /// True when playback reached the end of the file.
    fn ended(&self) -> bool;
}

/// A playing (or paused) sound.
pub trait AudioPlayer: Send {
    fn play(&mut self);
    fn pause(&mut self);
    fn is_playing(&self) -> bool;
    fn seek(&mut self, secs: f64);
    fn position(&self) -> f64;
    fn duration(&self) -> Option<f64>;
    /// 0..1.
    fn set_volume(&mut self, volume: f32);
    /// Loop forever when true (gapless where the platform supports it).
    fn set_looping(&mut self, looping: bool);
    fn ended(&self) -> bool;
}

/// A media platform.
pub trait Backend: Send + Sync {
    /// "avfoundation", "fake" or "none".
    fn name(&self) -> &'static str;
    /// Whether this backend actually plays anything.
    fn can_play(&self) -> bool;
    fn open_video(&self, path: &Path) -> Result<Box<dyn VideoPlayer>, String>;
    fn open_audio(&self, path: &Path) -> Result<Box<dyn AudioPlayer>, String>;
    /// A still from the video at `at` seconds, at most `max_dim` pixels on
    /// its longer side, encoded as PNG (ready to store as a poster asset).
    fn poster_png(&self, path: &Path, at: f64, max_dim: u32) -> Result<Vec<u8>, String>;
}

/// The backend for this platform (`ZEBRA_FAKE_MEDIA=1` forces the fake one).
pub fn platform() -> Arc<dyn Backend> {
    if std::env::var_os("ZEBRA_FAKE_MEDIA").is_some_and(|v| v != "0") {
        return Arc::new(Fake);
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(avf::AvFoundation)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(Null)
    }
}

/// Nothing plays; posters only.
pub struct Null;

impl Backend for Null {
    fn name(&self) -> &'static str {
        "none"
    }
    fn can_play(&self) -> bool {
        false
    }
    fn open_video(&self, _: &Path) -> Result<Box<dyn VideoPlayer>, String> {
        Err("video plays on macOS only".into())
    }
    fn open_audio(&self, _: &Path) -> Result<Box<dyn AudioPlayer>, String> {
        Err("audio plays on macOS only".into())
    }
    fn poster_png(&self, _: &Path, _: f64, _: u32) -> Result<Vec<u8>, String> {
        Err("poster frames need macOS".into())
    }
}

/// Synthetic media for tests: frames are a colour that cycles with the
/// position, durations come from the file header (or 10 s).
pub struct Fake;

impl Backend for Fake {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn can_play(&self) -> bool {
        true
    }
    fn open_video(&self, path: &Path) -> Result<Box<dyn VideoPlayer>, String> {
        let (w, h, d) = header(path)?;
        Ok(Box::new(FakePlayer::new(w.max(16), h.max(16), d)))
    }
    fn open_audio(&self, path: &Path) -> Result<Box<dyn AudioPlayer>, String> {
        let (_, _, d) = header(path)?;
        Ok(Box::new(FakePlayer::new(0, 0, d)))
    }
    fn poster_png(&self, path: &Path, at: f64, max_dim: u32) -> Result<Vec<u8>, String> {
        let (w, h, _) = header(path)?;
        let (w, h) = fit(w.max(16), h.max(16), max_dim);
        Ok(encode_png(&fake_frame(w, h, at)))
    }
}

fn header(path: &Path) -> Result<(u32, u32, f64), String> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut head = vec![];
    f.take(1 << 20).read_to_end(&mut head).map_err(|e| e.to_string())?;
    let (mut w, mut h, mut d) = (320, 180, 10.0);
    if let Some(i) = mp4_info(&head) {
        if i.0 > 0 {
            (w, h) = (i.0, i.1);
        }
        if i.2 > 0.0 {
            d = i.2;
        }
    }
    Ok((w, h, d))
}

/// Width, height and duration from an MP4 `moov` in the first megabyte.
/// (zebra-core has the full parser; this crate stays dependency-free.)
fn mp4_info(b: &[u8]) -> Option<(u32, u32, f64)> {
    fn boxes(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
        let mut out = vec![];
        let mut p = 0usize;
        while p + 8 <= b.len() {
            let n = u32::from_be_bytes(b[p..p + 4].try_into().unwrap()) as usize;
            let t: [u8; 4] = b[p + 4..p + 8].try_into().unwrap();
            if n < 8 {
                break;
            }
            let end = p.saturating_add(n).min(b.len());
            out.push((t, &b[p + 8..end]));
            p = end;
        }
        out
    }
    let moov = boxes(b).into_iter().find(|(t, _)| t == b"moov")?.1;
    let mut d = 0.0;
    let (mut w, mut h) = (0, 0);
    for (t, body) in boxes(moov) {
        if &t == b"mvhd" && body.len() >= 20 {
            let ts = u32::from_be_bytes(body[12..16].try_into().unwrap()) as f64;
            let du = u32::from_be_bytes(body[16..20].try_into().unwrap()) as f64;
            if ts > 0.0 {
                d = du / ts;
            }
        }
        if &t == b"trak" {
            for (t2, b2) in boxes(body) {
                if &t2 == b"tkhd" && b2.len() >= 8 && w == 0 {
                    let n = b2.len();
                    w = u32::from_be_bytes(b2[n - 8..n - 4].try_into().unwrap()) >> 16;
                    h = u32::from_be_bytes(b2[n - 4..n].try_into().unwrap()) >> 16;
                }
            }
        }
    }
    Some((w, h, d))
}

/// Scale `w`x`h` down to fit `max_dim` (never up).
pub fn fit(w: u32, h: u32, max_dim: u32) -> (u32, u32) {
    let m = w.max(h).max(1);
    if m <= max_dim || max_dim == 0 {
        return (w.max(1), h.max(1));
    }
    let k = max_dim as f64 / m as f64;
    (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
}

fn fake_frame(w: u32, h: u32, t: f64) -> Frame {
    // Hue cycles every 4 s; a white bar sweeps across once a second, so
    // screenshots show motion.
    let [r, g, b] = hsv((t / 4.0).fract(), 0.6, 0.85);
    let bar = (t.fract() * w as f64) as u32;
    let half = (w / 40).max(1);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..h {
        for x in 0..w {
            if x.abs_diff(bar) < half {
                rgba.extend_from_slice(&[255, 255, 255, 255]);
            } else {
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
    }
    Frame { width: w, height: h, rgba }
}

fn hsv(h: f64, s: f64, v: f64) -> [u8; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

/// Minimal PNG encoder (stored deflate blocks) for an opaque frame.
pub fn encode_png(f: &Frame) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, t: &[u8; 4], d: &[u8]) {
        out.extend_from_slice(&(d.len() as u32).to_be_bytes());
        let mut c = t.to_vec();
        c.extend_from_slice(d);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc(&c).to_be_bytes());
    }
    let mut raw = Vec::with_capacity(f.rgba.len() + f.height as usize);
    for row in f.rgba.chunks((f.width * 4) as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, blk) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        z.extend_from_slice(&(blk.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(blk.len() as u16)).to_le_bytes());
        z.extend_from_slice(blk);
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend_from_slice(&f.width.to_be_bytes());
    ihdr.extend_from_slice(&f.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Fake player: the position follows the wall clock while playing.
pub struct FakePlayer {
    w: u32,
    h: u32,
    duration: f64,
    base: f64,
    started: Option<Instant>,
    last_frame_at: Option<f64>,
}

impl FakePlayer {
    fn new(w: u32, h: u32, duration: f64) -> Self {
        FakePlayer { w, h, duration, base: 0.0, started: None, last_frame_at: None }
    }
    fn pos(&self) -> f64 {
        let p = self.base + self.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        p.min(self.duration)
    }
    fn do_play(&mut self) {
        if self.started.is_none() {
            if self.pos() >= self.duration {
                self.base = 0.0;
            }
            self.started = Some(Instant::now());
        }
    }
    fn do_pause(&mut self) {
        self.base = self.pos();
        self.started = None;
    }
    fn do_seek(&mut self, secs: f64) {
        // Not a time (a NaN from a 0/0 somewhere): stay where we are.
        if !secs.is_finite() {
            return;
        }
        self.base = secs.clamp(0.0, self.duration);
        if self.started.is_some() {
            self.started = Some(Instant::now());
        }
        self.last_frame_at = None;
    }
}

impl VideoPlayer for FakePlayer {
    fn play(&mut self) {
        self.do_play()
    }
    fn pause(&mut self) {
        self.do_pause()
    }
    fn is_playing(&self) -> bool {
        self.started.is_some() && self.pos() < self.duration
    }
    fn seek(&mut self, secs: f64) {
        self.do_seek(secs)
    }
    fn position(&self) -> f64 {
        self.pos()
    }
    fn duration(&self) -> Option<f64> {
        Some(self.duration)
    }
    fn set_muted(&mut self, _: bool) {}
    fn set_volume(&mut self, _: f32) {}
    fn next_frame(&mut self, max_dim: u32) -> Option<Frame> {
        // 30 fps.
        let p = (self.pos() * 30.0).floor() / 30.0;
        if self.last_frame_at == Some(p) {
            return None;
        }
        self.last_frame_at = Some(p);
        let (w, h) = fit(self.w, self.h, max_dim);
        Some(fake_frame(w, h, p))
    }
    fn ended(&self) -> bool {
        self.pos() >= self.duration
    }
}

impl AudioPlayer for FakePlayer {
    fn play(&mut self) {
        self.do_play()
    }
    fn pause(&mut self) {
        self.do_pause()
    }
    fn is_playing(&self) -> bool {
        self.started.is_some() && self.pos() < self.duration
    }
    fn seek(&mut self, secs: f64) {
        self.do_seek(secs)
    }
    fn position(&self) -> f64 {
        self.pos()
    }
    fn duration(&self) -> Option<f64> {
        Some(self.duration)
    }
    fn set_volume(&mut self, _: f32) {}
    fn set_looping(&mut self, _: bool) {}
    fn ended(&self) -> bool {
        self.pos() >= self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_scales_down_only() {
        assert_eq!(fit(1920, 1080, 960), (960, 540));
        assert_eq!(fit(320, 180, 960), (320, 180));
        assert_eq!(fit(1080, 1920, 480), (270, 480));
    }

    /// R28: a seek to "not a number" is ignored.
    #[test]
    fn a_nan_seek_is_ignored() {
        let mut p = FakePlayer::new(64, 36, 5.0);
        VideoPlayer::seek(&mut p, 2.0);
        VideoPlayer::seek(&mut p, f64::NAN);
        VideoPlayer::seek(&mut p, f64::INFINITY);
        assert_eq!(VideoPlayer::position(&p), 2.0);
    }

    #[test]
    fn fake_player_clock_and_frames() {
        let mut p = FakePlayer::new(64, 36, 5.0);
        assert!(!VideoPlayer::is_playing(&p));
        assert_eq!(p.next_frame(32).map(|f| (f.width, f.height)), Some((32, 18)));
        assert!(p.next_frame(32).is_none(), "same frame twice");
        VideoPlayer::play(&mut p);
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(VideoPlayer::position(&p) > 0.04);
        VideoPlayer::seek(&mut p, 4.99);
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(VideoPlayer::ended(&p));
        VideoPlayer::pause(&mut p);
        VideoPlayer::play(&mut p); // play at the end restarts
        assert!(VideoPlayer::position(&p) < 0.5);
    }

    #[test]
    fn png_header() {
        assert!(encode_png(&fake_frame(5, 3, 0.0)).starts_with(b"\x89PNG"));
    }
}
