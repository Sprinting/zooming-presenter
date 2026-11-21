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
//! Animated GIFs: a short one decodes every frame at the chosen level once
//! and keeps them ([`GIF_BUDGET`]); a longer or bigger one is decoded as it
//! plays, about a second of frames ahead, on the same workers (B4). Either
//! way every frame plays, whatever the GIF's length.
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
pub struct Animation {
    /// End time of each frame, seconds, cumulative.
    pub ends: Vec<f64>,
    /// The first frame (thumbnails, paths, anything not playing).
    pub first: Arc<ImageLevel>,
    frames: Frames,
}

enum Frames {
    /// Every frame, decoded once and kept (a short GIF).
    All(Vec<Arc<ImageLevel>>),
    /// Decoded as it plays (a long or big GIF): see [`Player`].
    Played(Player),
}

/// A GIF decoded as it plays, like a video: its decoder and the screen it
/// composites on, and about a second of frames decoded ahead at this
/// level. Nothing it holds grows with the GIF's length.
struct Player {
    asset: String,
    bytes: AssetBytes,
    /// The level's size.
    size: (u32, u32),
    /// Bytes of the GIF's screen (and its "restore previous" copy).
    screen_bytes: usize,
    state: Mutex<PlayState>,
    /// Bytes held now (for the cache's budget).
    held: AtomicUsize,
}

#[derive(Default)]
struct PlayState {
    /// None before it starts, after a rest, or while a worker has it.
    screen: Option<Screen>,
    /// A worker is decoding on.
    busy: bool,
    /// The frame the next worker run starts from.
    want: usize,
    /// Frames decoded and not yet shown, in the order decoded.
    queue: std::collections::VecDeque<(usize, Arc<ImageLevel>)>,
    /// The frame shown last.
    shown: Option<(usize, Arc<ImageLevel>)>,
    /// No frame past the first can be decoded.
    broken: bool,
}

impl std::fmt::Debug for Animation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Animation({} frames, {})", self.ends.len(), if self.is_played() { "played" } else { "kept" })
    }
}

impl Animation {
    pub fn total(&self) -> f64 {
        self.ends.last().copied().unwrap_or(0.0)
    }

    /// Frames in the GIF.
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Whether it is decoded as it plays (not kept whole).
    pub fn is_played(&self) -> bool {
        matches!(self.frames, Frames::Played(_))
    }

    /// The frame showing at time `t` (looping) and seconds until the next.
    pub fn index_at(&self, t: f64) -> (usize, f64) {
        let total = self.total();
        if total <= 0.0 || self.ends.len() < 2 || !t.is_finite() {
            return (0, f64::INFINITY);
        }
        let t = t.rem_euclid(total);
        let i = self.ends.partition_point(|e| *e <= t).min(self.ends.len() - 1);
        (i, self.ends[i] - t)
    }

    /// Bytes it holds now.
    pub fn bytes(&self) -> usize {
        match &self.frames {
            Frames::All(f) => f.iter().map(|f| f.bytes()).sum(),
            Frames::Played(p) => p.held.load(Ordering::Relaxed),
        }
    }

    /// Frames decoded ahead of frame `i`: about [`AHEAD_SECONDS`] of them,
    /// within [`AHEAD_BYTES`], at least one, never a whole loop.
    fn ahead(&self, i: usize, frame_bytes: usize) -> usize {
        let n = self.ends.len();
        let most = (AHEAD_BYTES / frame_bytes.max(1)).max(2).min(n.saturating_sub(1)).max(1);
        let dur = |k: usize| self.ends[k] - if k == 0 { 0.0 } else { self.ends[k - 1] };
        let (mut k, mut t) = (1, dur(i % n));
        while k < most && t < AHEAD_SECONDS {
            t += dur((i + k) % n);
            k += 1;
        }
        k
    }

