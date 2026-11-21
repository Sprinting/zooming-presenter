//! A PowerPoint file's parts, read straight out of the (memory-mapped)
//! file as they are needed.
//!
//! BetterOffice's parser takes a whole file and unpacks every part into
//! memory: a deck with a big video either failed to import ("damaged") or
//! held the video in memory five times. So the parser is only ever given
//! small zips made here of the parts a few slides need, with the media
//! left in as empty entries (see [`crate::pptx_package`]). The media is
//! read here, part by part: big parts are streamed into the document's
//! spill files (disk, not memory), small ones read.
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Mutex;

use zebra_core::import::{asset_from, asset_from_bytes, av_asset_from_bytes};
use zebra_core::media::{MediaInfo, SPILL_MIN};
use zebra_core::{Asset, AssetBytes};

use crate::ImportError;

/// Pictures that are turned into something else (EMF, WMF, SVG) are read
/// into memory to be converted: as big as memory allows (the conversion
/// and drawing take several times the file), with no other limit (B9).
/// Bigger ones are left out with a note.
pub fn max_converted() -> u64 {
    zebra_core::io::memory_budget() / 16
}

/// Is this part a media part (never given to the parser)?
pub(crate) fn is_media(name: &str) -> bool {
    name.len() > 10 && name.is_char_boundary(10) && name[..10].eq_ignore_ascii_case("ppt/media/") && !name.ends_with('/')
}

/// A video or sound taken out of the deck: key, asset, what it is.
type Taken = Option<(String, Asset, MediaInfo)>;

pub struct MediaSource {
    zip: Mutex<zip::ZipArchive<Cursor<AssetBytes>>>,
    /// Every part's path -> index in the zip.
    entries: HashMap<String, usize>,
    /// Videos and sounds taken out already, by part path (one used on
    /// several slides is copied once).
    av: Mutex<HashMap<String, Taken>>,
}

impl MediaSource {
    /// Read the file's directory (not its parts).
    pub fn open(file: AssetBytes) -> Result<MediaSource, ImportError> {
        let damaged = |_| ImportError::Damaged("PowerPoint");
        let mut zip = zip::ZipArchive::new(Cursor::new(file)).map_err(damaged)?;
        let mut entries = HashMap::new();
        for i in 0..zip.len() {
            let f = zip.by_index_raw(i).map_err(damaged)?;
            let name = f.name().map_err(damaged)?.to_string();
            if !name.ends_with('/') {
                entries.entry(name).or_insert(i);
            }
        }
        Ok(MediaSource { zip: Mutex::new(zip), entries, av: Mutex::new(HashMap::new()) })
    }

    /// The size part `path` says it unpacks to.
    pub fn size(&self, path: &str) -> Option<u64> {
        let i = *self.entries.get(path)?;
        let mut zip = self.zip.lock().unwrap_or_else(|e| e.into_inner());
        zip.by_index_raw(i).ok().map(|f| f.size())
    }

