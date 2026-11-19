//! What importing a PDF adds to a document, against the alternative of a
//! picture pyramid per page (no PDF kept):
//!
//!   ZEBRA_PDFIUM=…/libpdfium.so cargo run -p zebra-import --example page_sizes -- deck.pdf [pages]
//!
//! Prints the PDF's size, the pictures import makes, and the pyramids
//! (PNG at 8192, 4096 … 512 px on the long side, or JPEG where that is
//! much smaller), measured on the first `pages` pages (default 5) and
//! scaled to the whole file.
use std::time::Instant;

use zebra_import::{Control, Options};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a PDF");
    let sample: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let reader = zebra_import::open(std::path::Path::new(&path)).expect("open");
    let pages = reader.page_count();
    let n = sample.min(pages);
    let theme = zebra_core::Editor::default().theme();
    let mut o = Options::all(theme);
    o.last = n;
    let t = Instant::now();
    let deck = reader.read(&o, &Control::default()).expect("read");
    let secs = t.elapsed().as_secs_f64();
    let pdf = std::fs::metadata(&path).unwrap().len() as f64;
    let pics: f64 = deck.assets.values().filter(|a| a.ext != "pdf").map(|a| a.bytes.len() as f64).sum();
    let mb = |b: f64| b / 1e6;
    println!("{path}: {pages} pages; measured on {n}");
    println!("  import: {:.2} s/page", secs / n as f64);
    println!("  PDF kept once:            {:>7.1} MB", mb(pdf));
    println!("  page pictures ({} px):  {:>7.1} MB  ({:.0} KB/page)", zebra_import::pdf::PICTURE_PX, mb(pics / n as f64 * pages as f64), pics / n as f64 / 1e3);
    println!("  document adds:            {:>7.1} MB", mb(pdf + pics / n as f64 * pages as f64));
    // The pyramid instead.
    let lib = zebra_import::pdf::pdfium().unwrap();
    let doc = lib.load_pdf_from_byte_vec(std::fs::read(&path).unwrap(), None).unwrap();
    let bytes = zebra_core::AssetBytes::from(std::fs::read(&path).unwrap());
    let mut total = 0.0;
    for i in 0..n {
        let page = doc.pages().get(i as i32).unwrap();
        let (w, h) = (page.width().value as f64, page.height().value as f64);
        let mut long = 8192u32;
        while long >= 512 {
            let k = long as f64 / w.max(h);
            let (pw, ph) = ((w * k).round() as u32, (h * k).round() as u32);
            // Drawn and encoded in pieces of 4096 at most.
            for y in (0..ph).step_by(4096) {
                for x in (0..pw).step_by(4096) {
                    let (dw, dh) = ((pw - x).min(4096), (ph - y).min(4096));
                    let px = zebra_import::pdf::raster("m", &bytes, i as u32, (pw, ph), (x, y, dw, dh)).expect("draw");
                    let rgb = image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(dw, dh, px).unwrap()).to_rgb8();
                    let mut png = vec![];
                    image::DynamicImage::ImageRgb8(rgb.clone()).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
                    let mut jpg = vec![];
                    rgb.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90)).unwrap();
                    total += if jpg.len() * 2 < png.len() { jpg.len() } else { png.len() } as f64;
                }
            }
            long /= 2;
        }
    }
    println!("  pyramids to 8192 px:      {:>7.1} MB  ({:.1} MB/page)", mb(total / n as f64 * pages as f64), mb(total / n as f64));
    let fifty = |per: f64| mb(per * 50.0);
    println!("  a 50-page deck like this: PDF + pictures {:.1} MB; pyramids {:.1} MB", mb(pdf / pages as f64 * 50.0) + fifty(pics / n as f64), fifty(total / n as f64));
}
