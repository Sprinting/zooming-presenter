//! Media files: sniffing what an asset is, reading video size and length
//! from MP4/QuickTime headers, and the on-disk media cache.
//!
//! Only headers are read here. Decoding is the platform's job
//! (`zebra-media`), so nothing in this module allocates per frame or
//! touches more than a few kilobytes of a file.
use std::io::Read as _;
use std::path::{Path, PathBuf};

use crate::error::{ZError, ZResult};
use crate::model::{AssetBytes, MediaKind};

/// What a media file is, from its first bytes and headers.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub ext: String,
    pub kind: MediaKind,
    /// Pixels (0 for audio, or when the header doesn't say).
    pub width: u32,
    pub height: u32,
    /// Seconds (0 when unknown).
    pub duration: f64,
}

/// Identify any supported asset: images, mp4/mov video, mp3/m4a/aac/wav audio.
pub fn info(bytes: &[u8]) -> ZResult<MediaInfo> {
    if let Some(m) = sniff_av(bytes) {
        return Ok(m);
    }
    let (ext, w, h) = crate::editor::image_info(bytes).map_err(|e| {
        if e.message.starts_with("unrecognised") {
            ZError::invalid("unrecognised media format").hint(FORMATS_HINT)
        } else {
            e
        }
    })?;
    Ok(MediaInfo { ext, kind: MediaKind::Image, width: w, height: h, duration: 0.0 })
}

fn sniff_av(b: &[u8]) -> Option<MediaInfo> {
    let audio = |ext: &str, duration: f64| MediaInfo { ext: ext.into(), kind: MediaKind::Audio, width: 0, height: 0, duration };
    if b.len() >= 12 && &b[4..8] == b"ftyp" {
        // HEIC and AVIF pictures (any HEIF picture brand) are pictures,
        // whatever else the file holds.
        if iso_picture(b).is_some() {
            return None;
        }
        let brand = &b[8..12];
        let mov = parse_mov(b);
        // A movie header with no tracks in it holds nothing to play: not
        // a sound, not a video (a bomb's first bytes, then zeros).
        if mov.as_ref().is_some_and(|m| m.tracks == 0) {
            return None;
        }
        let has_video = mov.as_ref().map(|m| m.has_video);
        let duration = mov.as_ref().map(|m| m.duration).unwrap_or(0.0);
        let is_audio_brand = matches!(brand, b"M4A " | b"M4B " | b"M4P ");
        if is_audio_brand || has_video == Some(false) {
            return Some(audio("m4a", duration));
        }
        let ext = match brand {
            b"qt  " => "mov",
            b"M4V " | b"M4VH" | b"M4VP" => "m4v",
            _ => "mp4",
        };
        let (w, h) = mov.as_ref().map(|m| (m.width, m.height)).unwrap_or((0, 0));
        return Some(MediaInfo { ext: ext.into(), kind: MediaKind::Video, width: w, height: h, duration });
    }
    // QuickTime files from older cameras can start with moov/mdat/wide.
    if b.len() >= 8 && matches!(&b[4..8], b"moov" | b"mdat" | b"wide" | b"free") {
        if let Some(m) = parse_mov(b).filter(|m| m.has_video) {
            return Some(MediaInfo { ext: "mov".into(), kind: MediaKind::Video, width: m.width, height: m.height, duration: m.duration });
        }
    }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE" {
        return Some(audio("wav", wav_duration(b).unwrap_or(0.0)));
    }
    if b.starts_with(b"ID3") {
        return Some(audio("mp3", 0.0));
    }
    if b.len() >= 4 && b[0] == 0xFF && (b[1] & 0xE0) == 0xE0 {
        // MPEG audio frame sync. Layer bits 00 mean ADTS (AAC).
        let layer = (b[1] >> 1) & 0x3;
        if (b[1] & 0xF0) == 0xF0 && layer == 0 {
            return Some(audio("aac", 0.0));
        }
        if layer != 0 {
            return Some(audio("mp3", 0.0));
        }
    }
    None
}

#[derive(Debug, Default)]
struct MovInfo {
    has_video: bool,
    /// Tracks in `moov`: none means nothing can play (a header followed
    /// by zeros).
    tracks: usize,
    width: u32,
    height: u32,
    duration: f64,
}

/// Walk the MP4/QuickTime box tree far enough to find `mvhd` (length) and
/// the first video track's `tkhd` (display size). Jumps over `mdat`, so
/// only headers are read even when `moov` sits at the end of a big file.
fn parse_mov(b: &[u8]) -> Option<MovInfo> {
    let moov = find_box(b, b"moov")?;
    let mut out = MovInfo::default();
    if let Some(mvhd) = find_box(moov, b"mvhd") {
        out.duration = mvhd_duration(mvhd).unwrap_or(0.0);
    }
    for trak in boxes(moov).filter(|(t, _)| t == b"trak").map(|(_, body)| body) {
        out.tracks += 1;
        let is_video = find_box(trak, b"mdia")
            .and_then(|mdia| find_box(mdia, b"hdlr"))
            .is_some_and(|hdlr| hdlr.len() >= 12 && &hdlr[8..12] == b"vide");
        // The first video track gives the size.
        if is_video && !out.has_video {
            out.has_video = true;
            if let Some(tkhd) = find_box(trak, b"tkhd") {
                // width and height are the last 8 bytes, 16.16 fixed point.
                if tkhd.len() >= 8 {
                    let n = tkhd.len();
                    out.width = u32::from_be_bytes(tkhd[n - 8..n - 4].try_into().ok()?) >> 16;
                    out.height = u32::from_be_bytes(tkhd[n - 4..n].try_into().ok()?) >> 16;
                }
            }
        }
    }
    Some(out)
}

/// Longest a video or sound may say it lasts, seconds. A header saying
/// more is lying (or is a stream of unknown length): its length is then
/// unknown (0), and the player asks the file itself.
pub const MAX_MEDIA_SECONDS: f64 = 24.0 * 60.0 * 60.0;

/// `d` seconds if it is a believable length, else 0 (unknown).
pub fn plausible_duration(d: f64) -> f64 {
    if d.is_finite() && d > 0.0 && d <= MAX_MEDIA_SECONDS { d } else { 0.0 }
}

fn mvhd_duration(m: &[u8]) -> Option<f64> {
    let version = *m.first()?;
    // A duration of all ones means "unknown" (ISO/IEC 14496-12).
    let (ts, dur) = if version == 1 {
        let d = u64::from_be_bytes(m.get(24..32)?.try_into().ok()?);
        (u32::from_be_bytes(m.get(20..24)?.try_into().ok()?) as f64, (d != u64::MAX).then_some(d as f64)?)
    } else {
        let d = u32::from_be_bytes(m.get(16..20)?.try_into().ok()?);
        (u32::from_be_bytes(m.get(12..16)?.try_into().ok()?) as f64, (d != u32::MAX).then_some(d as f64)?)
    };
    (ts > 0.0).then(|| plausible_duration(dur / ts))
}

/// Iterate the boxes directly inside `b`: (type, body).
fn boxes(b: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        if pos + 8 > b.len() {
            return None;
        }
        let size32 = u32::from_be_bytes(b[pos..pos + 4].try_into().unwrap()) as u64;
        let typ: [u8; 4] = b[pos + 4..pos + 8].try_into().unwrap();
        let (header, size) = match size32 {
            0 => (8u64, (b.len() - pos) as u64),
            1 => {
                if pos + 16 > b.len() {
                    return None;
                }
                (16, u64::from_be_bytes(b[pos + 8..pos + 16].try_into().unwrap()))
            }
            n => (8, n),
        };
        if size < header {
            return None;
        }
        let end = (pos as u64).checked_add(size)?.min(b.len() as u64) as usize;
        let body = &b[pos + header as usize..end];
        pos = end;
        Some((typ, body))
    })
}