    /// Frame `i`, and whether a worker should decode on. See
    /// [`ImageCache::frame`].
    fn frame(&self, i: usize, sync: bool) -> (Arc<ImageLevel>, bool) {
        let p = match &self.frames {
            Frames::All(f) => return (f[i.min(f.len() - 1)].clone(), false),
            Frames::Played(p) => p,
        };
        let n = self.ends.len();
        let ahead = self.ahead(i, p.frame_bytes());
        let mut s = lock_play(&p.state);
        // How far frame `k` is past `i`; within `ahead` it is still to
        // come, else it has been passed.
        let fwd = |k: usize| (k + n - i % n) % n;
        if let Some(pos) = s.queue.iter().position(|(k, _)| *k == i) {
            // Everything decoded before it has been passed.
            let f = s.queue.drain(..=pos).last().map(|(_, f)| f);
            s.shown = f.map(|f| (i, f));
        } else if s.shown.as_ref().map(|(k, _)| *k) != Some(i) {
            // The newest frame decoded before `i`, if a worker is behind.
            let passed = |k: usize| fwd(k) >= ahead;
            if let Some(pos) = s.queue.iter().rposition(|(k, _)| passed(*k)) {
                let f = s.queue.drain(..=pos).last();
                s.shown = f;
            }
            if sync && !s.broken {
                drop(s);
                let f = p.decode_now(i);
                s = lock_play(&p.state);
                if let Some(f) = f {
                    s.shown = Some((i, f));
                }
            }
        }
        let shown = s.shown.as_ref().map(|(_, f)| f.clone()).unwrap_or_else(|| self.first.clone());
        let to_come = s.queue.iter().filter(|(k, _)| (1..ahead).contains(&fwd(*k))).count();
        let work = !sync && !s.busy && !s.broken && to_come + 1 < ahead.max(2);
        if work {
            s.busy = true;
            s.want = i;
        }
        p.recount(&s);
        (shown, work)
    }

    /// Decode on from the frame asked for last, about a second ahead (on
    /// a picture worker).
    fn work(&self) {
        let Frames::Played(p) = &self.frames else { return };
        let n = self.ends.len();
        let (screen, want) = {
            let mut s = lock_play(&p.state);
            (s.screen.take(), s.want)
        };
        let ahead = self.ahead(want, p.frame_bytes());
        let mut screen = screen;
        let mut made = 0;
        // Where the screen is: within the frames wanted, before them in
        // this loop, or past them (start again).
        let plan = |pos: usize| -> (bool, usize) {
            let past_want = (pos + n - want) % n;
            if pos < n && past_want < ahead {
                (false, ahead - past_want)
            } else if pos <= want {
                (false, ahead)
            } else {
                (true, ahead)
            }
        };
        let mut todo = match &screen {
            Some(sc) => {
                let (restart, k) = plan(sc.pos);
                if restart {
                    screen = None;
                }
                k
            }
            None => ahead,
        };
        let mut restarts = 0;
        while todo > 0 {
            let sc = match &mut screen {
                Some(sc) => sc,
                None => match Screen::open(&p.bytes) {
                    Some(sc) => screen.insert(sc),
                    None => break,
                },
            };
            let k = sc.pos;
            let wanted = (k + n - want) % n < ahead;
            match sc.step(wanted.then_some((p.asset.as_str(), p.size.0, p.size.1))) {
                Some(f) => {
                    if let Some(f) = f {
                        let mut s = lock_play(&p.state);
                        s.queue.push_back((k, Arc::new(f.as_moving())));
                        p.recount(&s);
                        todo -= 1;
                        made += 1;
                    }
                }
                None => {
                    // The end: loop.
                    restarts += 1;
                    let empty = sc.pos == 0;
                    screen = None;
                    if empty || restarts > 2 {
                        break;
                    }
                }
            }
        }
        let mut s = lock_play(&p.state);
        if made == 0 {
            // Nothing past what was shown decodes: stay on it.
            s.broken = true;
        }
        s.screen = screen;
        s.busy = false;
        p.recount(&s);
    }

    /// Give back the screen and the frames ahead (nobody has drawn this
    /// GIF for a while); it starts again when it is drawn.
    fn rest(&self) {
        let Frames::Played(p) = &self.frames else { return };
        let mut s = lock_play(&p.state);
        if !s.busy {
            s.screen = None;
        }
        s.queue.clear();
        s.shown = None;
        p.recount(&s);
    }
}

