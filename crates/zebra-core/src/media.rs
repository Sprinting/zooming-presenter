//! Media files: sniffing what an asset is, reading video size and length
//! from MP4/QuickTime headers, and the on-disk media cache.
//!
//! Only headers are read here. Decoding is the platform's job
//! (`zebra-media`), so nothing in this module allocates per frame or
//! touches more than a few kilobytes of a file.
use std::path::{Path, PathBuf};

use crate::error::{ZError, ZResult};
use crate::model::MediaKind;

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

/// Where video and audio live as plain files, so the platform player can
/// open them by URL: `$ZEBRA_MEDIA_CACHE`, else the user cache directory
/// (`~/Library/Caches/Zebra/media` on macOS, inside the sandbox container
/// when sandboxed).
pub fn cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ZEBRA_MEDIA_CACHE") {
        return PathBuf::from(d);
    }
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    base.join("Zebra").join("media")
}

/// A temp name next to `path` that no other writer uses, in this process
/// or another: two imports of the same clip at once (same cache key) each
/// write their own copy, and the second rename simply replaces the first.
fn part_path(path: &Path, ext: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    path.with_extension(format!("{ext}.part{}-{n}", std::process::id()))
}

/// The cache file for an asset.
pub fn cache_path(key: &str, ext: &str) -> PathBuf {
    cache_dir().join(format!("{key}.{ext}"))
}

/// Make sure the asset exists as a plain file and return its path. Copies
/// at most once per asset (streams from the mapped `.zebra`, so this costs
/// disk, not RAM). Written to a temp name and renamed, so a half-written
/// file is never played.
pub fn materialize(key: &str, ext: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let path = cache_path(key, ext);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64) {
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = part_path(&path, ext);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Copy a media file into the cache (`clonefile` on APFS, so usually free)
/// and memory-map the copy. Mapping the copy, not the original, means the
/// person can move or edit the original without disturbing the document.
pub fn import_file(src: &Path, key: &str, ext: &str) -> std::io::Result<memmap2::Mmap> {
    let path = cache_path(key, ext);
    let len = std::fs::metadata(src)?.len();
    if !std::fs::metadata(&path).is_ok_and(|m| m.len() == len) {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = part_path(&path, ext);
        std::fs::copy(src, &tmp)?;
        std::fs::rename(&tmp, &path)?;
    }
    let f = std::fs::File::open(&path)?;
    // SAFETY: read-only map of a file only Zebra writes, and Zebra never
    // rewrites a cache file in place (see `materialize`).
    unsafe { memmap2::Mmap::map(&f) }
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
