//! Decoded-image cache with a byte budget.
//!
//! Pictures are decoded at the mip level their on-screen size needs, not at
//! full resolution: level 0 is the picture (capped at 8192 px), level n is
//! half of level n-1. Entries are evicted least-recently-used once the cache
//! is over its budget (default 256 MB), but never ones drawn in the current
//! frame, so a frame always has what it shows.
//!
//! The windowed app decodes on worker threads ([`SceneOptions::async_images`]):
//! while a level decodes, the nearest cached level (or a tiny thumbnail) is
//! drawn, and the waker asks for a repaint once it lands. Captures and PDF
//! export decode synchronously so their output is complete.
//!
//! Animated GIFs decode every frame at the chosen level, capped at
//! [`GIF_BUDGET`] bytes per GIF by stepping down a level.
//!
//! [`SceneOptions::async_images`]: crate::SceneOptions::async_images

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
/// std's Instant panics on wasm32-unknown-unknown (the web player). There
/// the cache only evicts by budget, so the idle clock stands still.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
struct Instant;
#[cfg(target_arch = "wasm32")]
impl Instant {
    fn now() -> Self {
        Instant
    }
    fn duration_since(&self, _: Instant) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

use zebra_core::AssetBytes;

/// One decoded mip level of an image (premultiplied RGBA8). Also used for
/// video frames.
pub struct ImageLevel {
    /// Asset key (for PDF export, which embeds the original file).
    pub asset: String,
    pub width: u32,
    pub height: u32,
    /// For vello (GPU). The blob id is stable, so vello's atlas caches it.
    pub data: vello_cpu::peniko::ImageData,
    /// For vello_cpu, made on first CPU render only (captures), so the
    /// windowed app doesn't hold every picture twice.
    pixmap: OnceLock<Arc<vello_cpu::Pixmap>>,
}

impl std::fmt::Debug for ImageLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ImageLevel({} {}x{})", self.asset, self.width, self.height)
    }
}

impl ImageLevel {
    /// From straight-alpha RGBA8 rows.
    pub fn from_rgba(asset: &str, width: u32, height: u32, mut px: Vec<u8>) -> ImageLevel {
        for p in px.chunks_exact_mut(4) {
            let a = p[3] as u16;
            if a != 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u16 * a + 127) / 255) as u8;
                }
            }
        }
        Self::from_premultiplied(asset, width, height, px)
    }

    /// From premultiplied RGBA8 rows (`width * height * 4` bytes).
    pub fn from_premultiplied(asset: &str, width: u32, height: u32, px: Vec<u8>) -> ImageLevel {
        use vello_cpu::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
        debug_assert_eq!(px.len(), (width * height * 4) as usize);
        let data = ImageData { data: Blob::from(px), format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::AlphaPremultiplied, width, height };
        ImageLevel { asset: asset.to_string(), width, height, data, pixmap: OnceLock::new() }
    }

    pub fn pixmap(&self) -> Arc<vello_cpu::Pixmap> {
        self.pixmap
            .get_or_init(|| {
                Arc::new(vello_cpu::Pixmap::from_parts(
                    self.data.data.data().to_vec(),
                    self.width.min(u16::MAX as u32) as u16,
                    self.height.min(u16::MAX as u32) as u16,
                    vello_cpu::PixelMetadata {
                        alpha_type: vello_cpu::peniko::ImageAlphaType::AlphaPremultiplied,
                        may_have_transparency: true,
                    },
                ))
            })
            .clone()
    }

    /// Bytes held (pixels, plus the CPU copy if one was made).
    pub fn bytes(&self) -> usize {
        let px = self.width as usize * self.height as usize * 4;
        if self.pixmap.get().is_some() { px * 2 } else { px }
    }
}

/// Frames of an animated GIF at one level.
#[derive(Debug)]
pub struct Animation {
    pub frames: Vec<Arc<ImageLevel>>,
    /// End time of each frame, seconds, cumulative.
    pub ends: Vec<f64>,
}

impl Animation {
    pub fn total(&self) -> f64 {
        self.ends.last().copied().unwrap_or(0.0)
    }
    /// The frame showing at time `t` (looping) and seconds until the next.
    pub fn frame_at(&self, t: f64) -> (&Arc<ImageLevel>, f64) {
        let total = self.total();
        if total <= 0.0 || self.frames.len() < 2 {
            return (&self.frames[0], f64::INFINITY);
        }
        let t = t.rem_euclid(total);
        let i = self.ends.partition_point(|e| *e <= t).min(self.frames.len() - 1);
        (&self.frames[i], self.ends[i] - t)
    }
    fn bytes(&self) -> usize {
        self.frames.iter().map(|f| f.bytes()).sum()
    }
}