fn lock_play(m: &Mutex<PlayState>) -> std::sync::MutexGuard<'_, PlayState> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Player {
    fn frame_bytes(&self) -> usize {
        self.size.0 as usize * self.size.1 as usize * 4
    }

    fn recount(&self, s: &PlayState) {
        let screen = if s.screen.is_some() || s.busy { self.screen_bytes } else { 0 };
        let frames: usize = s.queue.iter().map(|(_, f)| f.bytes()).sum::<usize>() + s.shown.as_ref().map_or(0, |(_, f)| f.bytes());
        self.held.store(self.frame_bytes() + screen + frames, Ordering::Relaxed);
    }

    /// Frame `i` decoded on this thread (captures and export): on from the
    /// player's screen when it is before `i` and free, else from the start.
    fn decode_now(&self, i: usize) -> Option<Arc<ImageLevel>> {
        // The player's own screen when no worker has it (started again if
        // it is past `i`), so frames asked for in turn cost one step each.
        let shared = {
            let mut s = lock_play(&self.state);
            if s.busy {
                None
            } else {
                s.busy = true;
                Some(s.screen.take())
            }
        };
        let is_shared = shared.is_some();
        let sc = match shared.flatten() {
            Some(sc) if sc.pos <= i => Some(sc),
            _ => Screen::open(&self.bytes),
        };
        let mut out = None;
        let sc = sc.map(|mut sc| {
            while sc.pos <= i {
                let keep = sc.pos == i;
                match sc.step(keep.then_some((self.asset.as_str(), self.size.0, self.size.1))) {
                    Some(Some(f)) => out = Some(Arc::new(f.as_moving())),
                    Some(None) => {}
                    None => break,
                }
            }
            sc
        });
        if is_shared {
            let mut s = lock_play(&self.state);
            s.screen = sc;
            s.busy = false;
            self.recount(&s);
        }
        out
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
            Decoded::Animated(a) => a.first.width,
        }
    }
}