fn find_box<'a>(b: &'a [u8], typ: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(b).find(|(t, _)| t == typ).map(|(_, body)| body)
}

fn wav_duration(b: &[u8]) -> Option<f64> {
    let mut pos = 12;
    let mut byte_rate = 0u32;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let len = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        if id == b"fmt " && pos + 20 <= b.len() {
            byte_rate = u32::from_le_bytes(b[pos + 16..pos + 20].try_into().ok()?);
        }
        if id == b"data" {
            return (byte_rate > 0).then(|| plausible_duration(len as f64 / byte_rate as f64));
        }
        pos += 8 + len + (len & 1);
    }
    None
}

/// Where players' copies of video and audio live as plain files (see
/// [`materialize`]): `$ZEBRA_MEDIA_CACHE`, else the user cache directory
/// (`~/Library/Caches/Zebra/media` on macOS, inside the sandbox container
/// when sandboxed).
pub fn cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ZEBRA_MEDIA_CACHE") {
        return PathBuf::from(d);
    }
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    base.join("Zebra").join("media")
}

// ----------------------------------------------------------------------
// Mapped files

/// A memory-mapped file: a saved `.zebra` whose assets are slices of it,
/// or a file holding one asset. An *owned* file is one Zebra made for the
/// mapping (a spill file, a player's copy): it is removed when the last
/// asset, undo step or player using it lets go, so nothing is left behind
/// after a delete, a save, a close or a quit.
pub struct MappedFile {
    map: memmap2::Mmap,
    path: Option<PathBuf>,
    owned: bool,
    /// The open file, kept only for a mapping whose name is gone (a
    /// document's clone or private copy, removed as soon as it is mapped):
    /// the PDF helper reads such bytes through it. Others reopen their
    /// name, so thousands of spill files hold no descriptors.
    file: Option<std::fs::File>,
}

impl MappedFile {
    /// Map `path` read-only. Not owned: the file stays when this goes.
    pub fn open(path: &Path) -> std::io::Result<MappedFile> {
        let f = std::fs::File::open(path)?;
        // SAFETY: read-only map. Zebra never writes a file it has mapped
        // (saves write a new file and rename it over); see `AssetBytes`.
        let map = unsafe { memmap2::Mmap::map(&f) }?;
        Ok(MappedFile { map, path: Some(path.to_path_buf()), owned: false, file: None })
    }

    /// Map `path`, which the caller removes right after (a clone or a
    /// private copy), keeping the open file so the bytes can still be
    /// handed to another process ([`MappedFile::reopen`]).
    pub fn open_unlinked(path: &Path) -> std::io::Result<MappedFile> {
        let f = std::fs::File::open(path)?;
        // SAFETY: as in `open`; the file is Zebra's own copy.
        let map = unsafe { memmap2::Mmap::map(&f) }?;
        Ok(MappedFile { map, path: Some(path.to_path_buf()), owned: false, file: Some(f) })
    }

    /// An open file holding these bytes, for another process to read (the
    /// PDF helper): the kept file, else the file by its name.
    pub fn reopen(&self) -> std::io::Result<std::fs::File> {
        match (&self.file, &self.path) {
            (Some(f), _) => f.try_clone(),
            (None, Some(p)) => {
                let f = std::fs::File::open(p)?;
                // The name may lead to another file by now: it must at
                // least be as long as the mapping.
                if f.metadata()?.len() < self.map.len() as u64 {
                    return Err(std::io::Error::other("the file changed"));
                }
                Ok(f)
            }
            (None, None) => Err(std::io::Error::other("no file")),
        }
    }

    /// Map a file only Zebra knows (its own spill file or copy), which is
    /// removed when the mapping goes.
    fn owned(path: PathBuf) -> std::io::Result<MappedFile> {
        let f = std::fs::File::open(&path)?;
        // SAFETY: as in `open`; nobody else writes Zebra's own files.
        let map = unsafe { memmap2::Mmap::map(&f) }?;
        Ok(MappedFile { map, path: Some(path), owned: true, file: None })
    }

    /// The same mapping, known by `path` (a clone mapped in place of the
    /// file it was cloned from) and not owned.
    pub fn renamed(mut self, path: &Path) -> MappedFile {
        self.owned = false;
        self.path = Some(path.to_path_buf());
        self
    }

    /// The mapped file's path (it may have been removed or renamed since).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Is this a file Zebra removes when the mapping goes?
    pub fn is_owned(&self) -> bool {
        self.owned
    }
}

impl std::ops::Deref for MappedFile {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.map
    }
}

impl Drop for MappedFile {
    fn drop(&mut self) {
        if let (true, Some(p)) = (self.owned, &self.path) {
            let _ = std::fs::remove_file(p);
        }
    }
}

impl std::fmt::Debug for MappedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MappedFile({:?}, {} bytes{})", self.path, self.map.len(), if self.owned { ", owned" } else { "" })
    }
}

impl From<MappedFile> for AssetBytes {
    fn from(m: MappedFile) -> AssetBytes {
        let len = m.len();
        AssetBytes::Mapped { map: std::sync::Arc::new(m), start: 0, len }
    }
}

// ----------------------------------------------------------------------
// Content keys

/// An asset's key, made while its bytes go by: FNV-1a 64 of the bytes
/// and their length, stable across runs and platforms.
#[derive(Debug, Clone)]
pub struct KeyHasher {
    h: u64,
    len: u64,
}

impl Default for KeyHasher {
    fn default() -> Self {
        KeyHasher { h: 0xcbf29ce484222325, len: 0 }
    }
}

impl KeyHasher {
    pub fn update(&mut self, bytes: &[u8]) {
        let mut h = self.h;
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        self.h = h;
        self.len += bytes.len() as u64;
    }
    pub fn finish(&self) -> String {
        format!("{:016x}{:x}", self.h, self.len)
    }
}

/// The content key of `bytes` (see [`KeyHasher`]).
pub fn content_key(bytes: &[u8]) -> String {
    let mut k = KeyHasher::default();
    k.update(bytes);
    k.finish()
}

// ----------------------------------------------------------------------
// Spill files

/// Where new media bytes are spilled (see [`spill`]); `None`: kept in
/// memory. The app sets it to a folder beside its autosaves.
static SPILL_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Media smaller than this stays in memory.
pub const SPILL_MIN: usize = 64 * 1024;

pub fn set_spill_dir(dir: Option<PathBuf>) {
    *SPILL_DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

pub fn spill_dir() -> Option<PathBuf> {
    SPILL_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Where media read from a file or a stream goes: the spill folder, or a
/// folder in the system's temporary folder when none is set (tools and
/// tests), since such media is never read into memory.
fn stream_dir() -> PathBuf {
    spill_dir().unwrap_or_else(|| std::env::temp_dir().join("Zebra-media"))
}

/// `<pid>-<n>-<tag>` in `dir`: a name no other spill uses, in this
/// process or another, that says which process made it.
fn spill_path(dir: &Path, tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!("{}-{n}-{tag}", std::process::id()))
}

fn check_ext(ext: &str) -> std::io::Result<()> {
    if !ext.is_empty() && ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Ok(());
    }
    Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a media file extension"))
}

