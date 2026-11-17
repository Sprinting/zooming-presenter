//! The clip-art library behind Insert › Symbols & Shapes: about a hundred
//! thousand SVG symbols in sets (emoji, colour icons, hand-drawn, people,
//! one-colour glyphs, classic clip art), built by
//! `tools/symbols/build_pack.py`.
//!
//! On disk each set is an index (`<key>.idx`: name, keywords, offset,
//! length) and the SVGs back to back (`<key>.dat`). The indexes stay in
//! memory for search; a symbol's SVG is read from disk when it is first
//! drawn or inserted, kept while it is used and dropped after
//! [`IDLE`] without use.
//!
//! An inserted symbol is an image whose asset is the SVG itself, so a deck
//! draws the same on a Mac without the library.
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A cached SVG not used for this long is dropped from memory.
pub const IDLE: Duration = Duration::from_secs(10 * 60);

/// Pixel size given to the long side of an SVG asset. Pictures are drawn
/// from mip levels of this size, so a symbol stays sharp until it is about
/// this many pixels across on screen.
pub const SVG_PX: f64 = 4096.0;

/// How a set's colours can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tint {
    /// Drawn in its own colours.
    #[default]
    #[serde(rename = "")]
    None,
    /// One colour (`currentColor`), which can follow the theme.
    Mono,
    /// IconPark: an outline colour and a fill colour.
    Two,
}

/// One set, as `sets.json` describes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetInfo {
    /// Catalogue id (C1..C36).
    pub id: String,
    /// File stem and the name used in documents and the API ("noto").
    pub key: String,
    pub name: String,
    /// emoji, colour, drawn, people, mono, classic.
    pub group: String,
    #[serde(default)]
    pub tint: Tint,
    pub author: String,
    #[serde(default)]
    pub author_url: String,
    pub licence: String,
    #[serde(default)]
    pub licence_url: String,
    /// The licence asks for a credit line (shown in Acknowledgements).
    #[serde(default)]
    pub credit: bool,
    pub count: usize,
}

struct Entry {
    name: Box<str>,
    keywords: Box<str>,
    offset: u64,
    len: u32,
}

struct Set {
    info: SetInfo,
    entries: Vec<Entry>,
    by_name: HashMap<Box<str>, u32>,
}

/// A symbol found by [`Library::search`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolRef {
    pub set: u16,
    pub index: u32,
}

pub struct Library {
    dir: PathBuf,
    sets: Vec<Set>,
    cache: Mutex<HashMap<SymbolRef, (Arc<[u8]>, Instant)>>,
}

impl Library {
    /// Read `sets.json` and every set's index from `dir`.
    pub fn open(dir: &Path) -> std::io::Result<Library> {
        let infos: Vec<SetInfo> = serde_json::from_slice(&std::fs::read(dir.join("sets.json"))?).map_err(std::io::Error::other)?;
        let mut sets = Vec::with_capacity(infos.len());
        for info in infos {
            let idx = match std::fs::read_to_string(dir.join(format!("{}.idx", info.key))) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut entries = Vec::new();
            for line in idx.lines() {
                let mut it = line.split('\t');
                let (Some(name), Some(kw), Some(off), Some(len)) = (it.next(), it.next(), it.next(), it.next()) else { continue };
                let (Ok(offset), Ok(len)) = (off.parse(), len.parse()) else { continue };
                entries.push(Entry { name: name.into(), keywords: kw.into(), offset, len });
            }
            let by_name = entries.iter().enumerate().map(|(i, e)| (e.name.clone(), i as u32)).collect();
            sets.push(Set { info, entries, by_name });
        }
        Ok(Library { dir: dir.to_path_buf(), sets, cache: Mutex::new(HashMap::new()) })
    }

    pub fn sets(&self) -> impl Iterator<Item = &SetInfo> {
        self.sets.iter().map(|s| &s.info)
    }