/// Largest dimension of level 0.
pub const MAX_IMAGE_DIM: u32 = 8192;
/// Levels stop once the picture is this small.
const MIN_LEVEL_DIM: u32 = 64;
/// Default budget for all decoded pictures.
pub const DEFAULT_BUDGET: usize = 256 << 20;
/// A GIF whose frames all fit in this many bytes at the level drawn (and
/// [`GIF_WORK`]) is decoded once and kept; a bigger one is decoded as it
/// plays. Not a limit: every frame plays either way.
pub const GIF_BUDGET: usize = 48 << 20;
/// About this much of a GIF played as it decodes is decoded ahead.
const AHEAD_SECONDS: f64 = 1.0;
/// At most this many bytes of frames are decoded ahead (at least one).
const AHEAD_BYTES: usize = 32 << 20;

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
    /// Other work for the picture workers (decoding a playing GIF on).
    task: Option<Box<dyn FnOnce() + Send>>,
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
        for e in i.map.values().filter(|e| now.duration_since(e.at) >= idle) {
            if let Decoded::Animated(a) = &e.img {
                a.rest();
            }
        }
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
                    self.enqueue(Job { key, ext: ext.to_string(), bytes: bytes.clone(), size, svg, make: None, task: None });
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
        let out = decode_job(&Job { key: key.clone(), ext: ext.to_string(), bytes: bytes.clone(), size, svg, make: None, task: None });
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
                    self.enqueue(Job { key, ext: String::new(), bytes: AssetBytes::from(Vec::new()), size: (0, 0), svg: None, make: Some(make), task: None });
                }
                return None;
            }
        }
        let out = decode_job(&Job { key: key.clone(), ext: String::new(), bytes: AssetBytes::from(Vec::new()), size: (0, 0), svg: None, make: Some(make), task: None });
        insert(&self.shared, key, out, g)
    }

    /// A handle for work that makes several pictures at once.
    pub fn handle(&self) -> CacheHandle {
        CacheHandle(Arc::downgrade(&self.shared))
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

    /// The frame of `anim` showing at time `t` (it loops) and the seconds
    /// until the next one. A GIF played as it decodes gives the newest
    /// frame decoded at or before `t` and has a worker decode on; with
    /// `sync` (captures, export) the frame at `t` is decoded now.
    pub fn frame(&self, anim: &Arc<Animation>, t: f64, sync: bool) -> (Arc<ImageLevel>, f64) {
        let (i, next) = anim.index_at(t);
        let (img, work) = anim.frame(i, sync);
        if work {
            let a = anim.clone();
            self.enqueue(Job { key: (String::new(), 0), ext: String::new(), bytes: AssetBytes::from(Vec::new()), size: (0, 0), svg: None, make: None, task: Some(Box::new(move || a.work())) });
        }
        (img, next)
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
                        let mut job = job;
                        if let Some(task) = job.task.take() {
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
                        } else if job.make.is_some() && made_already(&shared, &job.key) {
                            // Made along with another (a picture's tiles
                            // are made together).
                        } else {
                            let out = decode_job(&job);
                            let g = shared.generation.load(Ordering::Relaxed);
                            insert(&shared, job.key, out, g.saturating_sub(1));
                        }
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

/// Whether `key` is cached already; if so it is no longer pending.
fn made_already(shared: &Shared, key: &Key) -> bool {
    let mut i = lock(&shared.inner);
    let have = i.map.contains_key(key);
    if have {
        i.pending.remove(key);
    }
    have
}

/// The cache, for work on a worker that makes several pictures at once
/// (the tiles of a picture's detail, [`make_tiles`]).
#[derive(Clone)]
pub struct CacheHandle(std::sync::Weak<Shared>);

impl CacheHandle {
    /// Cache `img` as the made picture `key`.
    pub fn put(&self, key: &str, img: ImageLevel) {
        if let Some(s) = self.0.upgrade() {
            let g = s.generation.load(Ordering::Relaxed);
            insert(&s, (key.to_string(), 0), vec![(0, Decoded::Still(Arc::new(img)))], g);
        }
    }

    /// `key` can't be made (it isn't asked for again).
    pub fn fail(&self, key: &str) {
        if let Some(s) = self.0.upgrade() {
            insert(&s, (key.to_string(), 0), vec![], 0);
        }
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

/// `bytes` (a still picture) scaled down to at most `most` pixels, by
/// the same decoders as any picture (never held at full size where they
/// can avoid it), as a PNG: for a picture inside an SVG, which resvg
/// would decode whole. None if it can't be read.
pub(crate) fn scaled_png(bytes: &[u8], most: u64) -> Option<Vec<u8>> {
    let head = zebra_core::media::picture_header(bytes)?;
    let (w, h) = (head.width, head.height);
    let last = last_level(w, h);
    let mut level = 0;
    while level < last && {
        let (lw, lh) = level_size(w, h, level);
        lw as u64 * lh as u64 > most
    } {
        level += 1;
    }
    let img = decode_still("", bytes, level).into_iter().find(|(l, _)| *l == level)?;
    let Decoded::Still(img) = img.1 else { return None };
    let (iw, ih) = (img.width, img.height);
    let mut px = img.pixmap().data_as_u8_slice().to_vec();
    // Back to straight alpha for the PNG.
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let mut out = vec![];
    let mut e = png::Encoder::new(&mut out, iw, ih);
    e.set_color(png::ColorType::Rgba);
    e.set_compression(png::Compression::Fast);
    e.write_header().ok()?.write_image_data(&px).ok()?;
    Some(out)
}

/// A picture the web player can't decode (TIFF, BMP, HEIC) as one it
/// can: the top level (up to 8,192 pixels, as the player shows), turned
/// upright, as a JPEG (quality 92) or, with any transparency, a PNG.
/// (extension, file). None if it can't be drawn here.
pub fn for_web(bytes: &[u8]) -> Option<(&'static str, Vec<u8>)> {
    let (_, img) = decode_still("", bytes, 0).into_iter().find(|(l, _)| *l == 0)?;
    let Decoded::Still(img) = img else { return None };
    let (w, h) = (img.width, img.height);
    let mut px = img.pixmap().data_as_u8_slice().to_vec();
    let opaque = px.chunks_exact(4).all(|p| p[3] == 255);
    let mut out = vec![];
    if opaque {
        let rgb: Vec<u8> = px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92).encode(&rgb, w, h, image::ExtendedColorType::Rgb8).ok()?;
        return Some(("jpg", out));
    }
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let mut e = png::Encoder::new(&mut out, w, h);
    e.set_color(png::ColorType::Rgba);
    e.write_header().ok()?.write_image_data(&px).ok()?;
    Some(("png", out))
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
    let Some(head) = zebra_core::media::picture_header(bytes).filter(|h| h.fits()) else { return vec![] };
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
    } else if head.format == PictureFormat::Jpeg && (pixels > SCALED_JPEG_FROM || iw.max(ih) > WHOLE_JPEG_SIDE) || !head.streams_or_decodes_whole() || cfg!(target_os = "macos") && head.format.mac_draws() {
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
    // Every decoder above gives the stored pixels; the levels are turned
    // upright last, small (see `PictureHeader::orientation`).
    let o = head.orientation;
    let mut it = shrinks.into_iter();
    let mut out = vec![];
    if let Some(s) = it.next() {
        out.push((want, Decoded::Still(Arc::new(s.finish_oriented(asset, o)))));
    }
    if let Some(s) = it.next() {
        out.push((last, Decoded::Still(Arc::new(s.finish_oriented(asset, o)))));
    }
    out
}

/// `px` (`w`x`h`, 4 bytes a pixel) shown with `orientation`: the pixels
/// and their new size (5 to 8 turn it a quarter).
pub fn orient_pixels(px: Vec<u8>, (w, h): (u32, u32), orientation: u8) -> (Vec<u8>, (u32, u32)) {
    if !(2..=8).contains(&orientation) || px.len() < w as usize * h as usize * 4 {
        return (px, (w, h));
    }
    let (ow, oh) = if orientation >= 5 { (h, w) } else { (w, h) };
    let mut out = vec![0u8; ow as usize * oh as usize * 4];
    for y in 0..oh {
        let row = &mut out[y as usize * ow as usize * 4..(y as usize + 1) * ow as usize * 4];
        for x in 0..ow {
            let (sx, sy) = zebra_core::media::oriented_from(orientation, (w, h), (x, y));
            let at = (sy as usize * w as usize + sx as usize) * 4;
            row[x as usize * 4..x as usize * 4 + 4].copy_from_slice(&px[at..at + 4]);
        }
    }
    (out, (ow, oh))
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


/// The part `rect` (x, y, width, height) of a `w`x`h` picture drawn at
/// `dw`x`dh` pixels (no bigger than the picture), as premultiplied RGBA
/// rows: the detail of a picture bigger than [`MAX_IMAGE_DIM`] seen closer
/// than its top level (P6). A PNG streams, so only the rows and columns
/// of the part are kept; on the Mac, ImageIO draws a JPEG or WebP's part
/// if the whole picture fits in memory; else the picture decodes whole
/// as its top level does. None if it can't be drawn.
///
/// Sizes are as shown (a photo turned upright); the part is read from the
/// stored pixels it comes from and turned the same way.
pub fn picture_region(bytes: &[u8], (w, h): (u32, u32), (dw, dh): (u32, u32), (x, y, rw, rh): (u32, u32, u32, u32)) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || dw == 0 || dh == 0 || dw > w || dh > h || rw == 0 || rh == 0 || x.checked_add(rw)? > dw || y.checked_add(rh)? > dh {
        return None;
    }
    let head = zebra_core::media::picture_header(bytes)?;
    if head.shown() != (w, h) {
        return None;
    }
    let o = head.orientation;
    // The same part of the stored pixels (in the stored detail grid).
    let (x0, x1, y0, y1) = (x, dw - x - rw, y, dh - y - rh);
    let (grid, part) = match o {
        2 => ((dw, dh), (x1, y0, rw, rh)),
        3 => ((dw, dh), (x1, y1, rw, rh)),
        4 => ((dw, dh), (x0, y1, rw, rh)),
        5 => ((dh, dw), (y0, x0, rh, rw)),
        6 => ((dh, dw), (y0, x1, rh, rw)),
        7 => ((dh, dw), (y1, x1, rh, rw)),
        8 => ((dh, dw), (y1, x0, rh, rw)),
        _ => ((dw, dh), (x, y, rw, rh)),
    };
    let px = stored_region(bytes, &head, grid, part)?;
    Some(orient_pixels(px, (part.2, part.3), o).0)
}

/// [`picture_region`] in the stored pixels' own terms.
fn stored_region(bytes: &[u8], head: &zebra_core::media::PictureHeader, (dw, dh): (u32, u32), (x, y, rw, rh): (u32, u32, u32, u32)) -> Option<Vec<u8>> {
    use zebra_core::media::PictureFormat;
    let (w, h) = (head.width, head.height);
    if head.format == PictureFormat::Gif || dw > w || dh > h {
        return None;
    }
    // The picture's own pixels the part covers.
    let span = |a: u32, n: u32, d: u32, s: u32| {
        let a0 = (a as u64 * s as u64 / d as u64) as u32;
        let a1 = (((a + n) as u64 * s as u64).div_ceil(d as u64) as u32).clamp(a0 + 1, s);
        (a0, a1)
    };
    let (sx0, sx1) = span(x, rw, dw, w);
    let (sy0, sy1) = span(y, rh, dh, h);
    let (cw, ch) = (sx1 - sx0, sy1 - sy0);
    let mut s = Shrink::new(cw, ch, rw.min(cw), rh.min(ch));
    let crop = |row: &[u8], c: usize| row.get(sx0 as usize * c..sx1 as usize * c).map(<[u8]>::to_vec);
    if head.format == PictureFormat::Png && !head.interlaced {
        let _permit = DecodeBytes::acquire(w as u64 * 16 + rw as u64 * rh as u64 * 4);
        let mut d = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: MAX_DECODE_ALLOC as usize });
        d.set_transformations(png::Transformations::normalize_to_color8());
        let mut r = d.read_info().ok()?;
        let c = png_channels(&r)?;
        let mut y = 0;
        while y < sy1 {
            let Ok(Some(row)) = r.next_row() else { break };
            if y >= sy0 {
                s.push(&crop(row.data(), c)?, c);
            }
            y += 1;
        }
        return Some(s.pixels());
    }
    #[cfg(target_os = "macos")]
    if matches!(head.format, PictureFormat::Jpeg | PictureFormat::WebP) || head.format.mac_draws() {
        let _permit = DecodeBytes::acquire(head.pixels() * 4);
        if let Some(px) = crate::imageio::region(bytes, (sx0, sy0, cw, ch), (rw.min(cw), rh.min(ch)), gif_memory()) {
            return Some(px);
        }
    }
    let pixels = head.pixels();
    if pixels > MAX_WHOLE_PIXELS {
        return None;
    }
    let _permit = DecodeBytes::acquire(pixels * 4 + rw as u64 * rh as u64 * 4);
    let img = decode_limited(bytes)?;
    if (img.width(), img.height()) != (w, h) {
        return None;
    }
    let (raw, c): (std::borrow::Cow<[u8]>, usize) = match &img {
        image::DynamicImage::ImageLuma8(b) => (b.as_raw().into(), 1),
        image::DynamicImage::ImageLumaA8(b) => (b.as_raw().into(), 2),
        image::DynamicImage::ImageRgb8(b) => (b.as_raw().into(), 3),
        image::DynamicImage::ImageRgba8(b) => (b.as_raw().into(), 4),
        other => (other.to_rgba8().into_raw().into(), 4),
    };
    for row in raw.chunks_exact(w as usize * c).skip(sy0 as usize).take(ch as usize) {
        s.push(&crop(row, c)?, c);
    }
    Some(s.pixels())
}

/// Bytes per pixel of a PNG reader's rows (normalised to 8 bits).
fn png_channels<R: std::io::BufRead + std::io::Seek>(r: &png::Reader<R>) -> Option<usize> {
    match r.output_color_type() {
        (png::ColorType::Grayscale, png::BitDepth::Eight) => Some(1),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => Some(2),
        (png::ColorType::Rgb, png::BitDepth::Eight) => Some(3),
        (png::ColorType::Rgba, png::BitDepth::Eight) => Some(4),
        _ => None,
    }
}

/// A tile of a picture's detail, and the others on screen still to make:
/// all are made from one [`picture_region`] of the part they cover (one
/// pass over a PNG), the others cached through `handle`. Each tile is
/// (cache key, x, y, width, height) at `detail` size.
pub type DetailTile = (String, (u32, u32, u32, u32));

/// Make the tiles `tiles` of a `size` picture drawn at `detail`, caching
/// all but `me`, which is returned.
pub fn make_tiles(bytes: &[u8], size: (u32, u32), detail: (u32, u32), tiles: &[DetailTile], me: &str, handle: &CacheHandle) -> Option<ImageLevel> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (_, (x, y, w, h)) in tiles {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(x + w), y1.max(y + h));
    }
    let Some(px) = (x1 > x0 && y1 > y0).then(|| picture_region(bytes, size, detail, (x0, y0, x1 - x0, y1 - y0))).flatten() else {
        for (k, _) in tiles.iter().filter(|(k, _)| k != me) {
            handle.fail(k);
        }
        return None;
    };
    let bw = (x1 - x0) as usize;
    let mut mine = None;
    for (k, (x, y, w, h)) in tiles {
        let mut out = Vec::with_capacity(*w as usize * *h as usize * 4);
        for row in 0..*h as usize {
            let at = ((*y - y0) as usize + row) * bw + (*x - x0) as usize;
            out.extend_from_slice(px.get(at * 4..(at + *w as usize) * 4)?);
        }
        let img = ImageLevel::from_premultiplied(k, *w, *h, out);
        if k == me {
            mine = Some(img);
        } else {
            handle.put(k, img);
        }
    }
    mine
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
/// A GIF whose frames cover more screen pixels than this in all (screen
/// area times frames) is decoded as it plays rather than all at once, so
/// its first frame shows at once. Not a limit.
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

    /// Premultiplied RGBA8 rows, `dw`x`dh` (rows never pushed stay clear).
    fn pixels(mut self) -> Vec<u8> {
        self.flush();
        self.out.resize(self.dw as usize * self.dh as usize * 4, 0);
        self.out
    }

    /// Premultiplied RGBA8, `dw`x`dh` (rows never pushed stay clear).
    fn finish(self, asset: &str) -> ImageLevel {
        self.finish_oriented(asset, 1)
    }

    /// The level, turned upright by `orientation`.
    fn finish_oriented(mut self, asset: &str, orientation: u8) -> ImageLevel {
        self.flush();
        self.out.resize(self.dw as usize * self.dh as usize * 4, 0);
        let _ = self.sw;
        let (px, (w, h)) = orient_pixels(self.out, (self.dw, self.dh), orientation);
        ImageLevel::from_premultiplied(asset, w, h, px)
    }
}