/// New media bytes (an agent's picture, a theme font) as an asset. Kept
/// on the heap they stayed there until the document was saved: 400 photos
/// an agent added were 515 MB of heap. With a spill folder set, bytes of
/// [`SPILL_MIN`] or more are written to a file there (`<pid>-<n>-<key>.<ext>`)
/// and memory-mapped, like the media of an opened file, so the system can
/// page them out. The file is Zebra's own ([`MappedFile`]): it goes when
/// the last asset or undo step using it does, and a run that crashed has
/// its files swept at the next launch ([`sweep_spill`]). Recovery reads the
/// bytes from the autosave, which links to them. Any failure keeps the
/// bytes in memory.
pub fn spill(bytes: Vec<u8>, key: &str, ext: &str) -> AssetBytes {
    if bytes.len() < SPILL_MIN || !cfg!(unix) || !crate::check::safe_asset_key(key) || check_ext(ext).is_err() {
        return bytes.into();
    }
    let Some(dir) = spill_dir() else { return bytes.into() };
    let path = spill_path(&dir, &format!("{key}.{ext}"));
    let mapped = (|| -> std::io::Result<MappedFile> {
        use std::io::Write;
        std::fs::create_dir_all(&dir)?;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        if let Err(e) = f.write_all(&bytes) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        drop(f);
        MappedFile::owned(path.clone()).inspect_err(|_| {
            let _ = std::fs::remove_file(&path);
        })
    })();
    match mapped {
        Ok(m) if m.len() == bytes.len() => m.into(),
        _ => bytes.into(),
    }
}

/// Stream media into a spill file without holding it in memory (a video
/// out of a PowerPoint file, a clip being inserted): returns its content
/// key and the mapped bytes. `ext` is the file's type ("mp4").
/// Space unpacking or downloading media leaves free on the disk, and below
/// which it stops.
pub const SPACE_MARGIN: u64 = 512 * 1024 * 1024;
/// How often (bytes written) a spill looks at the free space again.
const SPACE_EVERY: u64 = 16 * 1024 * 1024;

/// Free bytes on the disk holding `dir` (for the user, not root). None
/// when it can't be told.
pub fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: plain old data, filled in by statvfs.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a NUL-terminated path and `st` a writable statvfs.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

#[doc(hidden)]
/// Tests only: pretend the disk has this much free (None: ask the disk).
pub fn set_free_space_for_tests(free: Option<u64>) {
    FREE_OVERRIDE.store(free.map_or(u64::MAX, |f| f.min(u64::MAX - 1)), std::sync::atomic::Ordering::Relaxed);
}
static FREE_OVERRIDE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

fn spill_room(dir: &Path) -> bool {
    let free = match FREE_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        u64::MAX => free_space(dir),
        f => Some(f),
    };
    free.is_none_or(|f| f >= SPACE_MARGIN)
}

/// Copy `r` into a spill file and map it: its content key and the mapped
/// bytes. Stops with `StorageFull` once the disk is down to
/// [`SPACE_MARGIN`], however much `r` has left (a zip bomb fills nothing).
pub fn spill_reader(r: &mut dyn std::io::Read, ext: &str) -> std::io::Result<(String, AssetBytes)> {
    check_ext(ext)?;
    let dir = stream_dir();
    std::fs::create_dir_all(&dir)?;
    if !spill_room(&dir) {
        return Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "the disk is full"));
    }
    let part = spill_path(&dir, "part");
    let mut key = KeyHasher::default();
    let r = (|| -> std::io::Result<()> {
        use std::io::Write;
        let f = std::fs::OpenOptions::new().write(true).create_new(true).open(&part)?;
        let mut w = std::io::BufWriter::with_capacity(1 << 20, f);
        let mut buf = vec![0u8; 1 << 20];
        let mut since = 0u64;
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            key.update(&buf[..n]);
            w.write_all(&buf[..n])?;
            since += n as u64;
            if since >= SPACE_EVERY {
                since = 0;
                if !spill_room(&dir) {
                    return Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "the disk is full"));
                }
            }
        }
        w.flush()
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    finish_spill(&dir, part, key.finish(), ext)
}

/// Copy a media file into a spill file (`clonefile` on APFS, so usually
/// free) and map the copy: returns its content key and the mapped bytes.
/// Mapping the copy, not the original, means the person can move, edit or
/// delete the original without disturbing the document.
pub fn spill_file(src: &Path, ext: &str) -> std::io::Result<(String, AssetBytes)> {
    check_ext(ext)?;
    regular_file(src)?;
    let dir = stream_dir();
    std::fs::create_dir_all(&dir)?;
    let part = spill_path(&dir, "part");
    if let Err(e) = std::fs::copy(src, &part) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    let key = match MappedFile::open(&part) {
        Ok(m) => content_key(&m),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    finish_spill(&dir, part, key, ext)
}

/// `path`'s metadata if it is a regular file (after symlinks). A device
/// (`/dev/zero`, `/dev/random`), a FIFO or a socket reports a length of 0
/// and never ends when read; a folder can't be read at all.
pub fn regular_file(path: &Path) -> std::io::Result<std::fs::Metadata> {
    let m = std::fs::metadata(path)?;
    if !m.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a file"));
    }
    Ok(m)
}

/// The first bytes of a regular file, enough to tell what it is
/// ([`sniff_kind`]) before copying any of it.
pub fn read_head(path: &Path) -> std::io::Result<(std::fs::Metadata, Vec<u8>)> {
    let meta = regular_file(path)?;
    let mut head = Vec::with_capacity(HEAD as usize);
    std::fs::File::open(path)?.take(HEAD).read_to_end(&mut head)?;
    Ok((meta, head))
}

const HEAD: u64 = 256 << 10;

// ----------------------------------------------------------------------
// Pictures of any size (Kartik, 21 Nov: no file-size cap; like Keynote, a
// huge picture comes in and is shown scaled down). What limits a picture
// is whether its pixels can be scaled down without ever holding them at
// full size, and that depends on the format, read from the header:
//
// * PNG: rows stream into the downscaler, so any height, and any width
//   up to `MAX_STREAM_WIDTH` (a row and the column table stay small).
//   Interlaced (Adam7) PNGs over the whole-decode budget are drawn from
//   their first pass, which is every 8th pixel each way.
// * JPEG and WebP on the Mac: drawn by ImageIO at the size shown, as
//   Keynote draws them (a JPEG straight from its DCT at a smaller size),
//   up to the formats' own most (65,535 a side for JPEG, 16,383 for WebP).
//   Elsewhere (development builds) they decode whole, so they are bounded
//   by `MAX_WHOLE_PIXELS`, and a progressive JPEG, which keeps every
//   coefficient while its scans arrive, by `MAX_PROGRESSIVE_PIXELS`.
// * GIF: its frames are composited on a full-size screen (and a copy for
//   "restore previous" frames), so its screen is bounded only by the
//   memory a document may take; frames are decoded as they play, so its
//   length has no bound (B4).
//
// Pictures within these come in whatever their file size; the renderer
// keeps to the same numbers (zebra_render::images).

/// Widest PNG row streamed (pixels): 16-bit RGBA rows of 8 MB, and a
/// 4 MB column table in the downscaler.
pub const MAX_STREAM_WIDTH: u64 = 1 << 20;
/// Largest picture decoded whole (pixels): 512 MB as RGBA.
pub const MAX_WHOLE_PIXELS: u64 = 128 << 20;
/// Largest progressive (or lossless) JPEG (pixels): its coefficients, 2
/// bytes a sample and up to 3 samples a pixel, fit in 384 MB.
pub const MAX_PROGRESSIVE_PIXELS: u64 = 64 << 20;

