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
    (pw / 8).clamp(TILE, 4 * TILE)
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