    pub fn set(&self, i: u16) -> Option<&SetInfo> {
        self.sets.get(i as usize).map(|s| &s.info)
    }

    pub fn set_index(&self, key: &str) -> Option<u16> {
        self.sets.iter().position(|s| s.info.key == key).map(|i| i as u16)
    }

    /// Symbols in all sets.
    pub fn total(&self) -> usize {
        self.sets.iter().map(|s| s.entries.len()).sum()
    }

    pub fn find(&self, set: &str, name: &str) -> Option<SymbolRef> {
        let s = self.set_index(set)?;
        let index = *self.sets[s as usize].by_name.get(name)?;
        Some(SymbolRef { set: s, index })
    }

    pub fn name(&self, r: SymbolRef) -> &str {
        &self.sets[r.set as usize].entries[r.index as usize].name
    }

    pub fn keywords(&self, r: SymbolRef) -> &str {
        &self.sets[r.set as usize].entries[r.index as usize].keywords
    }

    /// Every symbol of a set, in index order.
    pub fn all_in(&self, set: u16) -> impl Iterator<Item = SymbolRef> + '_ {
        let n = self.sets.get(set as usize).map_or(0, |s| s.entries.len());
        (0..n as u32).map(move |index| SymbolRef { set, index })
    }

    /// Symbols matching every word of `query` (name first, then keywords),
    /// best first; only in `sets` when given. An empty query lists them all.
    pub fn search(&self, query: &str, sets: Option<&[u16]>, limit: usize) -> Vec<SymbolRef> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
        let mut hits: Vec<(u16, SymbolRef)> = vec![];
        for (si, s) in self.sets.iter().enumerate() {
            if sets.is_some_and(|only| !only.contains(&(si as u16))) {
                continue;
            }
            'entry: for (i, e) in s.entries.iter().enumerate() {
                let mut rank = 0u16;
                for w in &words {
                    let name = &*e.name;
                    let r = if name == w {
                        0
                    } else if name.split([' ', '-']).any(|p| p == w) {
                        1
                    } else if name.starts_with(w.as_str()) || name.split([' ', '-']).any(|p| p.starts_with(w.as_str())) {
                        2
                    } else if e.keywords.split(',').any(|k| k == w) {
                        3
                    } else if e.keywords.split([',', ' ']).any(|k| k.starts_with(w.as_str())) {
                        4
                    } else {
                        continue 'entry;
                    };
                    rank = rank.max(r);
                }
                hits.push((rank, SymbolRef { set: si as u16, index: i as u32 }));
            }
        }
        // Stable: equal ranks keep set order, then index order.
        hits.sort_by_key(|(r, _)| *r);
        hits.into_iter().take(limit).map(|(_, r)| r).collect()
    }

    /// The SVG of a symbol, from memory when it was used lately.
    pub fn svg(&self, r: SymbolRef) -> std::io::Result<Arc<[u8]>> {
        let now = Instant::now();
        {
            let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            c.retain(|_, (_, used)| now.duration_since(*used) < IDLE);
            if let Some((b, used)) = c.get_mut(&r) {
                *used = now;
                return Ok(b.clone());
            }
        }
        let s = self.sets.get(r.set as usize).ok_or_else(|| std::io::Error::other("no such set"))?;
        let e = s.entries.get(r.index as usize).ok_or_else(|| std::io::Error::other("no such symbol"))?;
        let mut f = std::fs::File::open(self.dir.join(format!("{}.dat", s.info.key)))?;
        f.seek(SeekFrom::Start(e.offset))?;
        let mut buf = vec![0u8; e.len as usize];
        f.read_exact(&mut buf)?;
        let b: Arc<[u8]> = buf.into();
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(r, (b.clone(), now));
        Ok(b)
    }

    /// Drop SVGs not used for [`IDLE`]. Called on every read too.
    pub fn trim(&self) {
        let now = Instant::now();
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, (_, used)| now.duration_since(*used) < IDLE);
    }

    /// SVGs held in memory now.
    pub fn cached(&self) -> usize {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Where the library is: `$ZEBRA_SYMBOLS_DIR`, else `Symbols` in the app
/// bundle's Resources, else `assets/symbols` above the executable (a
/// development build).
pub fn find_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("ZEBRA_SYMBOLS_DIR") {
        return Some(PathBuf::from(d));
    }
    let exe = std::env::current_exe().ok()?;
    let mut d = exe.parent();
    if let Some(macos) = d {
        let res = macos.join("../Resources/Symbols");
        if res.join("sets.json").is_file() {
            return Some(res);
        }
    }
    while let Some(p) = d {
        let c = p.join("assets/symbols");
        if c.join("sets.json").is_file() {
            return Some(c);
        }
        d = p.parent();
    }
    None
}