/// A picture's format and size from its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureHeader {
    pub format: PictureFormat,
    pub width: u32,
    pub height: u32,
    /// An interlaced PNG or a progressive JPEG.
    pub interlaced: bool,
    /// How the stored pixels are turned to show it upright, as EXIF
    /// numbers it (1 as stored, 2 to 8 flipped or turned; 5 to 8 turn it
    /// a quarter, so it shows `height` wide). From EXIF in a JPEG, PNG,
    /// WebP or TIFF, and from a HEIC's rotation and mirror. Keynote shows
    /// photos upright, so Zebra does too.
    pub orientation: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureFormat {
    Png,
    Jpeg,
    WebP,
    Gif,
    Tiff,
    Bmp,
    Heic,
}

impl PictureFormat {
    /// The extension Zebra keeps the file under (the file is kept as it
    /// is, as Keynote keeps pictures in their own format).
    pub fn ext(self) -> &'static str {
        match self {
            PictureFormat::Png => "png",
            PictureFormat::Jpeg => "jpg",
            PictureFormat::WebP => "webp",
            PictureFormat::Gif => "gif",
            PictureFormat::Tiff => "tiff",
            PictureFormat::Bmp => "bmp",
            PictureFormat::Heic => "heic",
        }
    }
    /// Drawn by ImageIO on the Mac whatever its size (Zebra's own
    /// decoders are for the rest of the formats, and for development
    /// builds): the Mac reads every TIFF and BMP variant and HEIC, as
    /// Keynote does.
    pub fn mac_draws(self) -> bool {
        matches!(self, PictureFormat::Tiff | PictureFormat::Bmp | PictureFormat::Heic)
    }
}

impl PictureHeader {
    /// The size it shows at: turned a quarter for orientations 5 to 8.
    pub fn shown(&self) -> (u32, u32) {
        if self.orientation >= 5 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
    /// Whether Zebra can show it scaled down without its pixels ever being
    /// held at full size beyond the budgets above (see the module notes).
    pub fn fits(&self) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }
        if cfg!(target_os = "macos") && (matches!(self.format, PictureFormat::Jpeg | PictureFormat::WebP) || self.format.mac_draws()) {
            return true;
        }
        // Only the Mac decodes HEIC (there is no decoder without a new
        // library); a development build takes it and shows the "can't be
        // shown" placeholder. Its size is no reason to refuse it.
        if self.format == PictureFormat::Heic {
            return true;
        }
        self.streams_or_decodes_whole()
    }
    /// Whether Zebra's own decoders can scale it down within the budgets
    /// (streaming a PNG, or decoding the rest whole). What can't is left
    /// to ImageIO on the Mac.
    pub fn streams_or_decodes_whole(&self) -> bool {
        match self.format {
            PictureFormat::Png => self.width as u64 <= MAX_STREAM_WIDTH,
            PictureFormat::Jpeg => self.pixels() <= MAX_WHOLE_PIXELS && (!self.interlaced || self.pixels() <= MAX_PROGRESSIVE_PIXELS),
            PictureFormat::WebP => self.pixels() <= MAX_WHOLE_PIXELS,
            PictureFormat::Gif => self.pixels().saturating_mul(8) <= picture_memory(),
            PictureFormat::Tiff | PictureFormat::Bmp => self.pixels() <= MAX_WHOLE_PIXELS,
            PictureFormat::Heic => false,
        }
    }
}

/// Whether a picture's file is too small to hold the pixels its header
/// claims, so the header lies (a 74-byte PNG saying it is 200,000 x
/// 200,000, M17). Each bound is the format's own best case, so a real
/// picture always passes, however plain:
///
/// * PNG: deflate can't beat 1032:1 (zlib's documented limit for the
///   deflate format), so the file holds at least the filtered rows / 1032;
/// * JPEG: every 8x8 block of the least-sampled component (at most 4x
///   subsampled, so 32x32 pixels) takes at least one bit of entropy-coded
///   data;
/// * WebP: at least one bit per 1,048,576 pixels (its arithmetic coder can
///   make a near-certain bit cost far less than one, so the bound is loose).
///
/// GIF, TIFF, BMP and HEIC say nothing here: a GIF's screen can be far
/// bigger than its frames (a 65,535² screen with 1x1 frames is a valid
/// GIF, and its screen bound applies instead), and the others' whole-decode
/// bounds apply.
pub fn header_lies(head: &PictureHeader, file_len: u64) -> bool {
    let (w, h) = (head.width as u64, head.height as u64);
    let pixels = w.saturating_mul(h);
    let least: u64 = match head.format {
        PictureFormat::Png => {
            // Filtered rows at one byte per pixel (the least any colour
            // type takes, but 1, 2 and 4-bit greys and palettes).
            let row = 1 + w.div_ceil(8);
            row.saturating_mul(h) / 1032
        }
        PictureFormat::Jpeg => w.div_ceil(32).saturating_mul(h.div_ceil(32)) / 8,
        PictureFormat::WebP => pixels >> 23,
        PictureFormat::Gif | PictureFormat::Tiff | PictureFormat::Bmp | PictureFormat::Heic => 0,
    };
    file_len < least
}

/// The memory one picture's decoding may take where nothing smaller
/// applies (a GIF's screen): what a document may take.
fn picture_memory() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    return crate::io::memory_budget();
    #[cfg(target_arch = "wasm32")]
    return 1 << 31;
}

/// The header of a PNG, JPEG, WebP, GIF, TIFF, BMP or HEIC from its
/// bytes: the whole file or its start. Only what the header needs is
/// read: the first 256 KB for most formats (a JPEG's size can follow
/// 64 KB of metadata); a TIFF's tags and a HEIC's boxes can sit anywhere,
/// so they are found by their offsets, never by reading the rest.
pub fn picture_header(bytes: &[u8]) -> Option<PictureHeader> {
    let head = &bytes[..bytes.len().min(256 << 10)];
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    if head.starts_with(b"\x89PNG\r\n\x1a\n") && head.len() >= 29 && &head[12..16] == b"IHDR" {
        let orientation = png_orientation(head);
        return Some(PictureHeader { format: PictureFormat::Png, width: be32(&head[16..20]), height: be32(&head[20..24]), interlaced: head[28] != 0, orientation });
    }
    if head.starts_with(&[0xFF, 0xD8]) {
        return jpeg_header(head);
    }
    if let Some(t) = tiff_tags(bytes) {
        return (t.width > 0 && t.height > 0).then_some(PictureHeader { format: PictureFormat::Tiff, width: t.width, height: t.height, interlaced: false, orientation: t.orientation });
    }
    if is_heic(head) {
        return heic_header(bytes);
    }
    if head.starts_with(b"BM") {
        return bmp_header(head);
    }
    let format = match image::guess_format(head).ok()? {
        image::ImageFormat::WebP => PictureFormat::WebP,
        image::ImageFormat::Gif => PictureFormat::Gif,
        _ => return None,
    };
    let fmt = if format == PictureFormat::Gif { image::ImageFormat::Gif } else { image::ImageFormat::WebP };
    let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(head), fmt).into_dimensions().ok()?;
    let orientation = if format == PictureFormat::WebP { webp_orientation(head) } else { 1 };
    Some(PictureHeader { format, width, height, interlaced: false, orientation })
}

/// What a TIFF's first image says of itself (or an EXIF block's, which
/// is laid out as a TIFF).
#[derive(Debug, Default)]
struct TiffTags {
    width: u32,
    height: u32,
    orientation: u8,
}