#[derive(Debug, Clone)]
pub enum Decoded {
    Still(Arc<ImageLevel>),
    Animated(Arc<Animation>),
}

impl Decoded {
    fn bytes(&self) -> usize {
        match self {
            Decoded::Still(l) => l.bytes(),
            Decoded::Animated(a) => a.bytes(),
        }
    }
    pub fn width(&self) -> u32 {
        match self {
            Decoded::Still(l) => l.width,
            Decoded::Animated(a) => a.frames[0].width,
        }
    }
}

/// Largest dimension of level 0.
pub const MAX_IMAGE_DIM: u32 = 8192;
/// Levels stop once the picture is this small.
const MIN_LEVEL_DIM: u32 = 64;
/// Default budget for all decoded pictures.
pub const DEFAULT_BUDGET: usize = 256 << 20;
/// Most bytes one animated GIF may decode to (all frames).
pub const GIF_BUDGET: usize = 48 << 20;
/// Most frames kept per GIF.
const GIF_MAX_FRAMES: usize = 600;

/// Size of `level` for a picture of `w`x`h` pixels.
pub fn level_size(w: u32, h: u32, level: u8) -> (u32, u32) {
    let m = w.max(h).max(1);
    let k = if m > MAX_IMAGE_DIM { MAX_IMAGE_DIM as f64 / m as f64 } else { 1.0 } / (1u64 << level) as f64;
    (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
}

/// The smallest level of a `w`x`h` picture.
pub fn last_level(w: u32, h: u32) -> u8 {
    let mut l = 0u8;
    while l < 15 {
        let (a, b) = level_size(w, h, l);
        if a.max(b) <= MIN_LEVEL_DIM {
            break;
        }
        l += 1;
    }
    l
}

/// The smallest level at least `need_w` pixels wide.
pub fn level_for(w: u32, h: u32, need_w: f64) -> u8 {
    let last = last_level(w, h);
    let mut l = 0;
    while l < last && (level_size(w, h, l + 1).0 as f64) >= need_w {
        l += 1;
    }
    l
}

type Key = (String, u8);

struct Entry {
    img: Decoded,
    used: u64,
    /// When it was last drawn, for the idle trim.
    at: Instant,
}

#[derive(Default)]
struct Inner {
    map: HashMap<Key, Entry>,
    bytes: usize,
    pending: HashSet<Key>,
    /// Assets that failed to decode (drawn as a placeholder).
    failed: HashSet<String>,
}

pub type Waker = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    inner: Mutex<Inner>,
    budget: AtomicUsize,
    /// Bumped once per display list: entries used in this generation are
    /// never evicted.
    generation: AtomicU64,
    waker: Mutex<Option<Waker>>,
    jobs: OnceLock<Mutex<Sender<Job>>>,
}

struct Job {
    key: Key,
    ext: String,
    bytes: AssetBytes,
    size: (u32, u32),
    /// SVG symbols: how to draw them.
    svg: Option<crate::svg::SvgStyle>,
    /// Pictures made rather than decoded (tiles of a PDF page).
    make: Option<Make>,
}

/// Makes a picture on a worker (None when it can't).
pub type Make = Arc<dyn Fn() -> Option<ImageLevel> + Send + Sync>;

/// Cache keys of made tiles start with this. Each tile is its own entry
/// with one level, so the idle trim must not keep "the smallest level" of
/// each.
pub const TILE_PREFIX: &str = "tile:";

/// Thread-safe decoded-image cache; see the module docs.
pub struct ImageCache {
    shared: Arc<Shared>,
}

impl Default for ImageCache {
    fn default() -> Self {
        ImageCache {
            shared: Arc::new(Shared {
                inner: Mutex::default(),
                budget: AtomicUsize::new(DEFAULT_BUDGET),
                generation: AtomicU64::new(1),
                waker: Mutex::new(None),
                jobs: OnceLock::new(),
            }),
        }
    }
}

/// Cache statistics (for the `perf` query).
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct CacheStats {
    pub bytes: usize,
    pub budget: usize,
    pub entries: usize,
    pub pending: usize,
}