/// The memory a GIF's screen may take: the memory a document may take
/// (`zebra_core::io::memory_budget`), as for any picture.
fn gif_memory() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    return zebra_core::io::memory_budget();
    #[cfg(target_arch = "wasm32")]
    return 1 << 31;
}

fn gif_options() -> gif::DecodeOptions {
    let mut o = gif::DecodeOptions::new();
    o.set_color_output(gif::ColorOutput::RGBA);
    o.set_memory_limit(gif::MemoryLimit::Bytes(std::num::NonZeroU64::new(gif_memory()).unwrap_or(std::num::NonZeroU64::MIN)));
    o
}

/// A GIF's logical screen, with the decoder that draws its frames on it in
/// turn.
struct Screen {
    dec: gif::Decoder<std::io::Cursor<AssetBytes>>,
    w: usize,
    h: usize,
    canvas: Vec<u8>,
    /// What a "restore previous" frame covered.
    saved: Vec<u8>,
    /// The frame `dec` gives next.
    pos: usize,
}

impl Screen {
    /// None if it is unreadable, or its screen needs more memory than a
    /// document may take.
    fn open(bytes: &AssetBytes) -> Option<Screen> {
        let dec = gif_options().read_info(std::io::Cursor::new(bytes.clone())).ok()?;
        let (w, h) = (dec.width() as usize, dec.height() as usize);
        if !gif_fits(w as u32, h as u32) {
            return None;
        }
        Some(Screen { dec, w, h, canvas: vec![0u8; w * h * 4], saved: vec![], pos: 0 })
    }

