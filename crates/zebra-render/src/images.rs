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
//! Made pictures (tiles of a PDF page, drawn by the PDF helper, which may
//! take up to its deadline on a bad page) have a worker of their own: a
//! slow page never holds up the pictures, and never the UI thread.
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
///
/// The pixels are held once: vello_cpu's [`vello_cpu::Pixmap`] owns them,
/// and the [`ImageData`](vello_cpu::peniko::ImageData) the GPU draws from
/// shares that buffer instead of holding a second copy (a capture of an
/// 8000² picture held 512 MB, not 256).
pub struct ImageLevel {
    /// Asset key (for PDF export, which embeds the original file).
    pub asset: String,
    pub width: u32,
    pub height: u32,
    /// For vello (GPU). The blob id is stable, so vello's atlas caches it.
    pub data: vello_cpu::peniko::ImageData,
    /// For vello_cpu: the same pixels as `data`.
    pixmap: Arc<vello_cpu::Pixmap>,
    /// A frame of a moving picture (GIF, video): the next frame is the same
    /// picture, not a new one (see `GpuRenderer::needs_another_frame`).
    pub moving: bool,
}

/// A pixmap's pixels as the bytes of a peniko blob.
struct PixmapBytes(Arc<vello_cpu::Pixmap>);

impl AsRef<[u8]> for PixmapBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.data_as_u8_slice()
    }
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
    pub fn from_premultiplied(asset: &str, width: u32, height: u32, mut px: Vec<u8>) -> ImageLevel {
        use vello_cpu::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
        debug_assert_eq!(px.len() as u64, width as u64 * height as u64 * 4);
        let fits = width <= u16::MAX as u32 && height <= u16::MAX as u32 && px.len() as u64 == width as u64 * height as u64 * 4;
        let (pixmap, blob) = if fits {
            // The pixmap takes the buffer as 4-byte pixels, which needs a
            // capacity that is a multiple of 4.
            if !px.capacity().is_multiple_of(4) {
                px = px.as_slice().to_vec();
            }
            let meta = vello_cpu::PixelMetadata { alpha_type: ImageAlphaType::AlphaPremultiplied, may_have_transparency: true };
            let pixmap = Arc::new(vello_cpu::Pixmap::from_parts(px, width as u16, height as u16, meta));
            let blob = Blob::new(Arc::new(PixmapBytes(pixmap.clone())));
            (pixmap, blob)
        } else {
            // Bigger than vello_cpu can hold (decoders refuse these well
            // before): the CPU renderer draws nothing for it.
            (Arc::new(vello_cpu::Pixmap::new(1, 1)), Blob::from(px))
        };
        let data = ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::AlphaPremultiplied, width, height };
        ImageLevel { asset: asset.to_string(), width, height, data, pixmap, moving: false }
    }

    /// A video's frame (premultiplied RGBA8): marked moving, so a playing
    /// video's next frame is not a new picture to the GPU renderer.
    pub fn video_frame(asset: &str, width: u32, height: u32, px: Vec<u8>) -> ImageLevel {
        Self::from_premultiplied(asset, width, height, px).as_moving()
    }

    /// Marked as a frame of a moving picture ([`ImageLevel::moving`]).
    pub fn as_moving(self) -> ImageLevel {
        ImageLevel { moving: true, ..self }
    }

    /// The pixels for vello_cpu (shared with `data`, not a copy).
    pub fn pixmap(&self) -> Arc<vello_cpu::Pixmap> {
        self.pixmap.clone()
    }

    /// Bytes held.
    pub fn bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
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
    /// Pictures to decode (two workers).
    jobs: OnceLock<Mutex<Sender<Job>>>,
    /// Pictures to make (one worker: PDF pages are drawn one at a time).
    made: OnceLock<Mutex<Sender<Job>>>,
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
                made: OnceLock::new(),
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
        // Two decode workers: enough to keep up with a pan across a photo
        // deck without competing with the UI thread for cores.
        let (cell, name, workers) = if job.make.is_some() { (&self.shared.made, "zebra-pages", 1) } else { (&self.shared.jobs, "zebra-decode", 2) };
        let tx = cell.get_or_init(|| {
            let (tx, rx) = channel::<Job>();
            let rx = Arc::new(Mutex::new(rx));
            for n in 0..workers {
                let rx = rx.clone();
                let weak = Arc::downgrade(&self.shared);
                let _ = std::thread::Builder::new().name(format!("{name}-{n}")).spawn(move || {
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
/// next time. Empty on failure, a panic included: a decoder that panics
/// (tiny-skia did on hostile SVGs, audit T-C3) turns the picture into the
/// placeholder, and the worker thread lives on to decode the next one.
fn decode_job(job: &Job) -> Vec<(u8, Decoded)> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_job_inner(job))).unwrap_or_default()
}

fn decode_job_inner(job: &Job) -> Vec<(u8, Decoded)> {
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
            let (lw, lh) = svg_size(level_size(w, h, l));
            crate::svg::rasterize(&job.bytes, &style, lw, lh).map(|px| Decoded::Still(Arc::new(ImageLevel::from_premultiplied(asset, lw, lh, px))))
        };
        let Some(first) = make(level) else { return vec![] };
        let mut out = vec![(job.key.1, first)];
        if level != last {
            out.extend(make(last).map(|d| (last, d)));
        }
        return out;
    }
    if job.bytes.starts_with(b"GIF8") {
        // GIFs go through their own decoder only: image's would allocate
        // the whole logical screen once per frame.
        return decode_gif(asset, &job.bytes, level).map(|a| vec![(job.key.1, a)]).unwrap_or_default();
    }
    decode_still(asset, &job.bytes, job.key.1)
}

