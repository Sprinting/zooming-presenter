//! PDF pages, each page as one picture, read and drawn by the PDF helper.
//!
//! The PDF goes into the document once, as an asset; each page becomes a
//! picture element pointing at its page ([`zebra_core::PageRef`]), over a
//! small picture of the page made here ([`PREVIEW_PX`] on its long side).
//! The renderer draws pages again from the PDF when they are seen bigger
//! than that picture ([`raster`], handed to it by the app), so they stay
//! sharp on screen and in exports; thumbnails and the sidebar draw the
//! picture, and so does the page until its tiles land. The PDF is the
//! page; the picture is only a preview, so a 300-page PDF costs a few MB
//! of pictures, not hundreds.
//!
//! PDFium never runs in Zebra's own process: it runs in the helper
//! ([`zebra_pdf`]), which the app kills when a page takes too much memory
//! or time, or when the person cancels. A page that can't be drawn comes
//! in as a placeholder (noted); after [`zebra_pdf::limits::KILLS_PER_PDF`]
//! such pages the import stops there, keeping the pages before.
use zebra_core::import::{asset_from, ImportNote, ImportedDeck, ImportedItem, ImportedPage};
use zebra_core::{AssetBytes, Element, ElementKind, ImageData, PageRef, Size, Transform};
use zebra_pdf::{Lane, PdfError, Pool};

use crate::{Control, ImportError, Options, Reader};

/// Long side of each page's picture, pixels: what thumbnails, the sidebar
/// and a page whose tiles are still being made show. Past this size the
/// page is drawn from the PDF. (Documents imported before kept 2048 px
/// pictures; they still work, the picture's own width decides.)
/// (PNG; JPEG is about half the size for slides with gradients but ten
/// times slower to make, and no smaller for plain ones.)
pub const PREVIEW_PX: u32 = 512;
/// Long side of the largest page on the canvas, world units (a default
/// frame is 800 wide). Pages keep their sizes relative to each other.
pub const PAGE_UNITS: f64 = 1000.0;
/// A PDF with more pages than this asks which to import first (File ›
/// Import… fills in 1 to this; the API wants `to`).
pub const ASK_ABOVE_PAGES: usize = 200;
/// Most pages of a PDF imported at once, whatever is asked.
pub const MAX_PAGES: usize = 2000;
/// The placeholder for a page that can't be drawn: light grey.
const PLACEHOLDER_GREY: u8 = 0xe5;

/// Can PDFs be imported here (is the helper in place, and can it load
/// PDFium)?
pub fn available() -> bool {
    Pool::global().check().is_ok()
}

impl From<PdfError> for ImportError {
    fn from(e: PdfError) -> ImportError {
        match e {
            PdfError::NoHelper(s) | PdfError::NoEngine(s) => ImportError::NoEngine(s),
            PdfError::Encrypted => ImportError::Encrypted,
            PdfError::Damaged | PdfError::Page => ImportError::Damaged("PDF"),
            PdfError::TooDemanding => ImportError::TooDemanding,
            PdfError::Cancelled => ImportError::Cancelled,
            PdfError::Io(s) => ImportError::Read(s),
        }
    }
}

pub struct PdfReader {
    /// The PDF as the document will hold it, and its content key.
    key: String,
    bytes: AssetBytes,
    title: String,
    pages: usize,
}

impl PdfReader {
    /// `bytes` mapped from the person's file are copied to a spill file of
    /// the document's own first (streamed, not read into memory): the
    /// document must not depend on a file the person may move or change.
    /// Then the helper counts the pages (`ctl` cancels).
    pub fn new(bytes: AssetBytes, title: String, ctl: &Control) -> Result<PdfReader, ImportError> {
        let (key, bytes) = if bytes.is_mapped() && bytes.len() >= zebra_core::media::SPILL_MIN {
            zebra_core::media::spill_reader(&mut std::io::Cursor::new(&bytes[..]), "pdf").map_err(|e| ImportError::Read(e.to_string()))?
        } else {
            let key = zebra_core::media::content_key(&bytes);
            let bytes = match bytes {
                AssetBytes::Memory(v) => zebra_core::media::spill(std::sync::Arc::try_unwrap(v).unwrap_or_else(|v| (*v).clone()), &key, "pdf"),
                mapped => mapped,
            };
            (key, bytes)
        };
        let pool = Pool::global();
        let pages = pool.open(Lane::Import, &key, &bytes, &|| ctl.cancelled());
        // The person may take a while to choose pages: no helper meanwhile.
        pool.release(Lane::Import);
        let pages = pages? as usize;
        if pages == 0 {
            return Err(ImportError::Damaged("PDF"));
        }
        Ok(PdfReader { key, bytes, title, pages })
    }
}

