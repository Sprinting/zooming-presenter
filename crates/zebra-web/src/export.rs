//! File > Export > Web Page: one self-contained `.html` holding the player
//! (wasm), the document, the pictures and fonts it uses, and its video and
//! audio. A page that would pass [`INLINE_LIMIT`] becomes a folder instead:
//! `index.html` plus the video and audio files beside it in `media/`.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::Serialize;
use serde_json::json;
use zebra_core::model::MediaKind;
use zebra_core::{Editor, ElementKind, ZError};
use zebra_render::Renderer;

/// Above this many bytes of page, video and audio move out into files.
pub const INLINE_LIMIT: usize = 50 * 1024 * 1024;

/// The player, gzip-compressed, built from this crate for wasm32 by
/// `build.rs`. Empty when the build had no wasm32 target.
pub const PLAYER_WASM_GZ: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/player.wasm.gz"));

const PLAYER_JS: &str = include_str!("../web/player.js");
const PLAYER_CSS: &str = include_str!("../web/player.css");

/// Words the page shows, in the person's language. `{step}` and `{count}`
/// are filled in by the page.
#[derive(Debug, Clone, Serialize)]
pub struct Labels {
    pub previous: String,
    pub next: String,
    pub full_screen: String,
    pub step_of: String,
    pub overview: String,
    pub play: String,
    pub pause: String,
    /// Shown if the browser can't run the player.
    pub unsupported: String,
}

impl Default for Labels {
    fn default() -> Self {
        Labels {
            previous: "Previous Step".into(),
            next: "Next Step".into(),
            full_screen: "Full Screen".into(),
            step_of: "{step} of {count}".into(),
            overview: "Overview · {count} steps".into(),
            play: "Play".into(),
            pause: "Pause".into(),
            unsupported: "This browser can't show this presentation. Try a current Safari, Chrome or Firefox.".into(),
        }
    }
}

/// What an export wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    /// The page to open.
    pub page: PathBuf,
    /// True when it is a folder (`index.html` + `media/`).
    pub folder: bool,
    /// Bytes written in all.
    pub bytes: u64,
}

/// A built page, before it is written.
pub struct Page {
    pub html: String,
    /// Video and audio kept out of the page (folder export): path relative
    /// to the page, bytes.
    pub media: Vec<(String, Vec<u8>)>,
}