/// A still picture at `want` (and its smallest level), sized from its
/// header before anything is decoded, and never held at full size when it
/// is big (the rules are `zebra_core::media::PictureHeader::fits`):
///
/// * a PNG streams row by row into the levels; an interlaced one over
///   [`MAX_WHOLE_PIXELS`] streams its first pass (every 8th pixel);
/// * on the Mac, a big JPEG, and anything else Zebra can't stream or
///   decode whole (a huge progressive JPEG or WebP), is drawn by ImageIO
///   at the level's size, as Keynote draws pictures;
/// * the rest decode whole under [`decode_limits`] and shrink from their
///   own pixel format, with no RGBA copy and no clone of level 0.
///
/// A level can come out a little smaller than [`level_size`] when the
/// picture only gives that much (a scaled JPEG, a PNG's first pass).
fn decode_still(asset: &str, bytes: &[u8], want: u8) -> Vec<(u8, Decoded)> {
    use zebra_core::media::PictureFormat;
    let Some(head) = zebra_core::media::picture_header(&bytes[..bytes.len().min(256 << 10)]).filter(|h| h.fits()) else { return vec![] };
    let (iw, ih) = (head.width, head.height);
    let last = last_level(iw, ih);
    let level = want.min(last);
    let (lw, lh) = level_size(iw, ih, level);
    let (sw, sh) = level_size(iw, ih, last);
    let out_bytes = (lw as u64 * lh as u64 + sw as u64 * sh as u64) * 4;
    // Shrinks from a `w`x`h` source to the levels.
    let shrinks = |w: u32, h: u32| {
        let mut v = vec![Shrink::new(w, h, lw, lh)];
        if level != last {
            v.push(Shrink::new(w, h, sw, sh));
        }
        v
    };
    let pixels = head.pixels();
    let mut done = None;
    if head.format == PictureFormat::Png {
        let _permit = DecodeBytes::acquire(out_bytes + iw as u64 * 16);
        done = stream_png(bytes, &shrinks, pixels > MAX_WHOLE_PIXELS);
    } else if head.format == PictureFormat::Jpeg && (pixels > SCALED_JPEG_FROM || iw.max(ih) > WHOLE_JPEG_SIDE) || !head.streams_or_decodes_whole() {
        // On the Mac, ImageIO draws it at the level's size, as Keynote
        // does (a JPEG straight from its DCT at a smaller size). Elsewhere
        // (development builds) it decodes whole if it can.
        #[cfg(target_os = "macos")]
        {
            let _permit = DecodeBytes::acquire(out_bytes * 2);
            let Some((w, h, px)) = crate::imageio::thumbnail(bytes, lw.max(lh), MAX_WHOLE_DECODE_ALLOC) else { return vec![] };
            let mut out = shrinks(w, h);
            for row in px.chunks_exact(w as usize * 4) {
                for s in out.iter_mut() {
                    s.push(row, 4);
                }
            }
            done = Some(out);
        }
    }
    let shrinks = match done {
        Some(s) => s,
        None => {
            if pixels > MAX_WHOLE_PIXELS {
                return vec![];
            }
            let mut out = shrinks(iw, ih);
            let _permit = DecodeBytes::acquire(out_bytes + pixels * 4);
            let Some(img) = decode_limited(bytes) else { return vec![] };
            if (img.width(), img.height()) != (iw, ih) {
                return vec![];
            }
            let (raw, channels): (std::borrow::Cow<[u8]>, usize) = match &img {
                image::DynamicImage::ImageLuma8(b) => (b.as_raw().into(), 1),
                image::DynamicImage::ImageLumaA8(b) => (b.as_raw().into(), 2),
                image::DynamicImage::ImageRgb8(b) => (b.as_raw().into(), 3),
                image::DynamicImage::ImageRgba8(b) => (b.as_raw().into(), 4),
                other => (other.to_rgba8().into_raw().into(), 4),
            };
            for row in raw.chunks_exact(iw as usize * channels) {
                for s in out.iter_mut() {
                    s.push(row, channels);
                }
            }
            out
        }
    };
    let mut it = shrinks.into_iter();
    let mut out = vec![];
    if let Some(s) = it.next() {
        out.push((want, Decoded::Still(Arc::new(s.finish(asset)))));
    }
    if let Some(s) = it.next() {
        out.push((last, Decoded::Still(Arc::new(s.finish(asset)))));
    }
    out
}