/// The size and orientation tags of a TIFF (or BigTIFF) file's first
/// image, from its byte order mark and first directory. None if `b`
/// isn't one. A directory past the end, or a broken entry, leaves what
/// it can't read at 0 (orientation at 1).
fn tiff_tags(b: &[u8]) -> Option<TiffTags> {
    let le = match b.get(..4)? {
        [b'I', b'I', 42 | 43, 0] => true,
        [b'M', b'M', 0, 42 | 43] => false,
        _ => return None,
    };
    let big = b[2] == 43 || b[3] == 43;
    let rd = |o: usize, n: usize| -> Option<u64> {
        let s = b.get(o..o.checked_add(n)?)?;
        let mut v = 0u64;
        for i in 0..n {
            let x = if le { s[n - 1 - i] } else { s[i] };
            v = v << 8 | x as u64;
        }
        Some(v)
    };
    let mut t = TiffTags { orientation: 1, ..Default::default() };
    let (ifd, count_len, entry_len, value_at) = if big { (rd(8, 8)?, 8, 20, 12) } else { (rd(4, 4)?, 2, 12, 8) };
    let Ok(ifd) = usize::try_from(ifd) else { return Some(t) };
    let Some(n) = rd(ifd, count_len) else { return Some(t) };
    // A directory with more entries than the file could hold is broken;
    // reading stops at the file's end anyway.
    for i in 0..n.min(65_535) as usize {
        let Some(e) = ifd.checked_add(count_len + entry_len * i) else { break };
        let (Some(tag), Some(ty)) = (rd(e, 2), rd(e + 2, 2)) else { break };
        let v = match ty {
            3 => rd(e + value_at, 2),
            4 => rd(e + value_at, 4),
            16 => rd(e + value_at, 8),
            _ => None,
        };
        let Some(v) = v else { continue };
        match tag {
            256 => t.width = u32::try_from(v).unwrap_or(0),
            257 => t.height = u32::try_from(v).unwrap_or(0),
            274 if (1..=8).contains(&v) => t.orientation = v as u8,
            _ => {}
        }
    }
    Some(t)
}

/// The orientation in an EXIF block ("Exif\0\0" then a TIFF layout), 1
/// if it has none.
fn exif_orientation(exif: &[u8]) -> u8 {
    let b = exif.strip_prefix(b"Exif\0\0").unwrap_or(exif);
    tiff_tags(b).map_or(1, |t| t.orientation)
}

/// A PNG's EXIF orientation (its eXIf chunk), from the chunks in `head`.
fn png_orientation(head: &[u8]) -> u8 {
    let mut i = 8;
    while i + 8 <= head.len() {
        let len = u32::from_be_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]) as usize;
        let ty = &head[i + 4..i + 8];
        let Some(end) = (i + 8).checked_add(len) else { break };
        if ty == b"eXIf" {
            return head.get(i + 8..end).map_or(1, exif_orientation);
        }
        if ty == b"IDAT" || ty == b"IEND" {
            break;
        }
        i = end + 4;
    }
    1
}

/// A WebP's EXIF orientation (its EXIF chunk), from the chunks in `head`.
fn webp_orientation(head: &[u8]) -> u8 {
    let mut i = 12;
    while i + 8 <= head.len() {
        let len = u32::from_le_bytes([head[i + 4], head[i + 5], head[i + 6], head[i + 7]]) as usize;
        let Some(end) = (i + 8).checked_add(len) else { break };
        if &head[i..i + 4] == b"EXIF" {
            return head.get(i + 8..end).map_or(1, exif_orientation);
        }
        i = end + (len & 1);
    }
    1
}

/// A BMP's size from its header (a negative height is a top-down BMP).
fn bmp_header(b: &[u8]) -> Option<PictureHeader> {
    let le32 = |o: usize| b.get(o..o + 4).map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let le16 = |o: usize| b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let dib = le32(14)?;
    let (width, height) = if dib == 12 { (le16(18)? as u32, le16(20)? as u32) } else if dib >= 40 { (le32(18)?.unsigned_abs(), le32(22)?.unsigned_abs()) } else { return None };
    Some(PictureHeader { format: PictureFormat::Bmp, width, height, interlaced: false, orientation: 1 })
}

/// Whether a file is a HEIC/HEIF picture, from its `ftyp` brands (not
/// AVIF, which shares the container). A HEIF image sequence counts: it
/// shows its primary still picture.
pub fn is_heic(head: &[u8]) -> bool {
    iso_picture(head) == Some(IsoPicture::Heif)
}

/// What kind of picture an ISO media file (HEIF, AVIF and MP4 share the
/// container) says it is, from the brands in its `ftyp` box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoPicture {
    /// HEIC and the other HEIF pictures and image sequences.
    Heif,
    /// AVIF, which Zebra doesn't take.
    Avif,
}

/// The picture an ISO media file's `ftyp` brands name, if any: any HEIF
/// (ISO/IEC 23008-12) or MIAF (ISO/IEC 23000-22) picture brand makes it
/// a picture, an AVIF one if any brand is AVIF's (AVIF files name `mif1`
/// too; HEIC files never name `avif`). Movie and sound brands (`isom`,
/// `mp42`, `qt  `, `M4A `…) name none. Brands are read from the box,
/// clipped to the bytes given and to the first 256 KB, one at a time.
pub fn iso_picture(head: &[u8]) -> Option<IsoPicture> {
    let (mut heif, mut avif) = (false, false);
    for b in ftyp_brands(head) {
        match b {
            b"avif" | b"avis" | b"avio" => avif = true,
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"mif2" | b"msf1" | b"miaf" | b"MiHE" | b"MiHB" | b"MiPr" | b"MiHA" => heif = true,
            _ => {}
        }
    }
    if avif {
        Some(IsoPicture::Avif)
    } else if heif {
        Some(IsoPicture::Heif)
    } else {
        None
    }
}

/// The brands of the `ftyp` box `head` starts with: the major brand, then
/// the compatible ones (the minor version between them is skipped). As
/// far as the box and `head` go, never past the first 256 KB; none if
/// `head` doesn't start with an `ftyp` box.
fn ftyp_brands(head: &[u8]) -> impl Iterator<Item = &[u8; 4]> {
    let head = &head[..head.len().min(256 << 10)];
    let body: &[u8] = (|| {
        if head.get(4..8)? != b"ftyp" {
            return None;
        }
        let (start, size) = match u32::from_be_bytes(head.get(0..4)?.try_into().ok()?) {
            0 => (8, head.len() as u64),
            1 => (16, u64::from_be_bytes(head.get(8..16)?.try_into().ok()?)),
            n => (8, n as u64),
        };
        if size < start as u64 {
            return None;
        }
        let end = size.min(head.len() as u64) as usize;
        head.get(start..end.max(start))
    })()
    .unwrap_or(&[]);
    let major = body.get(..4);
    let compatible = body.get(8..).unwrap_or(&[]).chunks_exact(4);
    major.into_iter().chain(compatible).filter_map(|b| b.try_into().ok())
}

/// The boxes inside `b` (an ISO media box's body): (type, body). Stops at
/// the end, at a broken size, or after 4,096 boxes.
fn iso_boxes(b: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut i = 0usize;
    let mut n = 0;
    std::iter::from_fn(move || {
        if n >= 4096 || i + 8 > b.len() {
            return None;
        }
        n += 1;
        let size = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as u64;
        let ty = &b[i + 4..i + 8];
        let (start, size) = match size {
            0 => (i + 8, (b.len() - i) as u64),
            1 => (i + 16, u64::from_be_bytes(b.get(i + 8..i + 16)?.try_into().ok()?)),
            s => (i + 8, s),
        };
        let end = usize::try_from(size).ok().and_then(|s| i.checked_add(s)).filter(|&e| e >= start && e <= b.len())?;
        let body = &b[start..end];
        i = end;
        Some((ty, body))
    })
}