impl Page {
    pub fn is_folder(&self) -> bool {
        !self.media.is_empty()
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    let _ = e.write_all(bytes);
    e.finish().unwrap_or_default()
}

pub fn mime(ext: &str) -> &'static str {
    match ext {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// JSON safe inside `<script>`: no `</script>` or `<!--` can end it.
fn script_json(v: &serde_json::Value) -> String {
    v.to_string().replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026")
}

/// Build the page for `editor`'s document. `inline_limit` decides when
/// video and audio go into files beside the page instead.
pub fn build(renderer: &Renderer, editor: &Editor, labels: &Labels, inline_limit: usize) -> Result<Page, ZError> {
    if PLAYER_WASM_GZ.is_empty() {
        return Err(ZError::new("unavailable", "this build of Zebra has no web player (it was built without the wasm32 target)")
            .hint("rustup target add wasm32-unknown-unknown, then rebuild"));
    }
    // PDF pages: a sharp picture each, drawn now from the PDF, in a copy.
    let sharp = zebra_render::pages::export_pictures(renderer, &editor.doc);
    let doc = sharp.as_ref().unwrap_or(&editor.doc);
    // Blobs: id -> base64, each in its own <script> element.
    let mut blobs: Vec<(String, String)> = vec![];
    fn add(blobs: &mut Vec<(String, String)>, data: String) -> String {
        let id = format!("zb{}", blobs.len());
        blobs.push((id.clone(), data));
        id
    }
    macro_rules! add_blob {
        ($d:expr) => {
            add(&mut blobs, $d)
        };
    }
    let wasm = add_blob!(b64(PLAYER_WASM_GZ));
    let json = zebra_core::io::to_json(doc);
    let doc_id = add_blob!(b64(&gzip(json.as_bytes())));
    let fonts: Vec<String> = renderer.font_files_used(editor).iter().map(|f| add_blob!(b64(&gzip(f)))).collect();

    // Video and audio: what is used, in the page or (too big) as files.
    // An imported PDF stays out: the player draws its pages' pictures.
    let used: std::collections::BTreeSet<&str> =
        doc.used_assets().into_iter().filter(|k| doc.assets.get(*k).is_none_or(|a| a.media_kind() != MediaKind::Document)).collect();
    let font_keys: std::collections::HashSet<&str> = doc.themes.iter().flat_map(|t| t.fonts.values().flatten()).map(|s| s.as_str()).collect();
    let media_bytes: usize = doc
        .assets
        .iter()
        .filter(|(k, a)| used.contains(k.as_str()) && a.media_kind() != MediaKind::Image)
        .map(|(_, a)| a.bytes.len())
        .sum();
    let other_bytes: usize = blobs.iter().map(|(_, b)| b.len()).sum::<usize>()
        + doc.assets.iter().filter(|(k, a)| used.contains(k.as_str()) && a.media_kind() == MediaKind::Image).map(|(_, a)| a.bytes.len() * 4 / 3).sum::<usize>();
    let folder = media_bytes > 0 && other_bytes + media_bytes * 4 / 3 > inline_limit;

    let mut assets: Vec<serde_json::Value> = vec![];
    let mut media: Vec<(String, Vec<u8>)> = vec![];
    for (key, a) in doc.assets.iter().filter(|(k, _)| used.contains(k.as_str()) && !font_keys.contains(k.as_str())) {
        let kind = a.media_kind();
        let mut entry = json!({"key": key, "ext": a.ext, "w": a.width, "h": a.height, "mime": mime(&a.ext),
            "kind": match kind { MediaKind::Image => "image", MediaKind::Video => "video", MediaKind::Audio => "audio", MediaKind::Document => "pdf" }});
        if kind != MediaKind::Image && folder {
            let rel = format!("media/{key}.{}", a.ext);
            entry["src"] = json!(rel);
            media.push((rel, a.bytes.to_vec()));
        } else {
            entry["blob"] = json!(add_blob!(b64(&a.bytes)));
        }
        assets.push(entry);
    }
    // What the page needs to play each video: trim, loop, muted.
    let videos: BTreeMap<&str, serde_json::Value> = doc
        .elements
        .values()
        .filter_map(|e| match &e.kind {
            ElementKind::Video(v) => Some((
                e.id.as_str(),
                json!({"asset": v.asset, "loop": v.looping, "muted": v.muted, "start": v.start, "end": v.end, "alt": v.alt, "poster": v.poster}),
            )),
            _ => None,
        })
        .collect();
    let background = {
        let th = editor.theme();
        let c = th.color(&th.background);
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    };
    let manifest = json!({
        "wasm": wasm,
        "doc": doc_id,
        "fonts": fonts,
        "assets": assets,
        "videos": videos,
        "soundtrack": doc.soundtrack,
        "labels": labels,
        "title": doc.title,
        "background": background,
    });

    let mut html = String::with_capacity(other_bytes + 64 * 1024);
    html.push_str("<!doctype html>\n<html><head><meta charset=\"utf-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\n");
    html.push_str("<meta name=\"generator\" content=\"Zebra\">\n");
    html.push_str(&format!("<title>{}</title>\n<style>\n{PLAYER_CSS}</style>\n</head>\n", esc_html(&doc.title)));
    html.push_str(&format!("<body style=\"background:{}\">\n", manifest["background"].as_str().unwrap_or("#fff")));
    html.push_str("<div id=\"zw-stage\"><canvas id=\"zw-canvas\"></canvas><div id=\"zw-media\"></div><div id=\"zw-blank\"></div></div>\n");
    html.push_str("<noscript>JavaScript is needed to show this presentation.</noscript>\n");
    html.push_str(&format!("<script type=\"application/json\" id=\"zw-manifest\">{}</script>\n", script_json(&manifest)));
    for (id, data) in &blobs {
        html.push_str(&format!("<script type=\"application/octet-stream\" id=\"{id}\">{data}</script>\n"));
    }
    html.push_str("<script>\n");
    html.push_str(PLAYER_JS);
    html.push_str("</script>\n</body></html>\n");
    Ok(Page { html, media })
}

/// Build and write the page. A single page goes to `path`; a folder
/// export goes to a folder named after `path` (without `.html`) holding
/// `index.html` and `media/`.
pub fn export(renderer: &Renderer, editor: &Editor, labels: &Labels, path: &Path) -> Result<Written, ZError> {
    export_with_limit(renderer, editor, labels, path, INLINE_LIMIT)
}

pub fn export_with_limit(renderer: &Renderer, editor: &Editor, labels: &Labels, path: &Path, inline_limit: usize) -> Result<Written, ZError> {
    let page = build(renderer, editor, labels, inline_limit)?;
    let ioe = |p: &Path, e: std::io::Error| ZError::new("io", format!("{}: {e}", p.display()));
    if !page.is_folder() {
        std::fs::write(path, &page.html).map_err(|e| ioe(path, e))?;
        return Ok(Written { page: path.to_path_buf(), folder: false, bytes: page.html.len() as u64 });
    }
    let dir = path.with_extension("");
    let dir = if dir == path { path.with_extension("web") } else { dir };
    std::fs::create_dir_all(dir.join("media")).map_err(|e| ioe(&dir, e))?;
    let index = dir.join("index.html");
    std::fs::write(&index, &page.html).map_err(|e| ioe(&index, e))?;
    let mut bytes = page.html.len() as u64;
    for (rel, data) in &page.media {
        let p = dir.join(rel);
        std::fs::write(&p, data).map_err(|e| ioe(&p, e))?;
        bytes += data.len() as u64;
    }
    Ok(Written { page: index, folder: true, bytes })
}