/// Largest picture decoded whole (pixels); see `zebra_core::media`.
pub use zebra_core::media::MAX_WHOLE_PIXELS;
/// JPEGs over this many pixels are drawn by ImageIO at the level's size
/// on the Mac; smaller ones decode whole, which is faster. 64 MP is 192 MB
/// as RGB.
pub const SCALED_JPEG_FROM: u64 = 64 << 20;
/// JPEGs wider or taller than this go to ImageIO too.
pub const WHOLE_JPEG_SIDE: u32 = 16384;

/// Feed a PNG's rows to the shrinks `make` gives for the size streamed.
/// A non-interlaced PNG streams every row. An interlaced one decodes whole
/// (8 bits a sample) unless `first_pass_only`: then only its first Adam7
/// pass is read, every 8th pixel each way, so a huge one stays small.
/// None if it is broken, so the caller can decode it whole instead.
fn stream_png(bytes: &[u8], make: &dyn Fn(u32, u32) -> Vec<Shrink>, first_pass_only: bool) -> Option<Vec<Shrink>> {
    let mut d = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: MAX_DECODE_ALLOC as usize });
    d.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = d.read_info().ok()?;
    let (w, h) = (r.info().width, r.info().height);
    let channels = match r.output_color_type() {
        (png::ColorType::Grayscale, png::BitDepth::Eight) => 1,
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => 2,
        (png::ColorType::Rgb, png::BitDepth::Eight) => 3,
        (png::ColorType::Rgba, png::BitDepth::Eight) => 4,
        _ => return None,
    };
    if !r.info().interlaced {
        let mut shrinks = make(w, h);
        // A truncated file keeps the rows it has; the rest stays clear.
        while let Ok(Some(row)) = r.next_row() {
            for s in shrinks.iter_mut() {
                s.push(row.data(), channels);
            }
        }
        return Some(shrinks);
    }
    if first_pass_only {
        // Pass 1 comes first: pixels (8i, 8j), ceil(w/8) by ceil(h/8).
        let (pw, ph) = (w.div_ceil(8), h.div_ceil(8));
        let mut shrinks = make(pw, ph);
        for _ in 0..ph {
            let Ok(Some(row)) = r.next_row() else { break };
            if row.data().len() < pw as usize * channels {
                break;
            }
            for s in shrinks.iter_mut() {
                s.push(&row.data()[..pw as usize * channels], channels);
            }
        }
        return Some(shrinks);
    }
    let size = w as u64 * h as u64 * channels as u64;
    if size > MAX_WHOLE_DECODE_ALLOC {
        return None;
    }
    let _permit = DecodeBytes::acquire(size);
    let mut buf = vec![0u8; size as usize];
    r.next_frame(&mut buf).ok()?;
    let mut shrinks = make(w, h);
    for row in buf.chunks_exact(w as usize * channels) {
        for s in shrinks.iter_mut() {
            s.push(row, channels);
        }
    }
    Some(shrinks)
}