/// A HEIC's size and turn: its primary picture's `ispe`, `irot` and
/// `imir` properties, in the order they apply.
fn heic_header(b: &[u8]) -> Option<PictureHeader> {
    let meta = iso_boxes(b).find(|(t, _)| *t == b"meta")?.1.get(4..)?;
    let mut primary = None;
    let (mut ipco, mut ipma) = (None, None);
    for (t, body) in iso_boxes(meta) {
        match t {
            b"pitm" => {
                primary = match body.first()? {
                    0 => body.get(4..6).map(|s| u16::from_be_bytes([s[0], s[1]]) as u32),
                    _ => body.get(4..8).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])),
                }
            }
            b"iprp" => {
                for (t, body) in iso_boxes(body) {
                    match t {
                        b"ipco" => ipco = Some(body),
                        b"ipma" => ipma = Some(body),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let (primary, ipco, ipma) = (primary?, ipco?, ipma?);
    let props: Vec<(&[u8], &[u8])> = iso_boxes(ipco).collect();
    // ipma: version, flags, then entries of (item, associations).
    let (version, flags) = (*ipma.first()?, *ipma.get(3)?);
    let count = u32::from_be_bytes(ipma.get(4..8)?.try_into().ok()?);
    let mut i = 8usize;
    let mut found = None;
    for _ in 0..count.min(65_535) {
        let item = if version < 1 {
            let v = u16::from_be_bytes(ipma.get(i..i + 2)?.try_into().ok()?) as u32;
            i += 2;
            v
        } else {
            let v = u32::from_be_bytes(ipma.get(i..i + 4)?.try_into().ok()?);
            i += 4;
            v
        };
        let n = *ipma.get(i)? as usize;
        i += 1;
        let mut idx = Vec::with_capacity(n);
        for _ in 0..n {
            if flags & 1 != 0 {
                idx.push((u16::from_be_bytes(ipma.get(i..i + 2)?.try_into().ok()?) & 0x7FFF) as usize);
                i += 2;
            } else {
                idx.push((*ipma.get(i)? & 0x7F) as usize);
                i += 1;
            }
        }
        if item == primary {
            found = Some(idx);
            break;
        }
    }
    let (mut width, mut height, mut orientation) = (0, 0, 1u8);
    for k in found? {
        let Some((t, body)) = k.checked_sub(1).and_then(|k| props.get(k)) else { continue };
        match *t {
            b"ispe" => {
                width = u32::from_be_bytes(body.get(4..8)?.try_into().ok()?);
                height = u32::from_be_bytes(body.get(8..12)?.try_into().ok()?);
            }
            // A turn anticlockwise, in quarters.
            b"irot" => {
                let then = match body.first()? & 3 {
                    1 => 8,
                    2 => 3,
                    3 => 6,
                    _ => 1,
                };
                orientation = then_orient(orientation, then);
            }
            // A mirror about the vertical axis (0) or the horizontal (1).
            b"imir" => orientation = then_orient(orientation, if body.first()? & 1 == 0 { 2 } else { 4 }),
            _ => {}
        }
    }
    (width > 0 && height > 0).then_some(PictureHeader { format: PictureFormat::Heic, width, height, interlaced: false, orientation })
}

/// Where the pixel at (x, y) of a picture shown with `orientation` comes
/// from in its stored `w`x`h` pixels (EXIF's eight orientations).
pub fn oriented_from(orientation: u8, (w, h): (u32, u32), (x, y): (u32, u32)) -> (u32, u32) {
    let (mx, my) = (w.saturating_sub(1), h.saturating_sub(1));
    match orientation {
        2 => (mx - x, y),
        3 => (mx - x, my - y),
        4 => (x, my - y),
        5 => (y, x),
        6 => (y, my - x),
        7 => (mx - y, my - x),
        8 => (mx - y, x),
        _ => (x, y),
    }
}

/// `first` then `then`, as one orientation: worked out on a 3x2 grid of
/// distinct pixels, so it is right by construction.
fn then_orient(first: u8, then: u8) -> u8 {
    let shown = |o: u8, (w, h): (u32, u32), at: &dyn Fn(u32, u32) -> u32| -> (Vec<u32>, (u32, u32)) {
        let (sw, sh) = if o >= 5 { (h, w) } else { (w, h) };
        let mut v = vec![];
        for y in 0..sh {
            for x in 0..sw {
                let (a, b) = oriented_from(o, (w, h), (x, y));
                v.push(at(a, b));
            }
        }
        (v, (sw, sh))
    };
    let (once, size) = shown(first, (3, 2), &|x, y| y * 3 + x);
    let (twice, _) = shown(then, size, &|x, y| once[(y * size.0 + x) as usize]);
    (1..=8).find(|&o| shown(o, (3, 2), &|x, y| y * 3 + x).0 == twice).unwrap_or(1)
}

/// A JPEG's frame header (SOFn), walking the markers before it.
fn jpeg_header(b: &[u8]) -> Option<PictureHeader> {
    let mut i = 2;
    let mut orientation = 1;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1; // fill byte
            continue;
        }
        if matches!(m, 0x01 | 0xD0..=0xD7) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        // Every start-of-frame but DHT (C4), JPG (C8) and DAC (CC).
        if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            if i + 9 > b.len() {
                return None;
            }
            let height = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let width = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            // Only baseline/extended (C0, C1) and progressive (C2) Huffman
            // JPEGs are read; the rest count as not fitting.
            let interlaced = m != 0xC0 && m != 0xC1;
            return Some(PictureHeader { format: PictureFormat::Jpeg, width, height, interlaced, orientation });
        }
        if m == 0xDA || len < 2 {
            return None;
        }
        if m == 0xE1 && orientation == 1 {
            if let Some(app1) = b.get(i + 4..i + 2 + len).filter(|a| a.starts_with(b"Exif\0\0")) {
                orientation = exif_orientation(app1);
            }
        }
        i += 2 + len;
    }
    None
}

/// What kind of media a file is, from its first bytes alone; None for
/// anything Zebra can't show. Used before copying a file in, so dropping
/// an 8 GB disk image or a log file costs a read of 256 KB, not 8 GB.
pub fn sniff_kind(head: &[u8]) -> Option<MediaKind> {
    // Before the movie sniffer: HEIC shares MP4's container. An AVIF is
    // a picture Zebra doesn't take.
    match iso_picture(head) {
        Some(IsoPicture::Heif) => return Some(MediaKind::Image),
        Some(IsoPicture::Avif) => return None,
        None => {}
    }
    if let Some(m) = sniff_av(head) {
        return Some(m.kind);
    }
    if head.starts_with(b"%PDF-") {
        return Some(MediaKind::Document);
    }
    if crate::symbols::starts_as_svg(head) {
        return Some(MediaKind::Image);
    }
    match image::guess_format(head) {
        Ok(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP | image::ImageFormat::Gif | image::ImageFormat::Tiff | image::ImageFormat::Bmp) => Some(MediaKind::Image),
        _ => None,
    }
}

/// The names (lower case, no dot) of the files [`sniff_kind`] knows: a
/// file named so that isn't one is damaged (or misnamed), not a kind
/// Zebra doesn't take. (PowerPoint files are told by their name and a
/// zip's first bytes, see `zebra_import`.)
pub const MEDIA_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "heic", "heif", "tif", "tiff", "bmp", "gif", "webp", "svg", "mp4", "mov", "m4v", "mp3", "m4a", "aac", "wav", "pdf"];

