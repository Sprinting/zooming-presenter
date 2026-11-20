//! Imported PDF pages: the page's picture up to its own size; zoomed in
//! further, tiles drawn from the PDF through the app's page raster, made
//! on the workers when drawing is async; in PDF export, the page drawn
//! from the PDF at export size; in a web page export, a big picture.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use zebra_core::*;
use zebra_render::display::Item;
use zebra_render::pages::{PageRequest, EXPORT_PAGE_PX, MAX_PAGE_PX, TILE};
use zebra_render::{CaptureTarget, Renderer, SceneOptions};

const PDF: &str = "pdfkey";

/// A 200x100 page whose picture is red, on the canvas at 0,0.
fn deck(r: &Renderer) -> Editor {
    deck_with(r, 200)
}

/// A 200x100 page whose red picture is `pic_w` pixels wide.
fn deck_with(r: &Renderer, pic_w: u32) -> Editor {
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(400.0, 400.0);
    let img = image::RgbaImage::from_pixel(pic_w, pic_w / 2, image::Rgba([230, 0, 0, 255]));
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
fn without_a_raster_the_picture_is_drawn() {
    let r = Renderer::new();
    let ed = deck(&r);
    let l = r.build_display_list(&ed, &cam(20.0), [400.0, 400.0], 1.0, &sync());
    assert_eq!(images(&l), vec![(200, 100)]);
    // A raster that fails leaves the picture showing.
    r.set_page_raster(Some(Arc::new(|_r: &PageRequest| None)));
    let l = r.build_display_list(&ed, &cam(20.0), [400.0, 400.0], 1.0, &sync());
    assert_eq!(images(&l), vec![(200, 100)]);
}

#[test]
fn a_small_preview_gives_way_to_tiles_at_normal_sizes() {
    let r = Renderer::new();
    let ed = deck_with(&r, 512);
    let asked = Arc::new(Mutex::new(vec![]));
    r.set_page_raster(Some(green(asked.clone())));
    // Under 512 px on screen: the preview.
    r.build_display_list(&ed, &cam(2.0), [1200.0, 800.0], 1.0, &sync());
    assert!(asked.lock().unwrap().is_empty());
    // 1000 px on screen (it was the picture up to 2048 before): tiles of
    // the page drawn 1024 wide, all of it, on top of the preview.
    let l = r.build_display_list(&ed, &cam(5.0), [1200.0, 800.0], 1.0, &sync());
    let q = asked.lock().unwrap().clone();
    assert_eq!(q.len(), 2, "{:?}", q.iter().map(|q| q.rect).collect::<Vec<_>>());
    assert!(q.iter().all(|q| q.page_px == (1024, 512)));
    assert_eq!(images(&l)[0], (512, 256), "the preview under the tiles");
    assert!(!l.images_pending);
}

#[test]
fn a_page_filling_a_5k_screen_is_drawn_in_bigger_tiles() {
    let r = Renderer::new();
    let ed = deck_with(&r, 512);
    let asked = Arc::new(Mutex::new(vec![]));
    r.set_page_raster(Some(green(asked.clone())));
    // 5120 x 2880 device pixels, the page 4900 px wide: drawn 5792 wide
    // (levels are √2 apart: 4096, 5792, 8192).
    let l = r.build_display_list(&ed, &cam(12.25), [2560.0, 1440.0], 2.0, &sync());
    let q = asked.lock().unwrap().clone();
    assert!(!q.is_empty(), "the tile cap must not bite at fit-to-screen");
    assert!(q.iter().all(|q| q.page_px == (5792, 2896) && q.rect.2 == 724));
    // Every tile on screen: the page is 8 x 4 tiles, all visible.
    assert_eq!(q.len(), 32);
    assert!(!l.images_pending);
}

#[test]
fn zoomed_in_on_a_big_screen_the_biggest_tiles_still_come() {
    let r = Renderer::new();
    let ed = deck_with(&r, 512);
    let asked = Arc::new(Mutex::new(vec![]));
    r.set_page_raster(Some(green(asked.clone())));
    // A 2560 x 1440 screen 4x past fitting the page: 10000 px wide, drawn
    // 11584 wide in 1024 px tiles (the biggest; whole tiles reach past
    // the screen).
    let l = r.build_display_list(&ed, &cam(25.0), [1280.0, 720.0], 2.0, &sync());
    let q = asked.lock().unwrap().clone();
    assert!(!q.is_empty(), "fell back to the preview");
    assert!(q.iter().all(|q| q.page_px.0 == 11584 && q.rect.2 == 1024));
    assert!(!l.images_pending);
}

#[test]
fn pdf_export_draws_the_page_from_the_pdf_at_export_size() {
    let r = Renderer::new();
    let ed = deck_with(&r, 512);
    let asked = Arc::new(Mutex::new(vec![]));
    r.set_page_raster(Some(green(asked.clone())));
    let pdf = r.export_pdf(&ed).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    // Asked at once (export can't wait for workers), for the whole page,
    // at export size: not the 512 px preview.
    let q = asked.lock().unwrap().clone();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].page_px, (EXPORT_PAGE_PX, EXPORT_PAGE_PX / 2));
    assert_eq!(q[0].rect, (0, 0, EXPORT_PAGE_PX, EXPORT_PAGE_PX / 2));
    // The file holds that picture and not the preview.
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains(&format!("/Width {EXPORT_PAGE_PX}")), "no {EXPORT_PAGE_PX} px image");
    assert!(!text.contains("/Width 512"));
    // Without a raster, the picture.
    r.set_page_raster(None);
    let text = String::from_utf8_lossy(&r.export_pdf(&ed).unwrap()).to_string();
    assert!(text.contains("/Width 512"));
}

