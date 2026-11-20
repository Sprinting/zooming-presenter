//! `.zebra` file format: a zip holding `document.json` and `assets/<key>.<ext>`.
#![cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use crate::error::{ZError, ZResult};
use crate::model::{Asset, AssetBytes, Document};

pub const EXTENSION: &str = "zebra";

pub fn to_json(doc: &Document) -> String {
    let json = serde_json::to_string_pretty(doc).unwrap_or_else(|e| unreachable!("document types serialise: {e}"));
    // Saved as the lowest version that holds what's in it, so a file that
    // doesn't use anything new still opens in older builds.
    // `format_version` is the document's first field.
    let need = required_version(doc);
    let head = format!("{{\n  \"format_version\": {},", doc.format_version);
    match json.strip_prefix(&head) {
        Some(rest) if need != doc.format_version => format!("{{\n  \"format_version\": {need},{rest}"),
        _ => json,
    }
}

/// The lowest format version that holds everything in `doc`. Each check
/// names what needs it; a build that knows less refuses the file rather
/// than dropping what it doesn't know.
pub fn required_version(doc: &Document) -> u32 {
    use crate::model::ElementKind;
    let new_slot = |c: &Option<String>| c.as_deref().is_some_and(|c| c == "theme:shape5" || c == "theme:shape6");
    let mut v = 1;
    // 2: tables.
    if doc.elements.values().any(|e| matches!(e.kind, ElementKind::Table(_))) {
        v = v.max(2);
    }
    // 2: theme colours shape5 and shape6, in a theme of the document's or
    // used by an element.
    if doc.themes.iter().any(|t| t.shapes.len() > 4 && t.shapes[4..].iter().any(|c| !c.is_empty())) {
        v = v.max(2);
    }
    let uses_new_slot = |e: &crate::model::Element| match &e.kind {
        ElementKind::Text(t) => new_slot(&t.color) || t.spans.iter().any(|s| new_slot(&s.format.color)),
        ElementKind::Shape(s) => new_slot(&s.fill) || new_slot(&s.stroke),
        ElementKind::Frame(f) => new_slot(&f.color),
        ElementKind::Ink(i) => new_slot(&Some(i.color.clone())),
        ElementKind::Icon(i) => new_slot(&Some(i.color.clone())),
        ElementKind::Connector(c) => new_slot(&Some(c.color.clone())),
        ElementKind::Image(i) => i.symbol.as_ref().is_some_and(|s| new_slot(&s.color) || new_slot(&s.color2)),
        ElementKind::Video(_) | ElementKind::Table(_) => false,
    };
    if doc.elements.values().any(uses_new_slot) {
        v = v.max(2);
    }
    v.min(crate::FORMAT_VERSION)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_bytes(doc: &Document) -> ZResult<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    write_zip(doc, &mut buf, &|_, _| true)?;
    Ok(buf.into_inner())
}