/// Most bytes all decodes in flight may be estimated to need at once
/// (header size times the pixel formats, plus the levels made). One
/// decode bigger than this runs alone.
pub const DECODE_BYTES: u64 = 512 << 20;

/// A share of [`DECODE_BYTES`], given back on drop.
struct DecodeBytes(u64);

static IN_FLIGHT: Mutex<u64> = Mutex::new(0);
static FREED: std::sync::Condvar = std::sync::Condvar::new();

impl DecodeBytes {
    /// Wait until `n` bytes fit beside the decodes already running.
    fn acquire(n: u64) -> DecodeBytes {
        let n = n.min(DECODE_BYTES);
        let mut g = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        while *g > 0 && *g + n > DECODE_BYTES {
            g = FREED.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        *g += n;
        DecodeBytes(n)
    }
}

impl Drop for DecodeBytes {
    fn drop(&mut self) {
        let mut g = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        *g = g.saturating_sub(self.0);
        FREED.notify_all();
    }
}

/// A level's size, scaled to fit [`crate::svg::MAX_RASTER_SIDE`].
fn svg_size((w, h): (u32, u32)) -> (u32, u32) {
    let m = w.max(h);
    let cap = crate::svg::MAX_RASTER_SIDE;
    if m <= cap {
        return (w, h);
    }
    let k = cap as f64 / m as f64;
    (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
}

/// Most bytes one decoder may allocate for its pixels.
pub const MAX_DECODE_ALLOC: u64 = 256 << 20;
/// Most screen pixels a GIF's frames may cover in all (screen area times
/// frames kept): each frame is composited on a full screen, so this bounds
/// the time a GIF takes. 1920x1080 keeps 517 frames.
pub const GIF_WORK: u64 = 1 << 30;

/// Most bytes a JPEG or WebP decode may allocate: they decode whole, and a
/// 100-megapixel camera JPEG (11648x8736 RGB) is 305 MB. This is image's
/// own default; [`DECODE_BYTES`] lets only one such decode run at a time.
pub const MAX_WHOLE_DECODE_ALLOC: u64 = 512 << 20;

/// The limits every `image` decoder runs under.
pub fn decode_limits() -> image::Limits {
    // No limit on the sides: what is decoded whole is bounded by its
    // pixels (`MAX_WHOLE_PIXELS`, checked from the header first) and by
    // the allocation.
    let mut l = image::Limits::default();
    l.max_image_width = None;
    l.max_image_height = None;
    l.max_alloc = Some(MAX_WHOLE_DECODE_ALLOC);
    l
}

/// Decode a still picture under [`decode_limits`]. None if it is
/// unreadable or too big.
fn decode_limited(bytes: &[u8]) -> Option<image::DynamicImage> {
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    r.limits(decode_limits());
    r.decode().ok()
}

/// Shrinks a picture to `dw`x`dh` one source row at a time (area average,
/// premultiplied), so the full-size picture never has to exist at once.
struct Shrink {
    sw: u32,
    sh: u32,
    dw: u32,
    dh: u32,
    /// Destination column of each source column.
    col: Vec<u32>,
    /// Source columns per destination column.
    ncol: Vec<u32>,
    acc: Vec<u64>,
    /// Source rows summed into `acc`, and the destination row they make.
    nrow: u32,
    oy: u32,
    y: u32,
    out: Vec<u8>,
}

impl Shrink {
    fn new(sw: u32, sh: u32, dw: u32, dh: u32) -> Shrink {
        let (dw, dh) = (dw.clamp(1, sw.max(1)), dh.clamp(1, sh.max(1)));
        let col: Vec<u32> = (0..sw).map(|x| (x as u64 * dw as u64 / sw as u64) as u32).collect();
        let mut ncol = vec![0u32; dw as usize];
        for &c in &col {
            ncol[c as usize] += 1;
        }
        Shrink { sw, sh, dw, dh, col, ncol, acc: vec![0; dw as usize * 4], nrow: 0, oy: 0, y: 0, out: Vec::with_capacity(dw as usize * dh as usize * 4) }
    }

    /// The next source row: `channels` bytes per pixel (1 grey, 2 grey and
    /// alpha, 3 RGB, 4 RGBA), straight alpha.
    fn push(&mut self, row: &[u8], channels: usize) {
        if self.y >= self.sh {
            return;
        }
        let oy = (self.y as u64 * self.dh as u64 / self.sh as u64) as u32;
        if oy != self.oy && self.nrow > 0 {
            self.flush();
            self.oy = oy;
        }
        let acc = &mut self.acc;
        for (px, &c) in row.chunks_exact(channels).zip(self.col.iter()) {
            let (r, g, b, a) = match channels {
                1 => (px[0], px[0], px[0], 255),
                2 => (px[0], px[0], px[0], px[1]),
                3 => (px[0], px[1], px[2], 255),
                _ => (px[0], px[1], px[2], px[3]),
            };
            let i = c as usize * 4;
            let a32 = a as u32;
            if a == 255 {
                acc[i] += r as u64;
                acc[i + 1] += g as u64;
                acc[i + 2] += b as u64;
            } else if a != 0 {
                acc[i] += ((r as u32 * a32 + 127) / 255) as u64;
                acc[i + 1] += ((g as u32 * a32 + 127) / 255) as u64;
                acc[i + 2] += ((b as u32 * a32 + 127) / 255) as u64;
            }
            acc[i + 3] += a as u64;
        }
        self.nrow += 1;
        self.y += 1;
        if self.y == self.sh {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.nrow == 0 {
            return;
        }
        for (x, &n) in self.ncol.iter().enumerate() {
            let n = (n as u64 * self.nrow as u64).max(1);
            for c in 0..4 {
                let v = &mut self.acc[x * 4 + c];
                self.out.push(((*v + n / 2) / n).min(255) as u8);
                *v = 0;
            }
        }
        self.nrow = 0;
    }

    /// Premultiplied RGBA8, `dw`x`dh` (rows never pushed stay clear).
    fn finish(mut self, asset: &str) -> ImageLevel {
        self.flush();
        self.out.resize(self.dw as usize * self.dh as usize * 4, 0);
        let _ = self.sw;
        ImageLevel::from_premultiplied(asset, self.dw, self.dh, self.out)
    }
}

/// An animated (or still) GIF at `level` of its logical screen, decoded
/// with the `gif` crate: one screen-sized canvas is reused for every
/// frame, and the screen size and frame count are checked against the
/// limits before any pixels are decoded. None if it is unreadable or too
/// big.
fn decode_gif(asset: &str, bytes: &[u8], level: u8) -> Option<Decoded> {
    let options = || {
        let mut o = gif::DecodeOptions::new();
        o.set_color_output(gif::ColorOutput::RGBA);
        o.set_memory_limit(gif::MemoryLimit::Bytes(std::num::NonZeroU64::new(MAX_DECODE_ALLOC).unwrap_or(std::num::NonZeroU64::MIN)));
        o
    };
    // The header: the logical screen everything is drawn on.
    let mut dec = options().read_info(std::io::Cursor::new(bytes)).ok()?;
    let (w, h) = (dec.width() as u32, dec.height() as u32);
    let area = w as u64 * h as u64;
    if w == 0 || h == 0 || area > zebra_core::media::MAX_GIF_PIXELS || area * 4 > MAX_DECODE_ALLOC {
        return None;
    }
    // Count frames without decoding them (their compressed data is only
    // copied), so the level is chosen before any pixels exist.
    let count = {
        let mut o = options();
        o.skip_frame_decoding(true);
        let mut d = o.read_info(std::io::Cursor::new(bytes)).ok()?;
        let mut n = 0usize;
        while n < GIF_MAX_FRAMES && matches!(d.read_next_frame(), Ok(Some(_))) {
            n += 1;
        }
        n
    };
    let mut keep = count.min((GIF_WORK / area).max(1) as usize);
    if keep == 0 {
        return None;
    }
    // Step down levels until all frames kept fit the per-GIF budget; at
    // the smallest level, keep fewer frames instead.
    let last = last_level(w, h);
    let mut level = level.min(last);
    let level_bytes = |l: u8| {
        let (lw, lh) = level_size(w, h, l);
        lw as usize * lh as usize * 4
    };
    while level < last && keep * level_bytes(level) > GIF_BUDGET {
        level += 1;
    }
    keep = keep.min((GIF_BUDGET / level_bytes(level)).max(1));
    let (lw, lh) = level_size(w, h, level);

    let (cw, ch) = (w as usize, h as usize);
    let _permit = DecodeBytes::acquire(area * 8 + (keep * level_bytes(level)) as u64);
    let mut canvas = vec![0u8; cw * ch * 4];
    let mut saved: Vec<u8> = vec![];
    let mut frames: Vec<Arc<ImageLevel>> = vec![];
    let mut ends = vec![];
    let mut t = 0.0;
    while frames.len() < keep {
        let f = match dec.read_next_frame() {
            Ok(Some(f)) => f,
            _ => break,
        };
        // The frame's rectangle, clipped to the screen.
        let (fx, fy, fw, fh) = (f.left as usize, f.top as usize, f.width as usize, f.height as usize);
        let x0 = fx.min(cw);
        let x1 = (fx + fw).min(cw);
        let y0 = fy.min(ch);
        let y1 = (fy + fh).min(ch);
        if f.buffer.len() < fw * fh * 4 {
            break;
        }
        let dispose = f.dispose;
        if dispose == gif::DisposalMethod::Previous {
            saved.clear();
            for y in y0..y1 {
                saved.extend_from_slice(&canvas[(y * cw + x0) * 4..(y * cw + x1) * 4]);
            }
        }
        for y in y0..y1 {
            let src = &f.buffer[((y - fy) * fw + (x0 - fx)) * 4..((y - fy) * fw + (x1 - fx)) * 4];
            let dst = &mut canvas[(y * cw + x0) * 4..(y * cw + x1) * 4];
            for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                if s[3] != 0 {
                    d.copy_from_slice(s);
                }
            }
        }
        // Browsers treat delays under 20 ms as 100 ms.
        let d = f.delay as f64 / 100.0;
        t += if d < 0.02 { 0.1 } else { d };
        ends.push(t);
        let mut s = Shrink::new(w, h, lw, lh);
        for row in canvas.chunks_exact(cw * 4) {
            s.push(row, 4);
        }
        frames.push(Arc::new(s.finish(asset).as_moving()));
        match dispose {
            gif::DisposalMethod::Background => {
                for y in y0..y1 {
                    canvas[(y * cw + x0) * 4..(y * cw + x1) * 4].fill(0);
                }
            }
            gif::DisposalMethod::Previous => {
                let n = (x1 - x0) * 4;
                for (i, y) in (y0..y1).enumerate() {
                    canvas[(y * cw + x0) * 4..(y * cw + x1) * 4].copy_from_slice(&saved[i * n..(i + 1) * n]);
                }
            }
            _ => {}
        }
    }
    ends.truncate(frames.len());
    if frames.len() == 1 {
        return frames.pop().map(Decoded::Still);
    }
    if frames.is_empty() {
        return None;
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
    fn svgs_are_drawn_at_most_4096_wide() {
        assert_eq!(svg_size((8192, 4096)), (4096, 2048));
        assert_eq!(svg_size((300, 200)), (300, 200));
    }

    #[test]
    fn camera_sized_jpeg_fits_the_limits() {
        let l = decode_limits();
        assert!(l.max_alloc.unwrap() >= 11648 * 8736 * 3, "a 100 MP JPEG must decode");
        assert!(l.max_alloc.unwrap() >= MAX_WHOLE_PIXELS * 4);
        assert_eq!(l.max_image_width, None);
    }

    /// Audit T-C3: a panicking decode used to end its worker thread and
    /// leave the picture pending for good; two of them and nothing decoded
    /// again until restart.
    #[test]
    fn a_panicking_decode_is_a_placeholder_and_workers_live_on() {
        let c = ImageCache::default();
        let woke = Arc::new(AtomicUsize::new(0));
        let w2 = woke.clone();
        c.set_waker(Some(Arc::new(move || {
            w2.fetch_add(1, Ordering::SeqCst);
        })));
        let wait = |n: usize| {
            let t0 = std::time::Instant::now();
            while woke.load(Ordering::SeqCst) < n && t0.elapsed().as_secs() < 10 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        // More panics than there are workers.
        for i in 0..3 {
            let boom: Make = Arc::new(|| panic!("decoder bug"));
            assert!(c.get_made(&format!("tile:boom{i}"), false, boom).is_none());
        }
        wait(3);
        for i in 0..3 {
            assert!(c.is_failed(&format!("tile:boom{i}")));
        }
        let fine: Make = Arc::new(|| Some(ImageLevel::from_rgba("x", 1, 1, vec![1, 2, 3, 255])));
        assert!(c.get_made("tile:fine", false, fine.clone()).is_none());
        wait(4);
        assert!(c.peek("tile:fine").is_some(), "a worker is still decoding");
    }

    /// A PDF page whose helper hangs (up to its deadline) holds up
    /// neither the pictures nor whoever asked for the tile.
    #[test]
    fn a_hung_tile_holds_up_neither_pictures_nor_the_caller() {
        let c = ImageCache::default();
        let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
        for i in 0..4 {
            let go = go.clone();
            let hung: Make = Arc::new(move || {
                while !go.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Some(ImageLevel::from_rgba("t", 1, 1, vec![0, 0, 0, 255]))
            });
            let t0 = std::time::Instant::now();
            assert!(c.get_made(&format!("tile:hung{i}"), false, hung).is_none());
            assert!(t0.elapsed().as_millis() < 100, "asking for a tile waited");
        }
        let a = png(256, 256);
        assert!(c.get("photo", "png", &a, (256, 256), 0, false).is_none());
        let t0 = std::time::Instant::now();
        while c.get("photo", "png", &a, (256, 256), 0, false).is_none() {
            assert!(t0.elapsed().as_secs() < 10, "a picture waited behind a hung tile");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        go.store(true, Ordering::SeqCst);
        let t0 = std::time::Instant::now();
        while c.peek("tile:hung3").is_none() {
            assert!(t0.elapsed().as_secs() < 10, "the tiles never landed");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The pixel at (x, y) of tests/data/interlaced-61x37.png.
    fn fixture_px(x: u32, y: u32) -> [u8; 3] {
        [(x * 4) as u8, (y * 6) as u8, ((x + y) * 3) as u8]
    }

    /// An interlaced PNG too big to decode whole is drawn from its first
    /// Adam7 pass: every 8th pixel each way, exactly.
    #[test]
    fn interlaced_png_first_pass_is_every_8th_pixel() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/interlaced-61x37.png")).unwrap();
        let one = |w, h| vec![Shrink::new(w, h, w, h)];
        let s = stream_png(&bytes, &one, true).unwrap().pop().unwrap().finish("i");
        assert_eq!((s.width, s.height), (8, 5));
        let px = s.data.data.data();
        for (j, y) in (0..37).step_by(8).enumerate() {
            for (i, x) in (0..61).step_by(8).enumerate() {
                let at = (j * 8 + i) * 4;
                assert_eq!(px[at..at + 3], fixture_px(x, y), "({x}, {y})");
            }
        }
        // Small enough to decode whole: every pixel.
        let s = stream_png(&bytes, &one, false).unwrap().pop().unwrap().finish("i");
        assert_eq!((s.width, s.height), (61, 37));
        let at = (36 * 61 + 60) * 4;
        assert_eq!(s.data.data.data()[at..at + 3], fixture_px(60, 36));
        // And through the cache.
        let c = ImageCache::default();
        let got = c.get("i", "png", &bytes.into(), (61, 37), 0, true).unwrap();
        assert_eq!(got.width(), 61);
    }

    /// Wider than 16,384 used to be the placeholder; now it streams.
    #[test]
    fn a_png_wider_than_16384_streams() {
        let mut out = vec![];
        {
            let mut e = png::Encoder::new(&mut out, 20000, 30);
            e.set_color(png::ColorType::Rgb);
            let mut w = e.write_header().unwrap();
            let row: Vec<u8> = (0..20000u32).flat_map(|x| [if x < 10000 { 200 } else { 20 }, 100, 50]).collect();
            let mut s = w.stream_writer().unwrap();
            for _ in 0..30 {
                std::io::Write::write_all(&mut s, &row).unwrap();
            }
            s.finish().unwrap();
        }
        let c = ImageCache::default();
        let Some(Decoded::Still(l)) = c.get("w", "png", &out.into(), (20000, 30), 0, true) else { panic!("not decoded") };
        assert_eq!((l.width, l.height), level_size(20000, 30, 0));
        let px = l.data.data.data();
        assert_eq!(px[..4], [200, 100, 50, 255]);
        assert_eq!(px[px.len() - 4..], [20, 100, 50, 255]);
    }

    #[test]
    fn undecodable_is_remembered() {
        let c = ImageCache::default();
        let junk: AssetBytes = vec![1u8, 2, 3].into();
        assert!(c.get("j", "png", &junk, (10, 10), 0, true).is_none());
        assert!(c.is_failed("j"));
    }
}
