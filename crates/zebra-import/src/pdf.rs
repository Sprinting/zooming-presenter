//! PDF pages through PDFium, each page as one picture.
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
//! PDFium is a separate library, loaded at run time by full path from the
//! app bundle and never from the system's search paths.
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;

use pdfium_render::prelude::*;
use zebra_core::import::{asset_from, ImportNote, ImportedDeck, ImportedItem, ImportedPage};
use zebra_core::{AssetBytes, Element, ElementKind, ImageData, PageRef, Size, Transform};

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

/// Where PDFium is: inside the app (Zebra.app/Contents/Frameworks on the
/// Mac, beside the program elsewhere). Debug builds and tests also take
/// `ZEBRA_PDFIUM` (the library file); see scripts/fetch-pdfium.sh.
pub fn library_path() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("ZEBRA_PDFIUM").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = Pdfium::pdfium_platform_library_name();
    let p = if cfg!(target_os = "macos") { dir.parent()?.join("Frameworks").join(name) } else { dir.join(name) };
    p.is_file().then_some(p)
}

/// The loaded PDFium (bound once per process).
pub fn pdfium() -> Result<&'static Pdfium, ImportError> {
    static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();
    PDFIUM
        .get_or_init(|| {
            let p = library_path().ok_or_else(|| "libpdfium not found".to_string())?;
            Pdfium::bind_to_library(&p).map(Pdfium::new).map_err(|e| format!("{e}"))
        })
        .as_ref()
        .map_err(|e| ImportError::NoEngine(e.clone()))
}

/// Can PDFs be imported here (is PDFium in place and loadable)?
pub fn available() -> bool {
    pdfium().is_ok()
}

fn load_error(e: PdfiumError) -> ImportError {
    match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError | PdfiumInternalError::SecurityError) => ImportError::Encrypted,
        _ => ImportError::Damaged("PDF"),
    }
}

pub struct PdfReader {
    bytes: Vec<u8>,
    title: String,
    pages: usize,
}

impl PdfReader {
    pub fn new(bytes: Vec<u8>, title: String) -> Result<PdfReader, ImportError> {
        let lib = pdfium()?;
        let pages = {
            let doc = lib.load_pdf_from_byte_slice(&bytes, None).map_err(load_error)?;
            doc.pages().len() as usize
        };
        if pages == 0 {
            return Err(ImportError::Damaged("PDF"));
        }
        Ok(PdfReader { bytes, title, pages })
    }
}

/// A page drawn at `w` x `h` pixels, white behind: RGB rows.
fn render(page: &PdfPage, w: u32, h: u32) -> Option<image::RgbImage> {
    let cfg = PdfRenderConfig::new().set_target_size(w as i32, h as i32).render_form_data(true).render_annotations(true);
    let bmp = page.render_with_config(&cfg).ok()?;
    let rgba = image::RgbaImage::from_raw(bmp.width() as u32, bmp.height() as u32, bmp.as_rgba_bytes())?;
    Some(image::DynamicImage::ImageRgba8(rgba).to_rgb8())
}

/// PNG bytes, in a buffer no bigger than they are (a growing Vec has up
/// to twice the room it uses, and import keeps one per page).
fn encode(img: &image::RgbImage) -> Vec<u8> {
    let mut png = vec![];
    let _ = img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
    png.shrink_to_fit();
    png
}

/// The page's first line of text, for the picture's description.
fn first_line(page: &PdfPage) -> String {
    let all = page.text().map(|t| t.all()).unwrap_or_default();
    let line = all.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut s: String = line.chars().take(120).collect();
    if s.len() < line.len() {
        s.push('…');
    }
    s
}

/// Pixel size of a `w` x `h` page with `long` pixels on its long side.
fn fit(w: f64, h: f64, long: u32) -> (u32, u32) {
    let k = long as f64 / w.max(h).max(1.0);
    (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1))
}