fn lock(m: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ImageCache {
    pub fn set_budget(&self, bytes: usize) {
        self.shared.budget.store(bytes, Ordering::Relaxed);
        let g = self.shared.generation.load(Ordering::Relaxed);
        let budget = bytes;
        evict(&mut lock(&self.shared.inner), budget, g);
    }

    pub fn set_waker(&self, w: Option<Waker>) {
        *self.shared.waker.lock().unwrap_or_else(|e| e.into_inner()) = w;
    }

    pub fn stats(&self) -> CacheStats {
        let i = lock(&self.shared.inner);
        CacheStats { bytes: i.bytes, budget: self.shared.budget.load(Ordering::Relaxed), entries: i.map.len(), pending: i.pending.len() }
    }

    pub fn clear(&self) {
        let mut i = lock(&self.shared.inner);
        i.map.clear();
        i.bytes = 0;
        i.failed.clear();
    }

    /// Drop decoded pictures nobody has drawn for `idle`, whatever the
    /// budget: a deck left open in the background gives its memory back.
    /// The smallest level of each picture stays, so the picture is on
    /// screen at once (soft, then sharp) when it is drawn again. Returns
    /// (bytes freed, whether anything that could go later is still held).
    pub fn trim_idle(&self, idle: std::time::Duration) -> (usize, bool) {
        let now = Instant::now();
        let mut i = lock(&self.shared.inner);
        let mut smallest: HashMap<String, u8> = HashMap::new();
        for (k, _) in i.map.iter().filter(|(k, _)| !k.0.starts_with(TILE_PREFIX)) {
            let l = smallest.entry(k.0.clone()).or_insert(k.1);
            *l = (*l).max(k.1);
        }
        let before: usize = i.map.values().map(|e| e.img.bytes()).sum();
        i.map.retain(|k, e| smallest.get(&k.0) == Some(&k.1) || now.duration_since(e.at) < idle);
        i.bytes = i.map.values().map(|e| e.img.bytes()).sum();
        let more = i.map.iter().any(|(k, _)| smallest.get(&k.0) != Some(&k.1));
        (before - i.bytes, more)
    }

    /// Start a new frame: entries touched from now on are protected.
    pub fn next_generation(&self) {
        self.shared.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn is_failed(&self, asset: &str) -> bool {
        lock(&self.shared.inner).failed.contains(asset)
    }

    /// The picture `asset` at `level`, decoding now (`sync`) or on a worker.
    /// While a worker decodes, returns the nearest cached level instead, or
    /// None if there is none yet.
    pub fn get(&self, asset: &str, ext: &str, bytes: &AssetBytes, size: (u32, u32), level: u8, sync: bool) -> Option<Decoded> {
        self.get_styled(asset, ext, bytes, size, level, sync, None)
    }

    /// An SVG symbol drawn with `style`. Each style is its own cache entry,
    /// keyed `key` (the asset key plus the style).
    pub fn get_svg(&self, key: &str, bytes: &AssetBytes, size: (u32, u32), level: u8, sync: bool, style: crate::svg::SvgStyle) -> Option<Decoded> {
        self.get_styled(key, "svg", bytes, size, level, sync, Some(style))
    }

    #[allow(clippy::too_many_arguments)]
    fn get_styled(&self, asset: &str, ext: &str, bytes: &AssetBytes, size: (u32, u32), level: u8, sync: bool, svg: Option<crate::svg::SvgStyle>) -> Option<Decoded> {
        let g = self.shared.generation.load(Ordering::Relaxed);
        let key: Key = (asset.to_string(), level);
        {
            let mut i = lock(&self.shared.inner);
            if i.failed.contains(asset) {
                return None;
            }
            if let Some(e) = i.map.get_mut(&key) {
                e.used = g;
                e.at = Instant::now();
                return Some(e.img.clone());
            }
            if !sync {
                if !i.pending.contains(&key) {
                    i.pending.insert(key.clone());
                    drop(i);
                    self.enqueue(Job { key, ext: ext.to_string(), bytes: bytes.clone(), size, svg, make: None });
                    i = lock(&self.shared.inner);
                }
                // Nearest cached level: prefer sharper, then blurrier.
                let best = i
                    .map
                    .iter_mut()
                    .filter(|((a, _), _)| a == asset)
                    .min_by_key(|((_, l), _)| if *l < level { (0, level - *l) } else { (1, *l - level) });
                return best.map(|(_, e)| {
                    e.used = g;
                    e.at = Instant::now();
                    e.img.clone()
                });
            }
        }
        let out = decode_job(&Job { key: key.clone(), ext: ext.to_string(), bytes: bytes.clone(), size, svg, make: None });
        insert(&self.shared, key, out, g)
    }

    /// The picture `key` made by `make` (now if `sync`, else on a worker:
    /// None until it lands). `key` should start with [`TILE_PREFIX`].
    pub fn get_made(&self, key: &str, sync: bool, make: Make) -> Option<Decoded> {
        let g = self.shared.generation.load(Ordering::Relaxed);
        let key: Key = (key.to_string(), 0);
        {
            let mut i = lock(&self.shared.inner);
            if i.failed.contains(&key.0) {
                return None;
            }
            if let Some(e) = i.map.get_mut(&key) {
                e.used = g;
                e.at = Instant::now();
                return Some(e.img.clone());
            }
            if !sync {
                if i.pending.insert(key.clone()) {
                    drop(i);
                    self.enqueue(Job { key, ext: String::new(), bytes: AssetBytes::from(Vec::new()), size: (0, 0), svg: None, make: Some(make) });
                }
                return None;
            }
        }
        let out = decode_job(&Job { key: key.clone(), ext: String::new(), bytes: AssetBytes::from(Vec::new()), size: (0, 0), svg: None, make: Some(make) });
        insert(&self.shared, key, out, g)
    }

    /// `key` if it is already cached (made by [`ImageCache::get_made`]);
    /// never starts any work.
    pub fn peek(&self, key: &str) -> Option<Decoded> {
        let g = self.shared.generation.load(Ordering::Relaxed);
        let mut i = lock(&self.shared.inner);
        let e = i.map.get_mut(&(key.to_string(), 0))?;
        e.used = g;
        e.at = Instant::now();
        Some(e.img.clone())
    }

    fn enqueue(&self, job: Job) {
        let tx = self.shared.jobs.get_or_init(|| {
            let (tx, rx) = channel::<Job>();
            let rx = Arc::new(Mutex::new(rx));
            // Two workers: enough to keep up with a pan across a photo
            // deck without competing with the UI thread for cores.
            for n in 0..2 {
                let rx = rx.clone();
                let weak = Arc::downgrade(&self.shared);
                let _ = std::thread::Builder::new().name(format!("zebra-decode-{n}")).spawn(move || {
                    loop {
                        let job = match rx.lock().unwrap_or_else(|e| e.into_inner()).recv() {
                            Ok(j) => j,
                            Err(_) => return,
                        };
                        let Some(shared) = weak.upgrade() else { return };
                        let out = decode_job(&job);
                        let g = shared.generation.load(Ordering::Relaxed);
                        insert(&shared, job.key, out, g.saturating_sub(1));
                        let waker = shared.waker.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        if let Some(w) = waker {
                            w();
                        }
                    }
                });
            }
            Mutex::new(tx)
        });
        let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(job);
    }
}

fn insert(shared: &Shared, key: Key, out: Vec<(u8, Decoded)>, g: u64) -> Option<Decoded> {
    let mut i = lock(&shared.inner);
    i.pending.remove(&key);
    if out.is_empty() {
        i.failed.insert(key.0.clone());
        return None;
    }
    let mut wanted = None;
    for (level, img) in out {
        let k = (key.0.clone(), level);
        if level == key.1 {
            wanted = Some(img.clone());
        }
        if !i.map.contains_key(&k) {
            i.bytes += img.bytes();
            i.map.insert(k, Entry { img, used: g, at: Instant::now() });
        }
    }
    evict(&mut i, shared.budget.load(Ordering::Relaxed), shared.generation.load(Ordering::Relaxed));
    wanted
}

fn evict(i: &mut Inner, budget: usize, generation: u64) {
    // Recount: CPU pixmaps made since insertion add to the size.
    i.bytes = i.map.values().map(|e| e.img.bytes()).sum();
    while i.bytes > budget {
        let Some(k) = i.map.iter().filter(|(_, e)| e.used < generation).min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone()) else {
            break;
        };
        if let Some(e) = i.map.remove(&k) {
            i.bytes -= e.img.bytes();
        }
    }
}

/// Decode `job.key.1`, plus the smallest level as a quick fallback for
/// next time. Empty on failure.
fn decode_job(job: &Job) -> Vec<(u8, Decoded)> {
    if let Some(make) = &job.make {
        return make().map(|l| vec![(job.key.1, Decoded::Still(Arc::new(l)))]).unwrap_or_default();
    }
    let (w, h) = job.size;
    let last = last_level(w, h);
    let level = job.key.1.min(last);
    let asset = job.key.0.as_str();
    if job.ext == "svg" {
        // Drawn at each level's size, so it is sharp at any zoom.
        let style = job.svg.unwrap_or_default();
        let make = |l: u8| {
            let (lw, lh) = level_size(w, h, l);
            crate::svg::rasterize(&job.bytes, &style, lw, lh).map(|px| Decoded::Still(Arc::new(ImageLevel::from_premultiplied(asset, lw, lh, px))))
        };
        let Some(first) = make(level) else { return vec![] };
        let mut out = vec![(job.key.1, first)];
        if level != last {
            out.extend(make(last).map(|d| (last, d)));
        }
        return out;
    }
    if job.ext == "gif" {
        if let Some(a) = decode_gif(asset, &job.bytes, (w, h), level) {
            return vec![(job.key.1, a)];
        }
    }
    let Ok(img) = image::load_from_memory(&job.bytes) else { return vec![] };
    let rgba = img.to_rgba8();
    drop(img);
    let (iw, ih) = (rgba.width(), rgba.height());
    let make = |l: u8| {
        let (lw, lh) = level_size(iw, ih, l);
        let px = if (lw, lh) == (iw, ih) {
            rgba.as_raw().clone()
        } else {
            image::imageops::resize(&rgba, lw, lh, image::imageops::FilterType::Triangle).into_raw()
        };
        Decoded::Still(Arc::new(ImageLevel::from_rgba(asset, lw, lh, px)))
    };
    let mut out = vec![(job.key.1, make(level))];
    if level != last {
        out.push((last, make(last)));
    }
    out
}

fn decode_gif(asset: &str, bytes: &[u8], (w, h): (u32, u32), level: u8) -> Option<Decoded> {
    use image::AnimationDecoder;
    let dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    // Step down levels until all frames fit the per-GIF budget. Frames
    // are counted lazily, so start from the requested level and check
    // as frames arrive.
    let last = last_level(w, h);
    let mut level = level;
    let mut frames: Vec<image::RgbaImage> = vec![];
    let mut ends = vec![];
    let mut t = 0.0;
    for f in dec.into_frames() {
        let Ok(f) = f else { break };
        let (num, den) = f.delay().numer_denom_ms();
        // Browsers treat delays under 20 ms as 100 ms.
        let d = if den == 0 { 0.1 } else { num as f64 / den as f64 / 1000.0 };
        t += if d < 0.02 { 0.1 } else { d };
        ends.push(t);
        let mut buf = f.into_buffer();
        let (lw, lh) = level_size(w, h, level);
        if (buf.width(), buf.height()) != (lw, lh) {
            buf = image::imageops::resize(&buf, lw, lh, image::imageops::FilterType::Triangle);
        }
        frames.push(buf);
        let used: usize = frames.iter().map(|b| b.as_raw().len()).sum();
        if used > GIF_BUDGET {
            if level >= last {
                break;
            }
            level += 1;
            let (lw, lh) = level_size(w, h, level);
            for b in frames.iter_mut() {
                *b = image::imageops::resize(b, lw, lh, image::imageops::FilterType::Triangle);
            }
        }
        if frames.len() >= GIF_MAX_FRAMES {
            break;
        }
    }
    ends.truncate(frames.len());
    if frames.is_empty() {
        return None;
    }
    let frames: Vec<Arc<ImageLevel>> =
        frames.into_iter().map(|b| Arc::new(ImageLevel::from_rgba(asset, b.width(), b.height(), b.into_raw()))).collect();
    if frames.len() == 1 {
        return Some(Decoded::Still(frames.into_iter().next().unwrap()));
    }
    Some(Decoded::Animated(Arc::new(Animation { frames, ends })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> AssetBytes {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner().into()
    }

    #[test]
    fn levels() {
        assert_eq!(level_size(4000, 3000, 0), (4000, 3000));
        assert_eq!(level_size(4000, 3000, 2), (1000, 750));
        assert_eq!(level_size(16384, 100, 0), (8192, 50));
        assert_eq!(level_for(4000, 3000, 900.0), 2);
        assert_eq!(level_for(4000, 3000, 1001.0), 1);
        assert_eq!(level_for(4000, 3000, 1e9), 0);
        assert_eq!(level_size(4000, 3000, last_level(4000, 3000)).0, 63);
    }

    #[test]
    fn budget_evicts_old_entries_not_current_ones() {
        let c = ImageCache::default();
        c.set_budget(3 * 512 * 512 * 4);
        let a = png(512, 512);
        for n in 0..6 {
            c.next_generation();
            let got = c.get(&format!("a{n}"), "png", &a, (512, 512), 0, true).unwrap();
            assert_eq!(got.width(), 512);
        }
        let s = c.stats();
        assert!(s.bytes <= 3 * 512 * 512 * 4 + 6 * 64 * 64 * 4, "{s:?}");
        // The newest is still there.
        c.next_generation();
        assert!(lock(&c.shared.inner).map.contains_key(&("a5".to_string(), 0)));
        assert!(!lock(&c.shared.inner).map.contains_key(&("a0".to_string(), 0)));
    }

    /// Speed fix 4: pictures nobody draws are given back after a while,
    /// except the small stand-in level.
    #[test]
    fn idle_pictures_are_dropped_and_drawn_ones_kept() {
        let c = ImageCache::default();
        let a = png(1024, 1024);
        for n in 0..4 {
            c.get(&format!("a{n}"), "png", &a, (1024, 1024), 0, true).unwrap();
        }
        let full = c.stats().bytes;
        assert!(full >= 4 * 1024 * 1024 * 4);
        // Nothing is idle yet.
        assert_eq!(c.trim_idle(std::time::Duration::from_secs(60)).0, 0);
        std::thread::sleep(std::time::Duration::from_millis(60));
        // One picture is drawn again; the others have been idle "long".
        c.next_generation();
        c.get("a3", "png", &a, (1024, 1024), 0, true).unwrap();
        let (freed, more) = c.trim_idle(std::time::Duration::from_millis(50));
        assert!(freed >= 3 * 1024 * 1024 * 4, "{freed}");
        assert!(more, "a3's full picture is still held");
        let i = lock(&c.shared.inner);
        assert!(i.map.contains_key(&("a3".to_string(), 0)));
        assert!(!i.map.contains_key(&("a0".to_string(), 0)));
        assert!(i.map.keys().any(|k| k.0 == "a0"), "the small level stays as a stand-in");
        assert_eq!(i.bytes, i.map.values().map(|e| e.img.bytes()).sum::<usize>());
    }

    #[test]
    fn async_decode_falls_back_then_wakes() {
        let c = ImageCache::default();
        let woke = Arc::new(AtomicUsize::new(0));
        let w2 = woke.clone();
        c.set_waker(Some(Arc::new(move || {
            w2.fetch_add(1, Ordering::SeqCst);
        })));
        let a = png(1024, 512);
        assert!(c.get("p", "png", &a, (1024, 512), 1, false).is_none());
        let t0 = std::time::Instant::now();
        while woke.load(Ordering::SeqCst) == 0 && t0.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(c.get("p", "png", &a, (1024, 512), 1, false).unwrap().width(), 512);
        // Another level: the cached one stands in while it decodes.
        assert_eq!(c.get("p", "png", &a, (1024, 512), 0, false).unwrap().width(), 512);
    }

    #[test]
    fn gif_frames_and_timing() {
        use image::codecs::gif::{GifEncoder, Repeat};
        use image::{Delay, Frame};
        let mut out = vec![];
        {
            let mut enc = GifEncoder::new(&mut out);
            enc.set_repeat(Repeat::Infinite).unwrap();
            for i in 0..3u8 {
                let img = image::RgbaImage::from_pixel(40, 30, image::Rgba([i * 80, 0, 0, 255]));
                enc.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(200, 1))).unwrap();
            }
        }
        let c = ImageCache::default();
        let Some(Decoded::Animated(a)) = c.get("g", "gif", &out.into(), (40, 30), 0, true) else { panic!("not animated") };
        assert_eq!(a.frames.len(), 3);
        assert!((a.total() - 0.6).abs() < 1e-9);
        let (f, next) = a.frame_at(0.25);
        assert!(Arc::ptr_eq(f, &a.frames[1]));
        assert!((next - 0.15).abs() < 1e-9);
        assert!(Arc::ptr_eq(a.frame_at(0.65).0, &a.frames[0]));
    }

    #[test]
    fn undecodable_is_remembered() {
        let c = ImageCache::default();
        let junk: AssetBytes = vec![1u8, 2, 3].into();
        assert!(c.get("j", "png", &junk, (10, 10), 0, true).is_none());
        assert!(c.is_failed("j"));
    }
}
