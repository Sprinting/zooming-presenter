//! `.zebra` file format: a zip holding `document.json` and `assets/<key>.<ext>`.
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use crate::error::{ZError, ZResult};
use crate::model::{Asset, Document};

pub const EXTENSION: &str = "zebra";

pub fn to_json(doc: &Document) -> String {
    serde_json::to_string_pretty(doc).expect("document serialises")
}

pub fn save_bytes(doc: &Document) -> ZResult<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let io = |e: zip::result::ZipError| ZError::new("io", e.to_string());
        z.start_file("document.json", opts).map_err(io)?;
        z.write_all(to_json(doc).as_bytes()).map_err(|e| ZError::new("io", e.to_string()))?;
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (k, a) in &doc.assets {
            z.start_file(format!("assets/{k}.{}", a.ext), stored).map_err(io)?;
            z.write_all(&a.bytes).map_err(|e| ZError::new("io", e.to_string()))?;
        }
        z.finish().map_err(io)?;
    }
    Ok(buf.into_inner())
}

pub fn load_bytes(bytes: &[u8]) -> ZResult<Document> {
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
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| bad(e.to_string()))?;
        let (ext, w, h) = crate::editor::image_info(&data)?;
        doc.assets.insert(key.to_string(), Asset { ext, bytes: Arc::new(data), width: w, height: h });
    }
    Ok(doc)
}

pub fn from_json(json: &str) -> ZResult<Document> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| ZError::new("bad_file", e.to_string()))?;
    let ver = v.get("format_version").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    if ver > crate::FORMAT_VERSION {
        return Err(ZError::new("bad_file", format!("file format {ver} is newer than this app ({})", crate::FORMAT_VERSION)));
    }
    let v = migrate(v, ver);
    let mut doc: Document = serde_json::from_value(v).map_err(|e| ZError::new("bad_file", e.to_string()))?;
    doc.format_version = crate::FORMAT_VERSION;
    // repair z_order
    doc.z_order.retain(|id| doc.elements.contains_key(id));
    let missing: Vec<_> = doc.elements.keys().filter(|k| !doc.z_order.contains(k)).cloned().collect();
    doc.z_order.extend(missing);
    Ok(doc)
}

/// Format migrations. Each version bump adds a step here plus a fixture test.
fn migrate(v: serde_json::Value, _from: u32) -> serde_json::Value {
    v
}

pub fn save(doc: &Document, path: &Path) -> ZResult<()> {
    let bytes = save_bytes(doc)?;
    // atomic write
    let tmp = path.with_extension("zebra.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| ZError::new("io", format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| ZError::new("io", format!("{}: {e}", path.display())))?;
    Ok(())
}

pub fn load(path: &Path) -> ZResult<Document> {
    let bytes = std::fs::read(path).map_err(|e| ZError::new("io", format!("{}: {e}", path.display())))?;
    if path.extension().is_some_and(|e| e == "json") {
        return from_json(std::str::from_utf8(&bytes).map_err(|e| ZError::new("bad_file", e.to_string()))?);
    }
    load_bytes(&bytes)
}
