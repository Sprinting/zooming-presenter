//! Pages of imported PDFs, drawn sharp at any zoom.
//!
//! An imported page is a picture element whose [`zebra_core::PageRef`]
//! points at the PDF, kept once in the document as an asset. Up to the
//! size of the page's own picture (a small preview made at import; 2048
//! px in documents imported before) that picture is drawn, like any
//! photo. Seen bigger, the visible part of the page is drawn again from
//! the PDF at the zoom it is seen at, in tiles ([`tile_for`]) made on the
//! image cache's workers and cached and evicted like photos. The page's
//! picture shows under them until they land. Captures draw the tiles at
//! once; PDF export draws the visible part as one picture at least
//! [`EXPORT_PAGE_PX`] across.
//!
//! The renderer can't read PDFs itself (the web player has no PDF engine):
//! the app hands it a [`PageRaster`]. Without one, as in the web player,
//! the page's picture is what is drawn; a web page export swaps in a
//! bigger picture per page first ([`export_pictures`]).
use std::sync::Arc;

use zebra_core::AssetBytes;

/// Side of a tile, in pixels, for pages up to 4096 px across.
pub const TILE: u32 = 512;
/// Most pixels of tiles made for one page in one frame, at least (more
/// on a bigger screen: four times its pixels).
pub const MAX_TILE_PIXELS: u64 = 96 * (TILE as u64) * (TILE as u64);
/// Long side of a page drawn for an export, at least.
pub const EXPORT_PAGE_PX: u32 = 2048;
/// Biggest part of a page drawn in one go, pixels per side.
pub const MAX_RECT_PX: u32 = 4096;

/// Side of the tiles of a page drawn `pw` pixels across: bigger tiles for
/// bigger pages, so a page filling a 5K screen is ~40 tiles, not ~150
/// (each tile is the whole page drawn again, clipped).
pub fn tile_for(pw: u32) -> u32 {
    (pw / 8).clamp(TILE, 2 * TILE)
}

/// Width to draw a page `on_screen` pixels across at: the smallest level
/// `TILE * √2^k` that covers it (capped at [`MAX_PAGE_PX`]). Every second
/// level is a power of two, so half a level is a level.
pub fn page_level(on_screen: f64) -> u32 {
    let mut k = 0u32;
    loop {
        let pw = level(k);
        if pw as f64 >= on_screen || pw >= MAX_PAGE_PX {
            return pw.min(MAX_PAGE_PX);
        }
        k += 1;
    }
}

fn level(k: u32) -> u32 {
    // Odd levels are 724 << m exactly, so half of one is the level below.
    let base = if k % 2 == 0 { TILE } else { (TILE as f64 * std::f64::consts::SQRT_2).round() as u32 };
    base << (k / 2)
}
/// Widest a page is ever drawn (pixels across the whole page). Beyond
/// this zoom the tiles are scaled up.
pub const MAX_PAGE_PX: u32 = 1 << 17;

/// A part of one page to draw.
#[derive(Clone)]
pub struct PageRequest {
    /// Asset key of the PDF (the same file keeps the same key).
    pub pdf: String,
    pub bytes: AssetBytes,
    /// 0-based page number.
    pub index: u32,
    /// Size of the whole page as drawn, pixels.
    pub page_px: (u32, u32),
    /// The part wanted: x, y, width, height in those pixels.
    pub rect: (u32, u32, u32, u32),
}

/// Draws `rect` of a page: straight-alpha RGBA rows, `rect.2 * rect.3 * 4`
/// bytes, or None when the page can't be drawn.
pub type PageRaster = Arc<dyn Fn(&PageRequest) -> Option<Vec<u8>> + Send + Sync>;

/// For a web page export (the player has no PDF engine and draws each
/// page's picture): a copy of `doc` whose PDF pages show a picture
/// [`EXPORT_PAGE_PX`] on its long side drawn now from the PDF, in place
/// of the small preview kept in the document. None when nothing changes
/// (no PDF pages, no page raster, or pictures that big already, as in
/// documents imported before previews were small). The open document is
/// left as it is.
#[cfg(not(target_arch = "wasm32"))]
pub fn export_pictures(r: &crate::Renderer, doc: &zebra_core::Document) -> Option<zebra_core::Document> {
    use std::collections::BTreeMap;
    use zebra_core::ElementKind;
    let raster = r.page_raster()?;
    // Each page once, however often it is on the canvas: (pdf, index) ->
    // pixel size.
    let mut wanted: BTreeMap<(String, u32), (u32, u32)> = BTreeMap::new();
    for e in doc.elements.values() {
        let ElementKind::Image(img) = &e.kind else { continue };
        let (Some(p), Some(pic)) = (&img.page, doc.assets.get(&img.asset)) else { continue };
        if pic.width.max(pic.height) >= EXPORT_PAGE_PX || pic.width == 0 || pic.height == 0 || !doc.assets.contains_key(&p.pdf) {
            continue;
        }
        let k = EXPORT_PAGE_PX as f64 / pic.width.max(pic.height) as f64;
        let size = (((pic.width as f64 * k).round() as u32).max(1), ((pic.height as f64 * k).round() as u32).max(1));
        wanted.insert((p.pdf.clone(), p.index), size);
    }
    if wanted.is_empty() {
        return None;
    }
    // Drawn one at a time (PDFium is single-threaded), encoded on two
    // workers meanwhile.
    let (tx, rx) = std::sync::mpsc::sync_channel::<((String, u32), u32, u32, Vec<u8>)>(2);
    let rx = std::sync::Mutex::new(rx);
    let made = std::sync::Mutex::new(BTreeMap::new());
    std::thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| loop {
                let Ok((page, w, h, px)) = rx.lock().unwrap_or_else(|e| e.into_inner()).recv() else { return };
                let Some(rgba) = image::RgbaImage::from_raw(w, h, px) else { continue };
                let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
                let mut png = vec![];
                if rgb.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_ok() {
                    png.shrink_to_fit();
                    made.lock().unwrap_or_else(|e| e.into_inner()).insert(page, png);
                }
            });
        }
        for ((pdf, index), (w, h)) in &wanted {
            let req = PageRequest { pdf: pdf.clone(), bytes: doc.assets[pdf].bytes.clone(), index: *index, page_px: (*w, *h), rect: (0, 0, *w, *h) };
            // A page that can't be drawn keeps its preview.
            let Some(px) = raster(&req).filter(|px| px.len() == (*w * *h * 4) as usize) else { continue };
            if tx.send(((pdf.clone(), *index), *w, *h, px)).is_err() {
                break;
            }
        }
        drop(tx);
    });
    let made = made.into_inner().unwrap_or_else(|e| e.into_inner());
    if made.is_empty() {
        return None;
    }
    let mut out = doc.clone();
    let mut keys = BTreeMap::new();
    for (page, png) in made {
        let Ok((key, asset)) = zebra_core::import::asset_from(png) else { continue };
        out.assets.insert(key.clone(), asset);
        keys.insert(page, key);
    }
    for e in out.elements.values_mut() {
        let ElementKind::Image(img) = &mut e.kind else { continue };
        let Some(p) = &img.page else { continue };
        if let Some(k) = keys.get(&(p.pdf.clone(), p.index)) {
            img.asset = k.clone();
        }
    }
    Some(out)
}