/// Pixel size of a `w` x `h` page with `long` pixels on its long side.
fn fit(w: f64, h: f64, long: u32) -> (u32, u32) {
    let k = long as f64 / w.max(h).max(1.0);
    (((w * k).round() as u32).clamp(1, long), ((h * k).round() as u32).clamp(1, long))
}

/// PNG bytes, in a buffer no bigger than they are (a growing Vec has up
/// to twice the room it uses, and import keeps one per page).
fn encode(img: &image::RgbImage) -> Vec<u8> {
    let mut png = vec![];
    let _ = img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
    png.shrink_to_fit();
    png
}

/// Stops the import's helper however the import ends.
struct Release;

impl Drop for Release {
    fn drop(&mut self) {
        Pool::global().release(Lane::Import);
    }
}

/// A page read: its index, size in points, alt text and picture
/// (None: a placeholder).
type Page = (usize, f64, f64, String, Option<image::RgbImage>);

impl Reader for PdfReader {
    fn kind(&self) -> &'static str {
        "PDF"
    }
    fn page_count(&self) -> usize {
        self.pages
    }
    fn read(&self, opts: &Options, ctl: &Control) -> Result<ImportedDeck, ImportError> {
        let pool = Pool::global();
        let _release = Release;
        let cancel = || ctl.cancelled();
        let range = crate::page_range(opts, self.pages);
        if range.len() > MAX_PAGES {
            return Err(ImportError::TooManyPages { pages: self.pages });
        }
        if range.is_empty() {
            return Err(ImportError::Damaged("PDF"));
        }
        let mut deck = ImportedDeck { title: self.title.clone(), ..Default::default() };
        let (pdf_key, pdf) = (self.key.clone(), zebra_core::Asset { ext: "pdf".into(), bytes: self.bytes.clone(), width: 0, height: 0 });
        let sizes = pool.sizes(Lane::Import, &self.key, &self.bytes, range.start as u32, (range.end - 1) as u32, &cancel)?;
        // World units per point, from the largest page asked for.
        let largest = sizes.iter().map(|(w, h)| w.max(*h)).fold(0.0f64, f64::max);
        let k = PAGE_UNITS / largest.max(1.0);
        // A page whose size can't be read is the size of the largest.
        let size_of = |i: usize| match sizes.get(i - range.start) {
            Some(&(w, h)) if w > 0.0 && h > 0.0 => (w, h),
            _ => (largest.max(1.0), largest.max(1.0)),
        };
        // Pages are drawn in the helper, one at a time (PDFium is
        // single-threaded), and encoded here on two workers meanwhile.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Page>(2);
        let rx = std::sync::Mutex::new(rx);
        let encoded = std::sync::Mutex::new(vec![]);
        let result = std::thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| {
                    loop {
                        let Ok((i, w, h, alt, img)) = rx.lock().unwrap_or_else(|e| e.into_inner()).recv() else { return };
                        let bytes = match img {
                            Some(img) => encode(&img),
                            None => {
                                let (pw, ph) = fit(w, h, PREVIEW_PX);
                                encode(&image::RgbImage::from_pixel(pw, ph, image::Rgb([PLACEHOLDER_GREY; 3])))
                            }
                        };
                        encoded.lock().unwrap_or_else(|e| e.into_inner()).push((i, w * k, h * k, alt, bytes));
                        ctl.page_done();
                    }
                });
            }
            for i in range.clone() {
                if ctl.cancelled() {
                    return Err(ImportError::Cancelled);
                }
                let page = match pool.preview(Lane::Import, &self.key, &self.bytes, i as u32, PREVIEW_PX, &cancel) {
                    Ok(p) => image::RgbImage::from_raw(p.px.0, p.px.1, p.rgb).map(|img| (p.w_pt, p.h_pt, p.text, img)),
                    Err(PdfError::Cancelled) => return Err(ImportError::Cancelled),
                    Err(e @ (PdfError::NoHelper(_) | PdfError::NoEngine(_) | PdfError::Io(_))) => return Err(e.into()),
                    Err(PdfError::TooDemanding) => {
                        if pool.given_up(&self.key, None) {
                            // Too many pages like this: stop here, keep
                            // what came in before.
                            deck.notes.insert(ImportNote::Stopped, i + 1);
                            break;
                        }
                        let (w, h) = size_of(i);
                        if tx.send((i, w, h, String::new(), None)).is_err() {
                            break;
                        }
                        *deck.notes.entry(ImportNote::Placeholder).or_default() += 1;
                        continue;
                    }
                    Err(_) => None,
                };
                match page {
                    Some((w, h, alt, img)) => {
                        if tx.send((i, w, h, alt, Some(img))).is_err() {
                            break;
                        }
                    }
                    // A page PDFium can't open is left out, not the whole file.
                    None => {
                        *deck.notes.entry(ImportNote::PageSkipped).or_default() += 1;
                        ctl.page_done();
                    }
                }
            }
            drop(tx);
            Ok(())
        });
        result?;
        if ctl.cancelled() {
            return Err(ImportError::Cancelled);
        }
        let mut encoded = encoded.into_inner().unwrap_or_else(|e| e.into_inner());
        encoded.sort_by_key(|e| e.0);
        if encoded.is_empty() {
            return Err(if deck.notes.contains_key(&ImportNote::Stopped) { ImportError::TooDemanding } else { ImportError::Damaged("PDF") });
        }
        for (i, w, h, alt, bytes) in encoded {
            let Ok((key, asset)) = asset_from(bytes) else {
                *deck.notes.entry(ImportNote::PageSkipped).or_default() += 1;
                continue;
            };
            deck.assets.insert(key.clone(), asset);
            let page = PageRef { pdf: pdf_key.clone(), index: i as u32 };
            let element = Element {
                id: String::new(),
                transform: Transform { x: w / 2.0, y: h / 2.0, scale: 1.0, rotation: 0.0 },
                size: Size { w, h },
                locked: false,
                name: String::new(),
                placeholder: false,
                group: None,
                kind: ElementKind::Image(ImageData { asset: key, alt, crop: None, mask: Default::default(), symbol: None, page: Some(page) }),
            };
            deck.pages.push(ImportedPage { w, h, name: (opts.page_name)(i + 1), items: vec![ImportedItem { element, fit: None }], notes: String::new(), slide: false, voiceover: None });
        }
        deck.assets.insert(pdf_key, pdf);
        Ok(deck)
    }
}

/// Draw part of page `index` of the PDF `bytes` (asset key `key`) for
/// the renderer (see `zebra_render::pages`): `rect` (x, y, width, height)
/// of the page drawn `page_px` pixels big, as straight RGBA rows. Runs on
/// the image cache's workers and export threads; the helper keeps the
/// PDF it last drew from open. None when the page can't be drawn (its
/// picture shows instead; see [`take_draw_failures`]).
pub fn raster(key: &str, bytes: &AssetBytes, index: u32, page_px: (u32, u32), rect: (u32, u32, u32, u32)) -> Option<Vec<u8>> {
    Pool::global().rect(Lane::Draw, key, bytes, index, page_px, rect, &|| false).ok()
}

/// The PDFs (asset keys) that `mine` claims some of whose pages couldn't
/// be drawn sharper: each PDF once per session, for a note.
pub fn take_draw_failures(mine: impl Fn(&str) -> bool) -> Vec<String> {
    Pool::global().take_draw_failures(mine)
}
