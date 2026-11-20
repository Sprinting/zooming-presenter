//! How much the helper process costs against drawing in-process (as Zebra
//! did before): start-up, page previews and tiles, on the PDFs given.
//!
//!   ZEBRA_PDFIUM=…/libpdfium.so cargo run --release -p zebra-pdf --example bench -- a.pdf [b.pdf …]
//!
//! (Build the helper first, in the same profile: cargo build --release
//! -p zebra-pdf --bin zebra-pdf-helper.)
use std::time::{Duration, Instant};

use pdfium_render::prelude::*;
use zebra_core::AssetBytes;
use zebra_pdf::{Lane, Limits, Pool};

const PAGES: u32 = 20;

fn tiles(pw: u32, ph: u32) -> Vec<(u32, u32, u32, u32)> {
    let t = (pw / 8).clamp(512, 1024);
    let mut v = vec![];
    let mut y = 0;
    while y < ph {
        let mut x = 0;
        while x < pw {
            v.push((x, y, t.min(pw - x), t.min(ph - y)));
            x += t;
        }
        y += t;
    }
    v
}

fn main() {
    let lib = std::env::var_os("ZEBRA_PDFIUM").expect("ZEBRA_PDFIUM");
    let exe = std::env::current_exe().unwrap();
    let helper = exe.parent().unwrap().parent().unwrap().join("zebra-pdf-helper");
    let t = Instant::now();
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).unwrap());
    let bind = t.elapsed();
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).unwrap();
        println!("\n{path}: {} KB", bytes.len() / 1024);

        // In-process, as before: bind, a copy of the PDF per worker, draw.
        let me = std::process::id();
        let heap0 = zebra_pdf::sys::footprint(me).unwrap_or(0);
        let t = Instant::now();
        let doc = pdfium.load_pdf_from_byte_vec(bytes.clone(), None).unwrap();
        let open = t.elapsed();
        let n = (doc.pages().len() as u32).min(PAGES);
        let size = doc.pages().page_size(0).unwrap();
        let (w, h) = (size.width().value as f64, size.height().value as f64);
        let fit = |long: u32| {
            let k = long as f64 / w.max(h);
            (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1))
        };
        let t = Instant::now();
        for i in 0..n {
            let page = doc.pages().get(i as i32).unwrap();
            let (pw, ph) = fit(512);
            let cfg = PdfRenderConfig::new().set_target_size(pw as i32, ph as i32).render_form_data(true).render_annotations(true);
            let _ = page.render_with_config(&cfg).unwrap().as_rgba_bytes();
            let _ = page.text().map(|t| t.all());
        }
        let previews_in = t.elapsed();
        let mut count = 0;
        let t = Instant::now();
        // As master's pdf::raster did: the page loaded for each tile.
        for i in 0..n {
            for long in [1448, 2896] {
                let (pw, ph) = fit(long);
                for (x, y, tw, th) in tiles(pw, ph) {
                    let page = doc.pages().get(i as i32).unwrap();
                    let mut bmp = PdfBitmap::empty(tw as i32, th as i32, PdfBitmapFormat::BGRA).unwrap();
                    let cfg = PdfRenderConfig::new().set_target_size(pw as i32, ph as i32).set_origin(-(x as i32), -(y as i32)).render_form_data(true).render_annotations(true);
                    page.render_into_bitmap_with_config(&mut bmp, &cfg).unwrap();
                    let _ = bmp.as_rgba_bytes();
                    count += 1;
                }
            }
        }
        let tiles_in = t.elapsed();
        let heap_in = zebra_pdf::sys::footprint(me).unwrap_or(0).saturating_sub(heap0);
        drop(doc);

        // Through the helper.
        let pool = Pool::new(Limits::default(), Some(helper.clone()));
        let b = AssetBytes::from(bytes.clone());
        let t = Instant::now();
        pool.check().unwrap();
        let start = t.elapsed();
        let t = Instant::now();
        pool.open(Lane::Draw, "k", &b, &|| false).unwrap();
        let open_h = t.elapsed();
        let t = Instant::now();
        for i in 0..n {
            pool.preview(Lane::Draw, "k", &b, i, 512, &|| false).unwrap();
        }
        let previews_h = t.elapsed();
        let t = Instant::now();
        for i in 0..n {
            for long in [1448, 2896] {
                let (pw, ph) = fit(long);
                for r in tiles(pw, ph) {
                    pool.rect(Lane::Draw, "k", &b, i, (pw, ph), r, &|| false).unwrap();
                }
            }
        }
        let tiles_h = t.elapsed();
        let helper_mem = pool.pid(Lane::Draw).and_then(zebra_pdf::sys::footprint).unwrap_or(0);
        // Start-up again, warm (the program and PDFium in the page cache).
        let mut warm = vec![];
        for _ in 0..5 {
            pool.release(Lane::Draw);
            let t = Instant::now();
            pool.check().unwrap();
            warm.push(t.elapsed());
        }
        warm.sort();
        let per = |d: Duration, n: u32| d.as_secs_f64() * 1000.0 / n.max(1) as f64;
        println!("  start: in-process bind {:.1} ms | helper spawn + bind {:.1} ms (warm median {:.1} ms)", per(bind, 1), per(start, 1), per(warm[2], 1));
        println!("  open:  in-process {:.1} ms | helper {:.1} ms", per(open, 1), per(open_h, 1));
        println!("  previews (512 px + text), {n} pages: in-process {:.1} ms/page | helper {:.1} ms/page", per(previews_in, n), per(previews_h, n));
        println!(
            "  tiles at 1448 and 2896 px, {count}: in-process {:.2} ms/tile ({:.0}/s) | helper {:.2} ms/tile ({:.0}/s), {:+.0}%",
            per(tiles_in, count),
            count as f64 / tiles_in.as_secs_f64(),
            per(tiles_h, count),
            count as f64 / tiles_h.as_secs_f64(),
            (tiles_h.as_secs_f64() / tiles_in.as_secs_f64() - 1.0) * 100.0
        );
        println!("  memory after the tiles: in-process +{} MB in the app | helper {} MB (its own process)", heap_in >> 20, helper_mem >> 20);
    }
}