/// The library, opened on first use (indexes take a moment: call this on
/// a background thread at startup). None when it isn't installed.
pub fn library() -> Option<&'static Library> {
    static LIB: OnceLock<Option<Library>> = OnceLock::new();
    LIB.get_or_init(|| Library::open(&find_dir()?).ok()).as_ref()
}

/// Width and height an SVG asks for: its viewBox, else its width and
/// height, else 100 x 100. Units are ignored.
pub fn svg_size(bytes: &[u8]) -> (f64, f64) {
    let s = String::from_utf8_lossy(&bytes[..bytes.len().min(16384)]);
    let Some(start) = s.find("<svg") else { return (100.0, 100.0) };
    let tag = &s[start..];
    let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
    let attr = |name: &str| -> Option<&str> {
        let mut from = 0;
        while let Some(i) = tag[from..].find(name) {
            let at = from + i;
            from = at + name.len();
            let before = tag[..at].chars().last();
            if !before.is_some_and(|c| c.is_whitespace()) {
                continue;
            }
            let rest = tag[from..].trim_start().strip_prefix('=')?.trim_start();
            let q = rest.chars().next()?;
            if q != '"' && q != '\'' {
                return None;
            }
            let v = &rest[1..];
            return Some(&v[..v.find(q)?]);
        }
        None
    };
    let num = |v: &str| -> Option<f64> {
        let end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E')).unwrap_or(v.len());
        if v[end..].trim_start().starts_with('%') {
            return None;
        }
        v[..end].parse().ok().filter(|x: &f64| x.is_finite() && *x > 0.0)
    };
    if let Some(vb) = attr("viewBox") {
        let n: Vec<f64> = vb.split([' ', ',', '\t', '\n']).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect();
        if n.len() == 4 && n[2] > 0.0 && n[3] > 0.0 && n[2].is_finite() && n[3].is_finite() {
            return (n[2], n[3]);
        }
    }
    match (attr("width").and_then(num), attr("height").and_then(num)) {
        (Some(w), Some(h)) => (w, h),
        _ => (100.0, 100.0),
    }
}

/// Pixel size recorded for an SVG asset: [`SVG_PX`] on the long side.
pub fn svg_pixels(bytes: &[u8]) -> (u32, u32) {
    let (w, h) = svg_size(bytes);
    let k = SVG_PX / w.max(h);
    (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1))
}

/// True when the bytes look like an SVG document.
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    head.contains("<svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_from_viewbox_or_width_and_height() {
        assert_eq!(svg_size(br#"<svg xmlns="x" viewBox="0 0 24 12">"#), (24.0, 12.0));
        assert_eq!(svg_size(br#"<?xml version="1.0"?><svg width="210mm" height="297mm">"#), (210.0, 297.0));
        assert_eq!(svg_size(br#"<svg width="100%" height="100%">"#), (100.0, 100.0));
        assert_eq!(svg_size(br#"<svg data-viewBox="0 0 1 1" viewBox='0,0,48,48'>"#), (48.0, 48.0));
        assert_eq!(svg_pixels(br#"<svg viewBox="0 0 24 12">"#), (4096, 2048));
    }
}