/// Why a file [`sniff_kind`] doesn't know is refused, from its first
/// bytes and its name's extension (`ext`, any case): it is empty; it is
/// an AVIF; it is named as a kind Zebra takes but isn't one ("damaged",
/// as an import says); or it is a kind Zebra doesn't take.
pub fn refusal(head: &[u8], ext: &str) -> ZError {
    let ext = ext.to_ascii_lowercase();
    if head.is_empty() {
        ZError::new("damaged", "the file is empty")
    } else if iso_picture(head) == Some(IsoPicture::Avif) {
        ZError::invalid("AVIF pictures aren't supported").hint(FORMATS_HINT)
    } else if MEDIA_EXTENSIONS.contains(&ext.as_str()) {
        ZError::new("damaged", format!("the file is damaged or isn't a .{ext} file"))
    } else {
        ZError::invalid("unrecognised media format").hint(FORMATS_HINT)
    }
}

/// The formats Zebra takes, for a refusal's hint.
const FORMATS_HINT: &str = "use PNG, JPEG, HEIC, TIFF, BMP, WebP, GIF, SVG, MP4, MOV, MP3, M4A, AAC or WAV";

/// A private copy of `path`, mapped, that nothing else can change: the
/// copy's name is removed at once (the mapping keeps its bytes until it
/// goes), so another program rewriting or truncating `path` afterwards
/// can't crash Zebra with SIGBUS. Known by `path`, not owned.
pub fn private_copy(path: &Path) -> std::io::Result<MappedFile> {
    regular_file(path)?;
    let dir = stream_dir();
    std::fs::create_dir_all(&dir)?;
    let part = spill_path(&dir, "open");
    let r = std::fs::copy(path, &part).and_then(|_| MappedFile::open_unlinked(&part));
    let _ = std::fs::remove_file(&part);
    Ok(r?.renamed(path))
}

/// Give a finished spill file its final name and map it (owned).
fn finish_spill(dir: &Path, part: PathBuf, key: String, ext: &str) -> std::io::Result<(String, AssetBytes)> {
    let to = spill_path(dir, &format!("{key}.{ext}"));
    if let Err(e) = std::fs::rename(&part, &to) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    match MappedFile::owned(to.clone()) {
        Ok(m) => Ok((key, m.into())),
        Err(e) => {
            let _ = std::fs::remove_file(&to);
            Err(e)
        }
    }
}

/// A spill file named for what it turned out to hold (`.mov` copied in,
/// found to be an MP4): players go by the name's extension, and a file
/// whose name is right is played as it is instead of being copied. Bytes
/// that aren't a spill file of their own, or are shared, come back as
/// they were.
pub fn set_ext(bytes: AssetBytes, ext: &str) -> AssetBytes {
    let AssetBytes::Mapped { map, start: 0, len } = bytes else { return bytes };
    let named_right = map.path().and_then(|p| p.extension()).is_some_and(|e| e.eq_ignore_ascii_case(ext));
    if named_right || !map.is_owned() || len != map.len() || check_ext(ext).is_err() {
        return AssetBytes::Mapped { map, start: 0, len };
    }
    match std::sync::Arc::try_unwrap(map) {
        Ok(mut m) => {
            if let Some(p) = m.path.clone() {
                let to = p.with_extension(ext);
                if std::fs::rename(&p, &to).is_ok() {
                    m.path = Some(to);
                }
            }
            m.into()
        }
        Err(map) => AssetBytes::Mapped { map, start: 0, len },
    }
}

/// The process that made a spill or cache file, from its name
/// (`<pid>-...`). None for names of older builds.
fn file_owner(name: &str) -> Option<u32> {
    name.split('-').next()?.parse().ok()
}

/// Spill files no running Zebra needs (Mac pass M19: a 6 GB picture
/// outlived its documents in `Autosave/Media`), from the spill folder and
/// the temporary one: see [`collect_spill`].
pub fn sweep_spill(made_by_live: impl Fn(u32, Option<std::time::SystemTime>) -> bool) -> Collected {
    let mut all = Collected::default();
    for dir in spill_dir().into_iter().chain(std::iter::once(std::env::temp_dir().join("Zebra-media"))) {
        let c = collect_spill(&dir, &made_by_live);
        all.files += c.files;
        all.bytes += c.bytes;
    }
    all
}

/// What [`collect_spill`] removed.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Collected {
    pub files: usize,
    pub bytes: u64,
}

/// Remove the spill files in `dir` whose maker is gone. A spill file
/// (`<pid>-<n>-<rest>`) is needed only by the process that made it: an
/// autosave keeps its media in its own folder, as hard links or copies,
/// and recovery never reads the spill folder. So a file goes when
/// `made_by_live(pid, t)` says no live Zebra made it, `t` being when the
/// file was made (its status-change time: a copy keeps the original's
/// modified time, and the status time can't be set back). Never:
/// - anything of this process (its mappings remove their own files, and
///   one may be half written);
/// - anything not named as a spill file, nor anything but a plain file
///   (a link, a folder, a pipe), nor another user's file;
/// - anything through a link: `dir` itself must be a real folder.
///
/// Errors are skipped (the next run tries again).
pub fn collect_spill(dir: &Path, made_by_live: impl Fn(u32, Option<std::time::SystemTime>) -> bool) -> Collected {
    let mut out = Collected::default();
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return out;
    }
    let me = std::process::id();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(spill_owner) else { continue };
        if pid == me {
            continue;
        }
        // Neither follows a link.
        let (Ok(kind), Ok(meta)) = (e.file_type(), e.metadata()) else { continue };
        if !kind.is_file() {
            continue;
        }
        #[cfg(unix)]
        let made = {
            use std::os::unix::fs::MetadataExt;
            // SAFETY: getuid never fails.
            if meta.uid() != unsafe { libc::getuid() } {
                continue;
            }
            u64::try_from(meta.ctime()).ok().map(|s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s))
        };
        #[cfg(not(unix))]
        let made = meta.modified().ok();
        if made_by_live(pid, made) {
            continue;
        }
        if std::fs::remove_file(e.path()).is_ok() {
            out.files += 1;
            out.bytes += meta.len();
        }
    }
    out
}

/// The process named by a spill file's name, `<pid>-<n>-<rest>` (digits,
/// digits, then anything). None for every other name.
fn spill_owner(name: &str) -> Option<u32> {
    let mut parts = name.splitn(3, '-');
    let (pid, n, rest) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(pid) || !digits(n) || rest.is_empty() {
        return None;
    }
    pid.parse().ok()
}

// ----------------------------------------------------------------------
// Players' files

/// A plain file a player opens (players take a path, not bytes): the
/// asset's own file when it has one, else a copy in [`cache_dir`]. A copy
/// is removed when the last `PlayFile` for it goes (the window closes,
/// another file opens, the app quits), and a crashed run's copies at the
/// next launch ([`sweep_cache`]).
#[derive(Debug)]
pub struct PlayFile {
    path: PathBuf,
    /// The asset's own file, kept alive while it plays.
    _keep: Option<std::sync::Arc<MappedFile>>,
    copy: bool,
}

