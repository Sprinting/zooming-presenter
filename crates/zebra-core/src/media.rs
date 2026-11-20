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
            ZError::invalid("unrecognised media format").hint("use PNG, JPEG, WebP, GIF, MP4, MOV, MP3, M4A, AAC or WAV")
        } else {
            e
        }
    })?;
    Ok(MediaInfo { ext, kind: MediaKind::Image, width: w, height: h, duration: 0.0 })
}

fn sniff_av(b: &[u8]) -> Option<MediaInfo> {
    let audio = |ext: &str, duration: f64| MediaInfo { ext: ext.into(), kind: MediaKind::Audio, width: 0, height: 0, duration };
    if b.len() >= 12 && &b[4..8] == b"ftyp" {
        let brand = &b[8..12];
        let mov = parse_mov(b);
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
        let is_video = find_box(trak, b"mdia")
            .and_then(|mdia| find_box(mdia, b"hdlr"))
            .is_some_and(|hdlr| hdlr.len() >= 12 && &hdlr[8..12] == b"vide");
        if is_video {
            out.has_video = true;
            if let Some(tkhd) = find_box(trak, b"tkhd") {
                // width and height are the last 8 bytes, 16.16 fixed point.
                if tkhd.len() >= 8 {
                    let n = tkhd.len();
                    out.width = u32::from_be_bytes(tkhd[n - 8..n - 4].try_into().ok()?) >> 16;
                    out.height = u32::from_be_bytes(tkhd[n - 4..n].try_into().ok()?) >> 16;
                }
            }
            break;
        }
    }
    Some(out)
}

fn mvhd_duration(m: &[u8]) -> Option<f64> {
    let version = *m.first()?;
    let (ts, dur) = if version == 1 {
        (u32::from_be_bytes(m.get(20..24)?.try_into().ok()?) as f64, u64::from_be_bytes(m.get(24..32)?.try_into().ok()?) as f64)
    } else {
        (u32::from_be_bytes(m.get(12..16)?.try_into().ok()?) as f64, u32::from_be_bytes(m.get(16..20)?.try_into().ok()?) as f64)
    };
    (ts > 0.0).then(|| dur / ts)
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
            return (byte_rate > 0).then(|| len as f64 / byte_rate as f64);
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
}

impl MappedFile {
    /// Map `path` read-only. Not owned: the file stays when this goes.
    pub fn open(path: &Path) -> std::io::Result<MappedFile> {
        let f = std::fs::File::open(path)?;
        // SAFETY: read-only map. Zebra never writes a file it has mapped
        // (saves write a new file and rename it over); see `AssetBytes`.
        let map = unsafe { memmap2::Mmap::map(&f) }?;
        Ok(MappedFile { map, path: Some(path.to_path_buf()), owned: false })
    }

    /// Map a file only Zebra knows (its own spill file or copy), which is
    /// removed when the mapping goes.
    fn owned(path: PathBuf) -> std::io::Result<MappedFile> {
        let f = std::fs::File::open(&path)?;
        // SAFETY: as in `open`; nobody else writes Zebra's own files.
        let map = unsafe { memmap2::Mmap::map(&f) }?;
        Ok(MappedFile { map, path: Some(path), owned: true })
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
pub fn spill_reader(r: &mut dyn std::io::Read, ext: &str) -> std::io::Result<(String, AssetBytes)> {
    check_ext(ext)?;
    let dir = stream_dir();
    std::fs::create_dir_all(&dir)?;
    let part = spill_path(&dir, "part");
    let mut key = KeyHasher::default();
    let r = (|| -> std::io::Result<()> {
        use std::io::Write;
        let f = std::fs::OpenOptions::new().write(true).create_new(true).open(&part)?;
        let mut w = std::io::BufWriter::with_capacity(1 << 20, f);
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            key.update(&buf[..n]);
            w.write_all(&buf[..n])?;
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

/// Largest picture file Zebra takes in (a PNG or JPEG this big is a scan
/// of a poster; anything larger is a mistake or an attack).
pub const MAX_PICTURE: u64 = 256 << 20;

/// What kind of media a file is, from its first bytes alone; None for
/// anything Zebra can't show. Used before copying a file in, so dropping
/// an 8 GB disk image or a log file costs a read of 256 KB, not 8 GB.
pub fn sniff_kind(head: &[u8]) -> Option<MediaKind> {
    if let Some(m) = sniff_av(head) {
        return Some(m.kind);
    }
    if head.starts_with(b"%PDF-") {
        return Some(MediaKind::Document);
    }
    match image::guess_format(head) {
        Ok(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP | image::ImageFormat::Gif) => Some(MediaKind::Image),
        _ => None,
    }
}

/// A private copy of `path`, mapped, that nothing else can change: the
/// copy's name is removed at once (the mapping keeps its bytes until it
/// goes), so another program rewriting or truncating `path` afterwards
/// can't crash Zebra with SIGBUS. Known by `path`, not owned.
pub fn private_copy(path: &Path) -> std::io::Result<MappedFile> {
    regular_file(path)?;
    let dir = stream_dir();
    std::fs::create_dir_all(&dir)?;
    let part = spill_path(&dir, "open");
    let r = std::fs::copy(path, &part).and_then(|_| MappedFile::open(&part));
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

/// Spill files a crashed run left: those of processes that aren't running.
/// `alive` says whether a pid is.
pub fn sweep_spill(alive: impl Fn(u32) -> bool) {
    for dir in spill_dir().into_iter().chain(std::iter::once(std::env::temp_dir().join("Zebra-media"))) {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if file_owner(&name).is_some_and(|p| !alive(p)) && e.path().is_file() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
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
pub fn sweep_cache(alive: impl Fn(u32) -> bool) {
    for e in std::fs::read_dir(cache_dir()).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !file_owner(&name).is_some_and(&alive) && e.path().is_file() {
            let _ = std::fs::remove_file(e.path());
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
