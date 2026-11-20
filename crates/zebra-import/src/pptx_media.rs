//! A PowerPoint file's media, streamed out of the (memory-mapped) file.
//!
//! BetterOffice's parser takes the whole file and unpacks every part into
//! memory, twice over, with a ceiling of 512 MB unpacked: a deck with a
//! big video either failed to import ("damaged") or held the video in
//! memory five times. So the parser gets a copy of the file *without* its
//! media (`ppt/media/*` left in as empty entries, so every link still
//! resolves): XML only, a few megabytes at most. The media is read here,
//! part by part, straight out of the file: big parts are streamed into
//! the document's spill files (disk, not memory), small ones read.
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

/// Is this part a media part (kept out of what the parser sees)?
fn is_media(name: &str) -> bool {
    name.len() > 10 && name[..10].eq_ignore_ascii_case("ppt/media/") && !name.ends_with('/')
}

/// A video or sound taken out of the deck: key, asset, what it is.
type Taken = Option<(String, Asset, MediaInfo)>;

pub struct MediaSource {
    zip: Mutex<zip::ZipArchive<Cursor<AssetBytes>>>,
    /// Media part path -> index in the zip.
    parts: HashMap<String, usize>,
    /// Videos and sounds taken out already, by part path (one used on
    /// several slides is copied once).
    av: Mutex<HashMap<String, Taken>>,
}

impl MediaSource {
    /// Read the file's directory; return it and the file without its media,
    /// for the parser.
    pub fn open(file: AssetBytes) -> Result<(MediaSource, Vec<u8>), ImportError> {
        let damaged = |_| ImportError::Damaged("PowerPoint");
        let mut zip = zip::ZipArchive::new(Cursor::new(file)).map_err(damaged)?;
        let mut slim = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let mut parts = HashMap::new();
        for i in 0..zip.len() {
            let f = zip.by_index_raw(i).map_err(damaged)?;
            let name = f.name().map_err(damaged)?.to_string();
            if is_media(&name) {
                parts.insert(name.clone(), i);
                drop(f);
                let empty = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
                slim.start_file(name, empty).map_err(damaged)?;
            } else {
                slim.raw_copy_file(f).map_err(damaged)?;
            }
        }
        let slim = slim.finish().map_err(damaged)?.into_inner();
        Ok((MediaSource { zip: Mutex::new(zip), parts, av: Mutex::new(HashMap::new()) }, slim))
    }

    /// Run `f` with a reader of part `path`, limited to what it may
    /// unpack to (a bomb errors out partway instead of filling the disk).
    fn with_part<T>(&self, path: &str, f: impl FnOnce(&mut dyn Read, u64) -> Option<T>) -> Option<T> {
        let i = *self.parts.get(path)?;
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