impl PlayFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PlayFile {
    fn drop(&mut self) {
        if self.copy {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Copies being played, by `<key>.<ext>`, so two players (two windows,
/// a video shown twice) share one.
static PLAYING: std::sync::Mutex<Option<std::collections::HashMap<String, std::sync::Weak<PlayFile>>>> = std::sync::Mutex::new(None);

/// Refuse a key or extension that could name a file outside the cache.
fn check_cache_name(key: &str, ext: &str) -> std::io::Result<()> {
    if crate::check::safe_asset_key(key) {
        return check_ext(ext);
    }
    Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a media asset name"))
}

/// A plain file holding the asset, for a player. Free when the asset has
/// a file of its own (new media: its spill file); otherwise the bytes
/// (mapped from the `.zebra`) are copied once, streamed, so this costs
/// disk, not memory. Written to a temp name and renamed, so a
/// half-written file is never played. Keep the result while the file is
/// in use: the copy goes with the last one.
pub fn materialize(key: &str, ext: &str, bytes: &AssetBytes) -> std::io::Result<std::sync::Arc<PlayFile>> {
    check_cache_name(key, ext)?;
    if let (Some(p), Some(map)) = (bytes.own_file(), bytes.mapped_file())
        && p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
        && std::fs::metadata(p).is_ok_and(|m| m.len() == bytes.len() as u64)
    {
        return Ok(std::sync::Arc::new(PlayFile { path: p.to_path_buf(), _keep: Some(map.clone()), copy: false }));
    }
    let name = format!("{key}.{ext}");
    // One copy at a time: two players of one clip wait for one copy.
    let mut playing = PLAYING.lock().unwrap_or_else(|e| e.into_inner());
    let playing = playing.get_or_insert_with(Default::default);
    playing.retain(|_, w| w.strong_count() > 0);
    if let Some(f) = playing.get(&name).and_then(|w| w.upgrade()) {
        return Ok(f);
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}-{name}", std::process::id()));
    let tmp = spill_path(&dir, "part");
    let r = std::fs::write(&tmp, &bytes[..]).and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let f = std::sync::Arc::new(PlayFile { path, _keep: None, copy: true });
    playing.insert(name, std::sync::Arc::downgrade(&f));
    Ok(f)
}

/// Players' copies a run left behind: those of processes that aren't
/// running (it crashed, or quit without tidying up) and every file of
/// builds before copies were tidied up (`<key>.<ext>`, the sample clip).
/// The same goes for web video downloads in `downloads/`
/// (`<pid>-<n>.<ext>` and its `.part`), which a run that dies
/// mid-download leaves behind.
pub fn sweep_cache(alive: impl Fn(u32) -> bool) {
    let dir = cache_dir();
    for d in [dir.clone(), dir.join("downloads")] {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !file_owner(&name).is_some_and(&alive) && e.path().is_file() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Tiny media files for tests (headers only; not playable).
#[doc(hidden)]
pub mod fixtures {
    fn bx(typ: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(typ);
        v.extend_from_slice(body);
        v
    }

    /// A minimal MP4: ftyp + moov(mvhd, trak(tkhd, mdia(hdlr vide))) + mdat.
    /// Enough for header parsing, not for playing.
    pub fn tiny_mp4(w: u32, h: u32, secs: u32, video: bool) -> Vec<u8> {
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&(secs * 1000).to_be_bytes());
        let mut tkhd = vec![0u8; 84];
        let n = tkhd.len();
        tkhd[n - 8..n - 4].copy_from_slice(&(w << 16).to_be_bytes());
        tkhd[n - 4..n].copy_from_slice(&(h << 16).to_be_bytes());
        let mut hdlr = vec![0u8; 24];
        hdlr[8..12].copy_from_slice(if video { b"vide" } else { b"soun" });
        let trak = bx(b"trak", &[bx(b"tkhd", &tkhd), bx(b"mdia", &bx(b"hdlr", &hdlr))].concat());
        let moov = bx(b"moov", &[bx(b"mvhd", &mvhd), trak].concat());
        let mut ftyp_body = b"isom".to_vec();
        ftyp_body.extend_from_slice(&[0, 0, 2, 0]);
        ftyp_body.extend_from_slice(b"isomavc1");
        [bx(b"ftyp", &ftyp_body), bx(b"mdat", &[0u8; 64]), moov].concat()
    }

    pub fn tiny_wav(secs: f64) -> Vec<u8> {
        let rate = 8000u32;
        let n = (rate as f64 * secs) as u32;
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(36 + n).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // mono
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes()); // byte rate (8-bit mono)
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&8u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&n.to_le_bytes());
        v.extend(std::iter::repeat_n(128u8, n as usize));
        v
    }

}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn mp4_header_with_moov_at_end() {
        let m = info(&tiny_mp4(1920, 1080, 12, true)).unwrap();
        assert_eq!((m.ext.as_str(), m.kind, m.width, m.height), ("mp4", MediaKind::Video, 1920, 1080));
        assert!((m.duration - 12.0).abs() < 1e-9);
    }

    /// An `mvhd` box (version 0) saying `duration` units of 1/`scale` s.
    fn mvhd_v0(scale: u32, duration: u32) -> Vec<u8> {
        let mut b = vec![0u8; 20];
        b[12..16].copy_from_slice(&scale.to_be_bytes());
        b[16..20].copy_from_slice(&duration.to_be_bytes());
        b
    }

    #[test]
    fn a_lying_or_unknown_length_is_unknown() {
        // 20 Nov audit, W10: timescale 1, duration 0xffffffff was believed
        // (136 years).
        assert_eq!(mvhd_duration(&mvhd_v0(1, u32::MAX)), None);
        assert_eq!(mvhd_duration(&mvhd_v0(1000, u32::MAX)), None);
        assert_eq!(mvhd_duration(&mvhd_v0(1, u32::MAX - 1)), Some(0.0));
        assert_eq!(mvhd_duration(&mvhd_v0(1, 86_400)), Some(86_400.0));
        assert_eq!(mvhd_duration(&mvhd_v0(1, 86_401)), Some(0.0));
        assert_eq!(mvhd_duration(&mvhd_v0(600, 600 * 90)), Some(90.0));
        assert_eq!(mvhd_duration(&mvhd_v0(0, 5)), None);
        let mut v1 = vec![0u8; 32];
        v1[0] = 1;
        v1[20..24].copy_from_slice(&1u32.to_be_bytes());
        v1[24..32].copy_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(mvhd_duration(&v1), None);
        v1[24..32].copy_from_slice(&(1u64 << 40).to_be_bytes());
        assert_eq!(mvhd_duration(&v1), Some(0.0));
        for d in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1e12] {
            assert_eq!(plausible_duration(d), 0.0);
        }
        // A WAV whose byte rate is 1 says 4 GB of data lasts 136 years.
        let mut w = tiny_wav(0.01);
        let rate = w.windows(4).position(|x| x == b"fmt ").unwrap() + 16;
        w[rate..rate + 4].copy_from_slice(&1u32.to_le_bytes());
        let data = w.windows(4).position(|x| x == b"data").unwrap() + 4;
        w[data..data + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(info(&w).unwrap().duration, 0.0);
    }

    #[test]
    fn audio_only_mp4_is_m4a() {
        let m = info(&tiny_mp4(0, 0, 3, false)).unwrap();
        assert_eq!((m.ext.as_str(), m.kind), ("m4a", MediaKind::Audio));
    }

    #[test]
    fn wav_mp3_aac() {
        let w = info(&tiny_wav(0.5)).unwrap();
        assert_eq!((w.ext.as_str(), w.kind), ("wav", MediaKind::Audio));
        assert!((w.duration - 0.5).abs() < 1e-3);
        assert_eq!(info(b"ID3\x04\0\0\0\0\0\0").unwrap().ext, "mp3");
        assert_eq!(info(&[0xFF, 0xFB, 0x90, 0x00]).unwrap().ext, "mp3");
        assert_eq!(info(&[0xFF, 0xF1, 0x50, 0x80]).unwrap().ext, "aac");
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(info(b"hello world, not media").is_err());
        // Truncated / hostile box sizes must not panic.
        let mut v = tiny_mp4(10, 10, 1, true);
        v.truncate(40);
        let _ = info(&v);
        let _ = info(&[0, 0, 0, 1, b'f', b't', b'y', b'p', 0xff, 0xff]);
    }
}