#[test]
fn old_big_pictures_still_decide() {
    // Imported before previews were small: the 2048 picture up to its size,
    // and in PDF export (as big as export would draw).
    let r = Renderer::new();
    let ed = deck_with(&r, 2048);
    let asked = Arc::new(Mutex::new(vec![]));
    r.set_page_raster(Some(green(asked.clone())));
    r.build_display_list(&ed, &cam(5.0), [1200.0, 800.0], 1.0, &sync());
    r.export_pdf(&ed).unwrap();
    assert!(asked.lock().unwrap().is_empty());
}

#[test]
fn web_export_pictures_are_big_and_the_open_document_keeps_its_preview() {
    let r = Renderer::new();
    let ed = deck_with(&r, 512);
    assert!(zebra_render::pages::export_pictures(&r, &ed.doc).is_none(), "no raster: nothing to do");
    let asked = Arc::new(Mutex::new(vec![]));
    r.set_page_raster(Some(green(asked.clone())));
    let before = ed.doc.clone();
    let out = zebra_render::pages::export_pictures(&r, &ed.doc).expect("pictures made");
    let page = |d: &Document| {
        let e = d.elements.values().find(|e| matches!(&e.kind, ElementKind::Image(i) if i.page.is_some())).unwrap();
        let ElementKind::Image(i) = &e.kind else { unreachable!() };
        d.assets[&i.asset].clone()
    };
    let big = page(&out);
    assert_eq!((big.width, big.height), (EXPORT_PAGE_PX, EXPORT_PAGE_PX / 2));
    let px = image::load_from_memory(&big.bytes).unwrap().to_rgba8();
    assert_eq!(px.get_pixel(10, 10).0, [0, 200, 0, 255], "drawn from the PDF");
    assert_eq!(ed.doc, before, "the open document is unchanged");
    assert_eq!(page(&ed.doc).width, 512);
    // Pictures that big already: nothing to do.
    let old = deck_with(&r, 2048);
    assert!(zebra_render::pages::export_pictures(&r, &old.doc).is_none());
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