    /// Is there a part at `path`?
    pub fn has(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    /// Copy part `path` into `out` as it is in the file (still compressed:
    /// nothing is unpacked).
    pub fn copy_raw(&self, path: &str, out: &mut zip::ZipWriter<Cursor<Vec<u8>>>) -> bool {
        let Some(&i) = self.entries.get(path) else { return false };
        let mut zip = self.zip.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(f) = zip.by_index_raw(i) else { return false };
        out.raw_copy_file(f).is_ok()
    }

    /// Any part's bytes (not only media), at most `max` unpacked; None when
    /// it is missing, can't be unpacked, or is bigger. `max` is the only
    /// bound: XML packs tightly (a slide of 20,000 alike runs, 266 to 1).
    pub fn read_part(&self, path: &str, max: u64) -> Option<Vec<u8>> {
        let i = *self.entries.get(path)?;
        let mut zip = self.zip.lock().unwrap_or_else(|e| e.into_inner());
        let entry = zip.by_index(i).ok()?;
        if entry.size() > max {
            return None;
        }
        let mut v = Vec::with_capacity(entry.size().min(1 << 20) as usize);
        entry.take(max.saturating_add(1)).read_to_end(&mut v).ok()?;
        (v.len() as u64 <= max).then_some(v)
    }

    /// Run `f` with a reader of media part `path` and the size it says it
    /// unpacks to. None when that is more than it may unpack to (a bomb,
    /// not media); the reader fails, rather than ends, where the part
    /// turns out longer or shorter than it says, so a part is never taken
    /// in cut short (a 2 GB bomb video, cut at 398 MB, came in as a sound).
    fn with_part<T>(&self, path: &str, f: impl FnOnce(&mut dyn Read, u64) -> Option<T>) -> Option<T> {
        if !is_media(path) {
            return None;
        }
        let i = *self.entries.get(path)?;
        let mut zip = self.zip.lock().unwrap_or_else(|e| e.into_inner());
        let entry = zip.by_index(i).ok()?;
        // However well it packed: nothing is cut (B8). A big part streams
        // to a spill file, which stops before the disk fills.
        let size = entry.size();
        f(&mut Exact { inner: entry, left: size }, size)
    }

    /// The bytes of a part of at most `max` bytes, read into memory.
    pub fn read(&self, path: &str, max: u64) -> Option<Vec<u8>> {
        self.with_part(path, |r, limit| {
            if limit > max {
                return None;
            }
            let mut v = Vec::with_capacity(limit as usize);
            r.read_to_end(&mut v).ok()?;
            Some(v)
        })
    }

    /// A part as a new asset's bytes and key: read when small, streamed to
    /// a spill file when big. `ext` names the spill file.
    fn bytes(&self, path: &str, ext: &str) -> Option<(String, AssetBytes)> {
        self.with_part(path, |r, limit| {
            if limit < SPILL_MIN as u64 {
                let mut v = vec![];
                r.read_to_end(&mut v).ok()?;
                return Some((zebra_core::media::content_key(&v), AssetBytes::from(v)));
            }
            zebra_core::media::spill_reader(r, ext).ok()
        })
    }

    /// A picture part (PNG, JPEG, GIF, WebP) as an asset, with its key.
    pub fn picture(&self, path: &str) -> Option<(String, Asset)> {
        let ext = part_ext(path);
        let (key, bytes) = self.bytes(path, &ext)?;
        if !bytes.is_mapped() {
            let v = bytes.to_vec();
            return asset_from(v).ok();
        }
        let a = asset_from_bytes(bytes).ok()?;
        Some((key, Asset { bytes: zebra_core::media::set_ext(a.bytes, &a.ext), ..a }))
    }

    /// A video or sound part as an asset, with its key and what its header
    /// says. Each part is taken out once.
    pub fn av(&self, path: &str) -> Option<(String, Asset, MediaInfo)> {
        if let Some(done) = self.av.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
            return done.clone();
        }
        let got = (|| {
            let (key, bytes) = self.bytes(path, &part_ext(path))?;
            let probe = bytes.clone();
            let (a, info) = av_asset_from_bytes(bytes, &probe).ok()?;
            drop(probe);
            let a = Asset { bytes: zebra_core::media::set_ext(a.bytes, &a.ext), ..a };
            Some((key, a, info))
        })();
        self.av.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_string(), got.clone());
        got
    }
}

/// A reader of exactly `left` more bytes of `inner`: an error, not the
/// end, when `inner` gives fewer or more.
struct Exact<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Exact<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::io::{Error, ErrorKind};
        if self.left == 0 {
            // At the size it said: anything more means it lied.
            let mut one = [0u8; 1];
            return match self.inner.read(&mut one)? {
                0 => Ok(0),
                _ => Err(Error::new(ErrorKind::InvalidData, "part longer than it says")),
            };
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let want = buf.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        match self.inner.read(&mut buf[..want])? {
            0 => Err(Error::new(ErrorKind::UnexpectedEof, "part shorter than it says")),
            n => {
                self.left -= n as u64;
                Ok(n)
            }
        }
    }
}

/// A safe file extension for part `path` ("bin" if it has none usable).
fn part_ext(path: &str) -> String {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    if !ext.is_empty() && ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        ext
    } else {
        "bin".into()
    }
}

#[cfg(test)]
mod tests {
    use super::Exact;
    use std::io::Read;

    fn read(bytes: &[u8], says: u64) -> std::io::Result<Vec<u8>> {
        let mut v = vec![];
        Exact { inner: bytes, left: says }.read_to_end(&mut v).map(|_| v)
    }

    #[test]
    fn a_part_is_read_whole_or_not_at_all() {
        assert_eq!(read(b"abcd", 4).unwrap(), b"abcd");
        assert_eq!(read(b"", 0).unwrap(), b"");
        // Longer or shorter than it says: an error, never a cut part.
        assert_eq!(read(b"abcdef", 4).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(read(b"ab", 4).unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
