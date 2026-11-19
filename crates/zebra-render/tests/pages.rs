//! Imported PDF pages: the page's picture up to its own size; zoomed in
//! further, tiles drawn from the PDF through the app's page raster, made
//! on the workers when drawing is async, never in PDF export.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use zebra_core::*;
use zebra_render::display::Item;
use zebra_render::pages::{PageRequest, MAX_PAGE_PX, TILE};
use zebra_render::{CaptureTarget, Renderer, SceneOptions};

const PDF: &str = "pdfkey";

/// A 200x100 page whose picture is red, on the canvas at 0,0.
fn deck(r: &Renderer) -> Editor {
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(400.0, 400.0);
    let img = image::RgbaImage::from_pixel(200, 100, image::Rgba([230, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let key = ed.add_asset(png).unwrap();
    ed.doc.assets.insert(PDF.into(), Asset { ext: "pdf".into(), bytes: b"%PDF-1.4 stand-in".to_vec().into(), width: 0, height: 0 });
    let add = serde_json::json!({"op":"add_element","element":{"x":0,"y":0,"w":200,"h":100,
        "kind":{"type":"image","asset":key,"alt":"","page":{"pdf":PDF,"index":3}}}});
    ed.apply(serde_json::from_value(add).unwrap(), Origin::Human).unwrap();
    ed
}

/// A raster that draws green and notes what it was asked for.
fn green(asked: Arc<Mutex<Vec<PageRequest>>>) -> zebra_render::pages::PageRaster {
    Arc::new(move |r: &PageRequest| {
        asked.lock().unwrap().push(r.clone());
        Some([0u8, 200, 0, 255].repeat((r.rect.2 * r.rect.3) as usize))
    })
}

fn images(list: &zebra_render::DisplayList) -> Vec<(u32, u32)> {
    list.items.iter().filter_map(|i| if let Item::Image { image, .. } = i { Some((image.width, image.height)) } else { None }).collect()
}

fn cam(zoom: f64) -> Camera {
    Camera { cx: 0.0, cy: 0.0, zoom, rotation: 0.0 }
}

fn sync() -> SceneOptions {
    SceneOptions { async_images: false, ..Default::default() }
}

#[test]
fn the_picture_up_to_its_size_then_tiles_of_the_visible_part() {
    let r = Renderer::new();
    let ed = deck(&r);
    let asked = Arc::new(Mutex::new(vec![]));
    r.set_page_raster(Some(green(asked.clone())));
    // Page on screen at its picture's size: just the picture.
    let l = r.build_display_list(&ed, &cam(1.0), [400.0, 400.0], 1.0, &sync());
    assert_eq!(images(&l), vec![(200, 100)]);
    assert!(asked.lock().unwrap().is_empty());
    // 20x: the page is 4000 px wide on screen, drawn 4096 wide; only the
    // tiles under the 400x400 screen are made.
    let l = r.build_display_list(&ed, &cam(20.0), [400.0, 400.0], 1.0, &sync());
    let imgs = images(&l);
    assert_eq!(imgs[0], (200, 100), "the picture under the tiles");
    assert!(imgs[1..].iter().all(|(w, h)| *w <= TILE && *h <= TILE));
    let asked = asked.lock().unwrap().clone();
    assert!((1..=4).contains(&asked.len()), "{}", asked.len());
    for q in &asked {
        assert_eq!((q.pdf.as_str(), q.index), (PDF, 3));
        assert_eq!(q.page_px, (4096, 2048));
        // Around the page's middle (the screen's centre).
        let (x, y) = (q.rect.0 + q.rect.2 / 2, q.rect.1 + q.rect.3 / 2);
        assert!((1024..3072).contains(&x) && (512..1536).contains(&y), "{:?}", q.rect);
    }
    assert!(!l.images_pending);
    // Drawn: the tiles' green, not the picture's red.
    let png = r.capture(&ed, &CaptureTarget::Camera { camera: cam(20.0) }, 400, 400).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    for (x, y) in [(5, 5), (200, 200), (395, 395)] {
        let p = img.get_pixel(x, y).0;
        assert!(p[1] > 150 && p[0] < 60, "({x},{y}) is {p:?}");
    }
    // Far in, the page is drawn no wider than MAX_PAGE_PX.
    let asked2 = Arc::new(Mutex::new(vec![]));
    r.set_page_raster(Some(green(asked2.clone())));
    r.build_display_list(&ed, &cam(5000.0), [400.0, 400.0], 1.0, &sync());
    assert!(asked2.lock().unwrap().iter().all(|q| q.page_px.0 == MAX_PAGE_PX));
}

#[test]
fn without_a_raster_and_in_pdf_export_the_picture_is_drawn() {
    let r = Renderer::new();
    let ed = deck(&r);
    let l = r.build_display_list(&ed, &cam(20.0), [400.0, 400.0], 1.0, &sync());
    assert_eq!(images(&l), vec![(200, 100)]);
    // PDF export draws pictures at full size: the page's picture.
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    r.set_page_raster(Some(Arc::new(move |_r: &PageRequest| {
        c.fetch_add(1, Ordering::Relaxed);
        None
    })));
    let pdf = r.export_pdf(&ed).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    // A raster that fails leaves the picture showing.
    let l = r.build_display_list(&ed, &cam(20.0), [400.0, 400.0], 1.0, &sync());
    assert_eq!(images(&l), vec![(200, 100)]);
}

#[test]
fn tiles_are_made_on_the_workers_and_the_half_zoom_ones_fill_in() {
    let r = Renderer::new();
    let ed = deck(&r);
    let woke = Arc::new(AtomicUsize::new(0));
    let w = woke.clone();
    r.images().set_waker(Some(Arc::new(move || {
        w.fetch_add(1, Ordering::Relaxed);
    })));
    let asked = Arc::new(Mutex::new(vec![]));
    r.set_page_raster(Some(green(asked.clone())));
    let o = SceneOptions { async_images: true, ..Default::default() };
    let settle = |zoom: f64| {
        for _ in 0..400 {
            let l = r.build_display_list(&ed, &cam(zoom), [400.0, 400.0], 1.0, &o);
            if !l.images_pending {
                return l;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("tiles never landed");
    };
    let first = r.build_display_list(&ed, &cam(10.0), [400.0, 400.0], 1.0, &o);
    assert!(first.images_pending);
    let l = settle(10.0);
    let at10 = images(&l).len();
    assert!(at10 >= 2, "picture and tiles");
    assert!(woke.load(Ordering::Relaxed) > 0);
    // Zoomed in to the next size: while its tiles are made, the cached
    // half-zoom tiles under them are drawn instead of the soft picture.
    let l = r.build_display_list(&ed, &cam(20.0), [400.0, 400.0], 1.0, &o);
    assert!(l.images_pending);
    assert!(images(&l).len() >= 2, "{:?}", images(&l));
    settle(20.0);
}