impl Reader for PdfReader {
    fn kind(&self) -> &'static str {
        "PDF"
    }
    fn page_count(&self) -> usize {
        self.pages
    }
    fn read(&self, opts: &Options, ctl: &Control) -> Result<ImportedDeck, ImportError> {
        let lib = pdfium()?;
        let doc = lib.load_pdf_from_byte_slice(&self.bytes, None).map_err(load_error)?;
        let mut deck = ImportedDeck { title: self.title.clone(), ..Default::default() };
        let (pdf_key, pdf) = asset_from(self.bytes.clone()).map_err(|_| ImportError::Damaged("PDF"))?;
        let range = crate::page_range(opts, self.pages);
        // World units per point, from the largest page asked for.
        let largest = range.clone().filter_map(|i| doc.pages().page_size(i as i32).ok()).map(|r| r.width().value.max(r.height().value)).fold(0.0f32, f32::max);
        let k = PAGE_UNITS / (largest as f64).max(1.0);
        // Pages are drawn here, one at a time (PDFium is single-threaded),
        // and encoded on two workers meanwhile.
        let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, f64, f64, String, image::RgbImage)>(2);
        let rx = std::sync::Mutex::new(rx);
        let encoded = std::sync::Mutex::new(vec![]);
        let result = std::thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| {
                    loop {
                        let Ok((i, w, h, alt, img)) = rx.lock().unwrap_or_else(|e| e.into_inner()).recv() else { return };
                        let bytes = encode(&img);
                        drop(img);
                        encoded.lock().unwrap_or_else(|e| e.into_inner()).push((i, w, h, alt, bytes));
                        ctl.page_done();
                    }
                });
            }
            for i in range.clone() {
                if ctl.cancelled() {
                    return Err(ImportError::Cancelled);
                }
                // A page PDFium can't open is left out, not the whole file.
                let drawn = doc.pages().get(i as i32).ok().and_then(|page| {
                    let (w, h) = (page.width().value as f64, page.height().value as f64);
                    let (pw, ph) = fit(w, h, PREVIEW_PX);
                    Some((w, h, first_line(&page), render(&page, pw, ph)?))
                });
                match drawn {
                    Some((w, h, alt, img)) => {
                        if tx.send((i, w * k, h * k, alt, img)).is_err() {
                            break;
                        }
                    }
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
            return Err(ImportError::Damaged("PDF"));
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
            deck.pages.push(ImportedPage { w, h, name: (opts.page_name)(i + 1), items: vec![ImportedItem { element, fit: None }], notes: String::new(), slide: false });
        }
        deck.assets.insert(pdf_key, pdf);
        Ok(deck)
    }
}

thread_local! {
    /// The PDF last drawn from on this thread, open (by asset key).
    static OPEN: RefCell<Option<(String, PdfDocument<'static>)>> = const { RefCell::new(None) };
}

/// Draw part of page `index` of the PDF `bytes` (asset key `key`) for
/// the renderer (see `zebra_render::pages`): `rect` (x, y, width, height)
/// of the page drawn `page_px` pixels big, as straight RGBA rows. Runs on
/// the image cache's workers; each keeps the PDF it last drew from open.
pub fn raster(key: &str, bytes: &AssetBytes, index: u32, page_px: (u32, u32), rect: (u32, u32, u32, u32)) -> Option<Vec<u8>> {
    let lib = pdfium().ok()?;
    let (x, y, w, h) = rect;
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    OPEN.with(|open| {
        let mut open = open.borrow_mut();
        if open.as_ref().is_none_or(|(k, _)| k != key) {
            *open = None;
            let doc = lib.load_pdf_from_byte_vec(bytes.to_vec(), None).ok()?;
            *open = Some((key.to_string(), doc));
        }
        let (_, doc) = open.as_ref()?;
        let page = doc.pages().get(i32::try_from(index).ok()?).ok()?;
        let mut bmp = PdfBitmap::empty(w as i32, h as i32, PdfBitmapFormat::BGRA).ok()?;
        let (pw, ph) = (i32::try_from(page_px.0).ok()?, i32::try_from(page_px.1).ok()?);
        let cfg = PdfRenderConfig::new()
            .set_target_size(pw, ph)
            .set_origin(-(x as i32), -(y as i32))
            .render_form_data(true)
            .render_annotations(true);
        page.render_into_bitmap_with_config(&mut bmp, &cfg).ok()?;
        let mut px = bmp.as_rgba_bytes();
        // Opaque, like the page's picture.
        for p in px.chunks_exact_mut(4) {
            p[3] = 255;
        }
        Some(px)
    })
}