#[cfg(not(target_arch = "wasm32"))]
/// Assets the document's elements use. History may hold others; they
/// stay in memory/mapped but aren't written to the file.
fn used_assets(doc: &Document) -> std::collections::BTreeSet<&str> {
    doc.used_assets()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_zip<W: Write + std::io::Seek>(doc: &Document, w: W, keep: &dyn Fn(&str, &Asset) -> bool) -> ZResult<()> {
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
    for (k, a) in doc.assets.iter().filter(|(k, a)| used.contains(k.as_str()) && keep(k, a)) {
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
fn read_zip(bytes: &[u8], map: Option<&Arc<crate::media::MappedFile>>) -> ZResult<Document> {
    let bad = |m: String| ZError::new("bad_file", m);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| bad(format!("not a .zebra file: {e}")))?;
    let f = z.by_name("document.json").map_err(|e| bad(e.to_string()))?;
    if f.size() > MAX_DOCUMENT_JSON {
        return Err(too_long());
    }
    let json = read_json(f)?;
    let mut doc = from_json(&json)?;
    doc.assets = read_assets(&mut z, map)?;
    Ok(doc)
}

/// Largest `document.json` Zebra opens. Media lives in its own files, so
/// even a deck of thousands of frames is a few MB; past this it is a
/// broken or hostile file (1 GB of spaces deflates to 1 MB).
pub const MAX_DOCUMENT_JSON: u64 = 32 << 20;

#[cfg(not(target_arch = "wasm32"))]
fn too_long() -> ZError {
    ZError::new("bad_file", "the document is too large to be a Zebra document (over 32 MB of text)")
}

/// Largest inflation a compressed asset may claim. Zebra stores assets
/// uncompressed; a picture or video another tool deflated shrinks by a
/// few percent, a font or SVG by 3-10x. A zip bomb is 1000x.
#[cfg(not(target_arch = "wasm32"))]
const MAX_INFLATE: u64 = 100;

/// Read `document.json` (or a `.json` document) up to
/// [`MAX_DOCUMENT_JSON`], whatever the zip's directory claims.
#[cfg(not(target_arch = "wasm32"))]
fn read_json(r: impl Read) -> ZResult<String> {
    let mut json = String::new();
    r.take(MAX_DOCUMENT_JSON + 1).read_to_string(&mut json).map_err(|e| ZError::new("bad_file", e.to_string()))?;
    if json.len() as u64 > MAX_DOCUMENT_JSON {
        return Err(too_long());
    }
    Ok(json)
}

#[cfg(not(target_arch = "wasm32"))]
/// The assets in a `.zebra` zip (mapped when `map` is the file and they
/// are stored uncompressed). Assets are only read as far as their headers.
fn read_assets(z: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, map: Option<&Arc<crate::media::MappedFile>>) -> ZResult<std::collections::BTreeMap<String, Asset>> {
    let bad = |m: String| ZError::new("bad_file", m);
    let mut assets = std::collections::BTreeMap::new();
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
                // Compressed (or no mapping): read at most what the entry
                // could honestly hold, so a 2 MB zip can't ask for 2 GB.
                // The type is checked from the first bytes before the
                // rest is read. Big ones go to a spill file, not the heap.
                let limit = f.size().min(f.compressed_size().saturating_mul(MAX_INFLATE).max(1 << 20));
                let mut head = Vec::new();
                (&mut f).take(limit.min(4096)).read_to_end(&mut head).map_err(|e| bad(e.to_string()))?;
                if !plausible_asset(_ext, &head) {
                    continue;
                }
                let rest = limit - head.len() as u64;
                let mut r = std::io::Cursor::new(head).chain((&mut f).take(rest));
                if limit >= 1 << 20 {
                    match crate::media::spill_reader(&mut r, &safe_ext(_ext)) {
                        Ok((_, b)) => b,
                        Err(_) => continue,
                    }
                } else {
                    let mut data = Vec::new();
                    r.read_to_end(&mut data).map_err(|e| bad(e.to_string()))?;
                    data.into()
                }
            }
        };
        let Some(asset) = sniff_asset(_ext, data) else { continue };
        assets.insert(key.to_string(), asset);
    }
    Ok(assets)
}

