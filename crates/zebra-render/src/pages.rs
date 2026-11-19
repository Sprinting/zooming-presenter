//! Pages of imported PDFs, drawn sharp at any zoom.
//!
//! An imported page is a picture element whose [`zebra_core::PageRef`]
//! points at the PDF, kept once in the document as an asset. Up to the
//! size of the page's own picture (made at import) that picture is drawn,
//! like any photo. Zoomed in further, the visible part of the page is
//! drawn again from the PDF at the zoom it is seen at, in [`TILE`]-pixel
//! tiles made on the image cache's workers and cached and evicted like
//! photos. The page's picture shows under them until they land.
//!
//! The renderer can't read PDFs itself (the web player has no PDF engine):
//! the app hands it a [`PageRaster`]. Without one, as in the web player,
//! thumbnails and exports, the page's picture is what is drawn.
use std::sync::Arc;

use zebra_core::AssetBytes;

/// Side of a tile, in pixels.
pub const TILE: u32 = 512;
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