    /// Draw the next frame on the screen; with `shrink` (asset, width,
    /// height) also give it at that size. None at the end (or where the
    /// file breaks off).
    fn step(&mut self, shrink: Option<(&str, u32, u32)>) -> Option<Option<ImageLevel>> {
        let (cw, ch) = (self.w, self.h);
        let f = match self.dec.read_next_frame() {
            Ok(Some(f)) => f,
            _ => return None,
        };
        // The frame's rectangle, clipped to the screen.
        let (fx, fy, fw, fh) = (f.left as usize, f.top as usize, f.width as usize, f.height as usize);
        let x0 = fx.min(cw);
        let x1 = (fx + fw).min(cw);
        let y0 = fy.min(ch);
        let y1 = (fy + fh).min(ch);
        if f.buffer.len() < fw * fh * 4 {
            return None;
        }
        let canvas = &mut self.canvas;
        let dispose = f.dispose;
        if dispose == gif::DisposalMethod::Previous {
            self.saved.clear();
            for y in y0..y1 {
                self.saved.extend_from_slice(&canvas[(y * cw + x0) * 4..(y * cw + x1) * 4]);
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
        let out = shrink.map(|(asset, lw, lh)| {
            let mut s = Shrink::new(cw as u32, ch as u32, lw, lh);
            for row in canvas.chunks_exact(cw * 4) {
                s.push(row, 4);
            }
            s.finish(asset)
        });
        match dispose {
            gif::DisposalMethod::Background => {
                for y in y0..y1 {
                    canvas[(y * cw + x0) * 4..(y * cw + x1) * 4].fill(0);
                }
            }
            gif::DisposalMethod::Previous => {
                let n = (x1 - x0) * 4;
                for (i, y) in (y0..y1).enumerate() {
                    canvas[(y * cw + x0) * 4..(y * cw + x1) * 4].copy_from_slice(&self.saved[i * n..(i + 1) * n]);
                }
            }
            _ => {}
        }
        self.pos += 1;
        Some(out)
    }
}

/// Whether a GIF screen of `w`x`h` fits in memory: the screen and its
/// "restore previous" copy.
fn gif_fits(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && (w as u64 * h as u64).saturating_mul(8) <= gif_memory()
}

/// When each frame of a GIF ends (seconds, cumulative), read without
/// decoding any pixels (their compressed data is only copied).
fn gif_timing(bytes: &[u8]) -> Vec<f64> {
    let mut o = gif_options();
    o.skip_frame_decoding(true);
    let Ok(mut d) = o.read_info(std::io::Cursor::new(bytes)) else { return vec![] };
    let mut ends = vec![];
    let mut t = 0.0;
    while let Ok(Some(f)) = d.read_next_frame() {
        // Browsers treat delays under 20 ms as 100 ms.
        let d = f.delay as f64 / 100.0;
        t += if d < 0.02 { 0.1 } else { d };
        ends.push(t);
    }
    ends
}

/// An animated (or still) GIF at `level` of its logical screen, decoded
/// with the `gif` crate on one screen-sized canvas. Every frame plays: a
/// GIF whose frames fit [`GIF_BUDGET`] at this level (and [`GIF_WORK`]) is
/// decoded whole now; a bigger one gets a player that decodes it as it
/// plays. Its screen's only bound is memory. None if it is unreadable or
/// its screen doesn't fit in memory.
fn decode_gif(asset: &str, bytes: &AssetBytes, level: u8) -> Option<Decoded> {
    let mut screen = Screen::open(bytes)?;
    let (w, h) = (screen.w as u32, screen.h as u32);
    let area = w as u64 * h as u64;
    let mut ends = gif_timing(bytes);
    let n = ends.len();
    let level = level.min(last_level(w, h));
    let (lw, lh) = level_size(w, h, level);
    let level_bytes = lw as usize * lh as usize * 4;
    let keep_all = n <= 1 || (n.saturating_mul(level_bytes) <= GIF_BUDGET && (n as u64).saturating_mul(area) <= GIF_WORK);
    if keep_all {
        let _permit = DecodeBytes::acquire(area * 8 + (n.max(1) * level_bytes) as u64);
        let mut frames: Vec<Arc<ImageLevel>> = vec![];
        while frames.len() < n.max(1) {
            match screen.step(Some((asset, lw, lh))) {
                Some(Some(f)) => frames.push(Arc::new(f.as_moving())),
                _ => break,
            }
        }
        ends.truncate(frames.len());
        if frames.len() <= 1 {
            return frames.pop().map(Decoded::Still);
        }
        let first = frames[0].clone();
        return Some(Decoded::Animated(Arc::new(Animation { ends, first, frames: Frames::All(frames) })));
    }
    let first = Arc::new(screen.step(Some((asset, lw, lh)))??.as_moving());
    let player = Player {
        asset: asset.to_string(),
        bytes: bytes.clone(),
        size: (lw, lh),
        screen_bytes: (area * 8) as usize,
        state: Mutex::new(PlayState { screen: Some(screen), ..Default::default() }),
        held: AtomicUsize::new(0),
    };
    player.recount(&lock_play(&player.state));
    Some(Decoded::Animated(Arc::new(Animation { ends, first, frames: Frames::Played(player) })))
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
        assert_eq!(a.len(), 3);
        assert!(!a.is_played(), "a short GIF is kept whole");
        assert!((a.total() - 0.6).abs() < 1e-9);
        let (i, next) = a.index_at(0.25);
        assert_eq!(i, 1);
        assert!((next - 0.15).abs() < 1e-9);
        assert_eq!(a.index_at(0.65).0, 0);
        let (f, _) = c.frame(&a, 0.25, true);
        assert_eq!(f.pixmap().data_as_u8_slice()[0], 80);
        assert!(Arc::ptr_eq(&c.frame(&a, 0.65, true).0, &a.first));
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