/// Could an asset that starts with `head` be one Zebra reads? (The full
/// check is [`sniff_asset`], once it has been read.)
#[cfg(not(target_arch = "wasm32"))]
fn plausible_asset(ext: &str, head: &[u8]) -> bool {
    match ext.to_ascii_lowercase().as_str() {
        "ttf" | "otf" | "ttc" | "svg" => true,
        _ => crate::media::sniff_kind(head).is_some(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn safe_ext(ext: &str) -> String {
    let e = ext.to_ascii_lowercase();
    if !e.is_empty() && e.len() <= 8 && e.bytes().all(|b| b.is_ascii_alphanumeric()) { e } else { "bin".into() }
}

/// An asset from its stored bytes and file extension. Only the header is
/// read; pixels are decoded lazily by the renderer, video and audio by the
/// platform player. None for a file Zebra can't read: one such file
/// doesn't stop the document opening, what showed it draws as a missing
/// picture.
fn sniff_asset(ext: &str, data: AssetBytes) -> Option<Asset> {
    let (ext, w, h) = match ext.to_ascii_lowercase().as_str() {
        // Embedded theme fonts.
        e @ ("ttf" | "otf" | "ttc") => (e.to_string(), 0, 0),
        // An imported PDF (pages are drawn from it).
        "pdf" if crate::import::is_pdf(&data) => ("pdf".to_string(), 0, 0),
        // Symbols from the clip-art library.
        "svg" => {
            let (w, h) = crate::symbols::svg_pixels(&data);
            ("svg".to_string(), w, h)
        }
        _ => match crate::media::info(&data) {
            Ok(m) => (m.ext, m.width, m.height),
            Err(_) => return None,
        },
    };
    Some(Asset { ext, bytes: data, width: w, height: h })
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
    let saved = write_and_map(doc, path)?;
    adopt_mapped(doc, saved);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
/// Write `doc` to `path` (as [`write_file`]) and map the assets back out
/// of the new file: the part of [`save`] that can run on a worker thread
/// with a copy of the document. Hand the result to [`adopt_mapped`].
pub fn write_and_map(doc: &Document, path: &Path) -> ZResult<Vec<(String, AssetBytes)>> {
    write_file(doc, path)?;
    // Failure here is harmless: the old bytes stay valid.
    Ok(mapped_assets(path).unwrap_or_default())
}

/// The web player has no file system: saving is never offered there.
#[cfg(target_arch = "wasm32")]
pub fn write_and_map(_doc: &Document, _path: &Path) -> ZResult<Vec<(String, AssetBytes)>> {
    Err(ZError::new("unavailable", "saving needs a file system"))
}

/// Assets `keys` (kept only for undo after a save) that are slices of a
/// `.zebra` (the file before this save, a document they were pasted
/// from), each copied out to a spill file of its own. Left mapped, a
/// deleted video kept the whole old file on disk, every other picture and
/// video in it too, until the document closed. Hand the result to
/// [`adopt_mapped`].
/// The web player has no file system: nothing is ever mapped there.
#[cfg(target_arch = "wasm32")]
pub fn rehome(_doc: &Document, _keys: &[String]) -> Vec<(String, AssetBytes)> {
    vec![]
}

#[cfg(not(target_arch = "wasm32"))]
pub fn rehome(doc: &Document, keys: &[String]) -> Vec<(String, AssetBytes)> {
    let mut out = vec![];
    for k in keys {
        let Some(a) = doc.assets.get(k) else { continue };
        let Some(m) = a.bytes.mapped_file() else { continue };
        if m.is_owned() || a.bytes.own_file().is_some() {
            continue;
        }
        if let Ok((key, bytes)) = crate::media::spill_reader(&mut std::io::Cursor::new(&a.bytes[..]), &a.ext)
            && &key == k
        {
            out.push((key, bytes));
        }
    }
    out
}

/// Point `doc`'s assets at the saved file (from [`write_and_map`]), so
/// freshly added media leaves memory. Assets are content addressed: the
/// same key is the same bytes, wherever they live.
pub fn adopt_mapped(doc: &mut Document, saved: Vec<(String, AssetBytes)>) {
    for (k, bytes) in saved {
        if let Some(mine) = doc.assets.get_mut(&k) {
            mine.bytes = bytes;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The assets of a saved `.zebra` that are mapped straight from the file.
/// Reads the zip's directory only, not the document.
fn mapped_assets(path: &Path) -> ZResult<Vec<(String, AssetBytes)>> {
    let ioe = |e: std::io::Error| ZError::new("io", format!("{}: {e}", path.display()));
    let map = Arc::new(map_document(path).map_err(ioe)?);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&map[..])).map_err(|e| ZError::new("bad_file", e.to_string()))?;
    Ok(read_assets(&mut z, Some(&map))?.into_iter().filter(|(_, a)| a.bytes.is_mapped()).map(|(k, a)| (k, a.bytes)).collect())
}

#[cfg(not(target_arch = "wasm32"))]
/// Size of the document (its `document.json`, uncompressed) in a
/// `.zebra` file, from the zip's directory: how long opening it will take.
pub fn document_size(path: &Path) -> Option<u64> {
    let file = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(file)).ok()?;
    let f = z.by_name("document.json").ok()?;
    Some(f.size())
}

#[cfg(not(target_arch = "wasm32"))]
/// Write a `.zebra` atomically without touching `doc` (autosave, copies).
pub fn write_file(doc: &Document, path: &Path) -> ZResult<()> {
    write_file_with(doc, path, |_, _| true)
}

/// The temp file a save writes before renaming it over `path`: hidden,
/// and named by the process, so [`sweep_temp`] can tell a leftover of a
/// run that crashed mid-save from one being written now.
#[cfg(not(target_arch = "wasm32"))]
fn temp_path(path: &Path) -> std::path::PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    path.with_file_name(format!(".{name}.saving-{}", std::process::id()))
}

/// Remove what a crashed save or open left in `dir`: temp files of saves
/// and autosaves (`.<name>.saving-<pid>`), clones of opened files
/// (`.<name>.open-<pid>`), and autosave media being copied
/// (`.<name>.part-<pid>`), of processes that aren't running. `alive`
/// says whether a pid is.
#[cfg(not(target_arch = "wasm32"))]
pub fn sweep_temp(dir: &Path, alive: impl Fn(u32) -> bool) {
    let mut all = vec![dir.to_path_buf()];
    // Autosave media folders hold `.part-` files too.
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if n.ends_with(".autosave.media") && e.file_type().is_ok_and(|t| t.is_dir()) {
            all.push(e.path());
        }
    }
    for d in all {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !n.starts_with('.') {
                // Before temp files were named by process: `x.zebra.tmp`.
                // Only beside the document it was a save of.
                if let Some(doc) = n.strip_suffix(".tmp").filter(|d| d.ends_with(".zebra"))
                    && d.join(doc).is_file()
                {
                    let _ = std::fs::remove_file(e.path());
                }
                continue;
            }
            // Only Zebra's own names: `.part-` files only inside an
            // autosave media folder, never loose in the person's folder.
            let marks: &[&str] = if d == dir { &[".saving-", ".open-"] } else { &[".part-"] };
            let pid = marks.iter().find_map(|m| n.rsplit_once(m).map(|(_, p)| p)).and_then(|p| p.parse::<u32>().ok());
            if pid.is_some_and(|p| !alive(p)) && e.file_type().is_ok_and(|t| t.is_file()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// [`write_file`], writing only the assets `keep` (key, asset) accepts.
fn write_file_with(doc: &Document, path: &Path, keep: impl Fn(&str, &Asset) -> bool) -> ZResult<()> {
    let tmp = temp_path(path);
    let ioe = |p: &Path, e: std::io::Error| ZError::new("io", format!("{}: {e}", p.display()));
    {
        let f = std::fs::File::create(&tmp).map_err(|e| ioe(&tmp, e))?;
        let mut w = std::io::BufWriter::new(f);
        if let Err(e) = write_zip(doc, &mut w, &keep) {
            drop(w);
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        let f = w.into_inner().map_err(|e| ioe(&tmp, e.into_error()))?;
        f.sync_all().map_err(|e| ioe(&tmp, e))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ioe(path, e)
    })?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load(path: &Path) -> ZResult<Document> {
    let ioe = |e: std::io::Error| ZError::new("io", format!("{}: {e}", path.display()));
    if path.extension().is_some_and(|e| e == "json") {
        crate::media::regular_file(path).map_err(ioe)?;
        return from_json(&read_json(std::fs::File::open(path).map_err(ioe)?)?);
    }
    let map = Arc::new(map_document(path).map_err(ioe)?);
    read_zip(&map, Some(&map))
}

/// Map a `.zebra` for reading its assets out of it. Zebra never writes a
/// file it has mapped (it writes a new one and renames it over), but
/// another program might rewrite the file in place (a `cp` over it, a
/// sync tool), and a mapping of a file that changes under it crashes the
/// app when read. On APFS the file is cloned first (no copy is made, it
/// costs no space until one of them changes) and the clone is mapped, so
/// whatever happens to the file afterwards, the open document keeps the
/// bytes it opened with. The clone's name goes at once. Elsewhere (a USB
/// stick or network share that can't clone) the file is copied to
/// Zebra's temporary folder instead and that copy mapped, for the same
/// reason: those are exactly the places other programs rewrite files.
#[cfg(not(target_arch = "wasm32"))]
pub fn map_document(path: &Path) -> std::io::Result<crate::media::MappedFile> {
    crate::media::regular_file(path)?;
    #[cfg(target_os = "macos")]
    if let Some(m) = clone_and_map(path) {
        return Ok(m);
    }
    crate::media::private_copy(path)
}

#[cfg(target_os = "macos")]
fn clone_and_map(path: &Path) -> Option<crate::media::MappedFile> {
    use std::os::unix::ffi::OsStrExt;
    let name = path.file_name()?.to_string_lossy().to_string();
    let clone = path.with_file_name(format!(".{name}.open-{}", std::process::id()));
    let _ = std::fs::remove_file(&clone);
    let src = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let dst = std::ffi::CString::new(clone.as_os_str().as_bytes()).ok()?;
    // SAFETY: two valid NUL-terminated paths; clonefile(2) only reads them.
    if unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), 0) } != 0 {
        return None;
    }
    let m = crate::media::MappedFile::open(&clone);
    let _ = std::fs::remove_file(&clone);
    // The mapping keeps the clone's data; its name names the original, so
    // a save's "is this asset already in the file" check still sees it.
    m.ok().map(|m| m.renamed(path))
}

// ----------------------------------------------------------------------
// Autosave

/// Where an autosave keeps its big media: `<autosave>.media/`, one file
/// per asset (`<key>.<ext>`).
#[cfg(not(target_arch = "wasm32"))]
pub fn autosave_media_dir(autosave: &Path) -> std::path::PathBuf {
    let name = autosave.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    autosave.with_file_name(format!("{name}.media"))
}

/// The stored sizes of a `.zebra`'s assets, by file name (`<key>.<ext>`),
/// from its zip directory.
#[cfg(not(target_arch = "wasm32"))]
fn stored_assets(path: &Path) -> std::collections::HashMap<String, u64> {
    let mut out = std::collections::HashMap::new();
    let Ok(f) = std::fs::File::open(path) else { return out };
    let Ok(mut z) = zip::ZipArchive::new(std::io::BufReader::new(f)) else { return out };
    for i in 0..z.len() {
        let Ok(e) = z.by_index_raw(i) else { continue };
        let Ok(name) = e.name() else { continue };
        if let Some(name) = name.strip_prefix("assets/") {
            out.insert(name.to_string(), e.size());
        }
    }
    out
}

/// Write an autosave of `doc` to `path` without writing its media again
/// every time. An autosave used to be a whole `.zebra`: every 30 seconds
/// of editing a deck with a 3 GB video wrote 3 GB, and needed room for the
/// deck, the old autosave and the new one at once. Now:
///
/// * media the saved document `base` already holds (same key, same size)
///   is not written at all: recovery reads it from there;
/// * other big media ([`crate::media::SPILL_MIN`] or more) goes once into
///   `<autosave>.media/` as `<key>.<ext>`: a hard link to its spill file
///   when it has one (no copy, no time), else a copy made once;
/// * the autosave file itself holds the document and small media.
///
/// Files in the media folder the document no longer uses are removed.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_autosave(doc: &Document, path: &Path, base: Option<&Path>) -> ZResult<()> {
    let media = autosave_media_dir(path);
    let in_base = base.map(stored_assets).unwrap_or_default();
    let used = doc.used_assets();
    let mut outside: std::collections::BTreeSet<String> = Default::default();
    let mut linked: std::collections::BTreeSet<String> = Default::default();
    for k in &used {
        let Some(a) = doc.assets.get(*k) else { continue };
        if a.bytes.len() < crate::media::SPILL_MIN || !crate::check::safe_asset_key(k) {
            continue;
        }
        let name = format!("{k}.{}", a.ext);
        if in_base.get(&name) == Some(&(a.bytes.len() as u64)) {
            outside.insert(k.to_string());
            continue;
        }
        if place_media(&media, &name, &a.bytes).is_ok() {
            outside.insert(k.to_string());
            linked.insert(name);
        }
    }
    write_file_with(doc, path, |k, _| !outside.contains(k))?;
    // What the document no longer uses goes.
    for e in std::fs::read_dir(&media).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if !linked.contains(&n) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    if linked.is_empty() {
        let _ = std::fs::remove_dir(&media);
    }
    Ok(())
}

/// Make sure `dir/name` holds `bytes`: already there, a hard link to the
/// asset's own file, or (another volume, no own file) a streamed copy.
#[cfg(not(target_arch = "wasm32"))]
fn place_media(dir: &Path, name: &str, bytes: &AssetBytes) -> std::io::Result<()> {
    let to = dir.join(name);
    if std::fs::metadata(&to).is_ok_and(|m| m.len() == bytes.len() as u64) {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::remove_file(&to);
    if let Some(own) = bytes.own_file() {
        if std::fs::hard_link(own, &to).is_ok() && std::fs::metadata(&to).is_ok_and(|m| m.len() == bytes.len() as u64) {
            return Ok(());
        }
        let _ = std::fs::remove_file(&to);
    }
    let tmp = dir.join(format!(".{name}.part-{}", std::process::id()));
    let r = std::fs::write(&tmp, &bytes[..]).and_then(|()| std::fs::rename(&tmp, &to));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// Read an autosave written by [`write_autosave`] (or a whole `.zebra`,
/// as autosaves were before): its own assets, then those in its media
/// folder, then those of the saved document `base` it was made from.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_autosave(path: &Path, base: Option<&Path>) -> ZResult<Document> {
    let mut doc = load(path)?;
    let missing = |doc: &Document| -> std::collections::BTreeSet<String> {
        doc.used_assets().into_iter().filter(|k| !doc.assets.contains_key(*k)).map(str::to_string).collect()
    };
    let want = missing(&doc);
    if want.is_empty() {
        return Ok(doc);
    }
    for e in std::fs::read_dir(autosave_media_dir(path)).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let Some((key, ext)) = n.rsplit_once('.') else { continue };
        if !want.contains(key) {
            continue;
        }
        // Mapped, not owned: the autosave's media stays until the autosave
        // goes (saved, or changes discarded).
        let Ok(m) = crate::media::MappedFile::open(&e.path()) else { continue };
        if let Some(a) = sniff_asset(ext, m.into()) {
            doc.assets.insert(key.to_string(), a);
        }
    }
    let want = missing(&doc);
    if let (false, Some(b)) = (want.is_empty(), base) {
        let from = (|| -> Option<std::collections::BTreeMap<String, Asset>> {
            let map = Arc::new(map_document(b).ok()?);
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(&map[..])).ok()?;
            read_assets(&mut z, Some(&map)).ok()
        })();
        for (k, a) in from.into_iter().flatten().filter(|(k, _)| want.contains(k)) {
            doc.assets.insert(k, a);
        }
    }
    Ok(doc)
}

/// Remove an autosave and its media folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn remove_autosave(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir_all(autosave_media_dir(path));
}

/// Move an autosave and its media folder (mappings of its media stay
/// valid: they follow the files).
#[cfg(not(target_arch = "wasm32"))]
pub fn rename_autosave(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)?;
    let (m0, m1) = (autosave_media_dir(from), autosave_media_dir(to));
    if m0.is_dir() {
        let _ = std::fs::remove_dir_all(&m1);
        std::fs::rename(&m0, &m1)?;
    }
    Ok(())
}
