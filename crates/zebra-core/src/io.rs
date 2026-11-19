//! `.zebra` file format: a zip holding `document.json` and `assets/<key>.<ext>`.
#![cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use crate::error::{ZError, ZResult};
use crate::model::{Asset, AssetBytes, Document};

pub const EXTENSION: &str = "zebra";

pub fn to_json(doc: &Document) -> String {
    serde_json::to_string_pretty(doc).expect("document serialises")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_bytes(doc: &Document) -> ZResult<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    write_zip(doc, &mut buf)?;
    Ok(buf.into_inner())
}

#[cfg(not(target_arch = "wasm32"))]
/// Assets the document's elements use. History may hold others; they
/// stay in memory/mapped but aren't written to the file.
fn used_assets(doc: &Document) -> std::collections::BTreeSet<&str> {
    doc.used_assets()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_zip<W: Write + std::io::Seek>(doc: &Document, w: W) -> ZResult<()> {
    let io = |e: zip::result::ZipError| ZError::new("io", e.to_string());
    let ioe = |e: std::io::Error| ZError::new("io", e.to_string());
    let mut z = zip::ZipWriter::new(w);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("document.json", opts).map_err(io)?;
    z.write_all(to_json(doc).as_bytes()).map_err(ioe)?;
    // Media is already compressed; storing it uncompressed also lets
    // `load` memory-map it straight out of the file. Large assets use
    // zip64 so video-sized files work.
    let used = used_assets(doc);
    for (k, a) in doc.assets.iter().filter(|(k, _)| used.contains(k.as_str())) {
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(a.bytes.len() as u64 >= u32::MAX as u64);
        z.start_file(format!("assets/{k}.{}", a.ext), stored).map_err(io)?;
        z.write_all(&a.bytes).map_err(ioe)?;
    }
    z.finish().map_err(io)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_bytes(bytes: &[u8]) -> ZResult<Document> {
    read_zip(bytes, None)
}

#[cfg(not(target_arch = "wasm32"))]
/// Parse a `.zebra` zip. With `map`, uncompressed assets become slices of
/// it instead of copies.
fn read_zip(bytes: &[u8], map: Option<&Arc<memmap2::Mmap>>) -> ZResult<Document> {
    let bad = |m: String| ZError::new("bad_file", m);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| bad(format!("not a .zebra file: {e}")))?;
    let mut json = String::new();
    z.by_name("document.json").map_err(|e| bad(e.to_string()))?.read_to_string(&mut json).map_err(|e| bad(e.to_string()))?;
    let mut doc = from_json(&json)?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| bad(e.to_string()))?;
        let name = f.name().map(|n| n.to_string()).unwrap_or_default();
        let Some(file) = name.strip_prefix("assets/") else { continue };
        let Some((key, _ext)) = file.rsplit_once('.') else { continue };
        // A key becomes a file name in the media cache: one that isn't a
        // plain name (a crafted "assets/../../x.mp4") is not an asset.
        if !crate::check::safe_asset_key(key) {
            continue;
        }
        let mapped = match (map, f.compression(), f.data_start()) {
            (Some(m), zip::CompressionMethod::Stored, Some(start)) if !f.encrypted() => {
                let (start, len) = (start as usize, f.size() as usize);
                (start.checked_add(len).is_some_and(|end| end <= m.len())).then(|| AssetBytes::Mapped { map: m.clone(), start, len })
            }
            _ => None,
        };
        let data = match mapped {
            Some(m) => m,
            None => {
                let mut data = Vec::new();
                f.read_to_end(&mut data).map_err(|e| bad(e.to_string()))?;
                data.into()
            }
        };
        // Only the header is read here; pixels are decoded lazily by the
        // renderer, video and audio by the platform player.
        let (ext, w, h) = match _ext.to_ascii_lowercase().as_str() {
            // Embedded theme fonts.
            e @ ("ttf" | "otf" | "ttc") => (e.to_string(), 0, 0),
            // Symbols from the clip-art library.
            "svg" => {
                let (w, h) = crate::symbols::svg_pixels(&data);
                ("svg".to_string(), w, h)
            }
            // One file Zebra can't read doesn't stop the document
            // opening; what showed it draws as a missing picture.
            _ => match crate::media::info(&data) {
                Ok(m) => (m.ext, m.width, m.height),
                Err(_) => continue,
            },
        };
        doc.assets.insert(key.to_string(), Asset { ext, bytes: data, width: w, height: h });
    }
    Ok(doc)
}

pub fn from_json(json: &str) -> ZResult<Document> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| ZError::new("bad_file", e.to_string()))?;
    let ver = v.get("format_version").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    if ver > crate::FORMAT_VERSION {
        return Err(ZError::new("bad_file", format!("file format {ver} is newer than this app ({})", crate::FORMAT_VERSION)));
    }
    let mut v = migrate(v, ver);
    crate::check::repair_json(&mut v);
    let mut doc: Document = serde_json::from_value(v).map_err(|e| ZError::new("bad_file", e.to_string()))?;
    doc.format_version = crate::FORMAT_VERSION;
    // Numbers, text ranges and the draw order of a file from an older
    // build or another program are brought back in range here, once.
    crate::check::repair(&mut doc);
    Ok(doc)
}

/// Format migrations. Each version bump adds a step here plus a fixture test.
fn migrate(v: serde_json::Value, _from: u32) -> serde_json::Value {
    v
}

#[cfg(not(target_arch = "wasm32"))]
/// Save atomically (temp file + rename), streaming assets from wherever
/// they live instead of building the whole zip in memory. Afterwards the
/// document's assets point into the new file, so freshly added media
/// leaves RAM after the first save.
pub fn save(doc: &mut Document, path: &Path) -> ZResult<()> {
    write_file(doc, path)?;
    // Re-point assets at the new file. Failure here is harmless: the old
    // bytes stay valid.
    if let Ok(saved) = load(path) {
        for (k, a) in saved.assets {
            if a.bytes.is_mapped() {
                if let Some(mine) = doc.assets.get_mut(&k) {
                    mine.bytes = a.bytes;
                }
            }
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
/// Write a `.zebra` atomically without touching `doc` (autosave, copies).
pub fn write_file(doc: &Document, path: &Path) -> ZResult<()> {
    let tmp = path.with_extension("zebra.tmp");
    let ioe = |p: &Path, e: std::io::Error| ZError::new("io", format!("{}: {e}", p.display()));
    {
        let f = std::fs::File::create(&tmp).map_err(|e| ioe(&tmp, e))?;
        let mut w = std::io::BufWriter::new(f);
        write_zip(doc, &mut w)?;
        let f = w.into_inner().map_err(|e| ioe(&tmp, e.into_error()))?;
        f.sync_all().map_err(|e| ioe(&tmp, e))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| ioe(path, e))?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load(path: &Path) -> ZResult<Document> {
    let ioe = |e: std::io::Error| ZError::new("io", format!("{}: {e}", path.display()));
    if path.extension().is_some_and(|e| e == "json") {
        let bytes = std::fs::read(path).map_err(ioe)?;
        return from_json(std::str::from_utf8(&bytes).map_err(|e| ZError::new("bad_file", e.to_string()))?);
    }
    let file = std::fs::File::open(path).map_err(ioe)?;
    // SAFETY: the mapping is read-only. Zebra itself never modifies a saved
    // file in place (it writes a new file and renames it over), so the
    // mapped bytes stay valid; see `AssetBytes`.
    let map = Arc::new(unsafe { memmap2::Mmap::map(&file) }.map_err(ioe)?);
    read_zip(&map, Some(&map))
}
