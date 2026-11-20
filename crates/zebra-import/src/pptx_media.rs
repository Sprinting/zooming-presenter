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

/// A media part (or any part) is at most this many times the size it
/// takes in the file: a part that unpacks to more is a zip bomb, not a
/// picture. Media is stored unpacked by PowerPoint (ratio 1).
const MAX_RATIO: u64 = 200;
/// Pictures that are turned into something else (EMF, WMF, SVG) are read
/// into memory to be converted; bigger ones are left out.
pub const MAX_CONVERTED: u64 = 64 * 1024 * 1024;

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
    /// it is missing, can't be unpacked, or is bigger.
    pub fn read_part(&self, path: &str, max: u64) -> Option<Vec<u8>> {
        self.with_entry(path, |r, limit| {
            let mut v = Vec::with_capacity(limit.min(max).min(1 << 20) as usize);
            r.take(max.saturating_add(1)).read_to_end(&mut v).ok()?;
            (v.len() as u64 <= max).then_some(v)
        })
    }

    /// Run `f` with a reader of part `path`, limited to what it may
    /// unpack to (a bomb errors out partway instead of filling the disk).
    fn with_part<T>(&self, path: &str, f: impl FnOnce(&mut dyn Read, u64) -> Option<T>) -> Option<T> {
        if !is_media(path) {
            return None;
        }
        self.with_entry(path, f)
    }

    /// [`Self::with_part`] for any part.
    fn with_entry<T>(&self, path: &str, f: impl FnOnce(&mut dyn Read, u64) -> Option<T>) -> Option<T> {
        let i = *self.entries.get(path)?;
        let mut zip = self.zip.lock().unwrap_or_else(|e| e.into_inner());
        let entry = zip.by_index(i).ok()?;
        let limit = entry.size().min(entry.compressed_size().saturating_mul(MAX_RATIO).max(1 << 20));
        let mut r = entry.take(limit);
        f(&mut r, limit)
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

/// A safe file extension for part `path` ("bin" if it has none usable).
fn part_ext(path: &str) -> String {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    if !ext.is_empty() && ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        ext
    } else {
        "bin".into()
    }
}
