//! P6: a picture bigger than 8,192 pixels stays sharp up to its own
//! pixels when zoomed in: the part on screen is drawn again from the file
//! in tiles, as a PDF page's is.

mod common;
use common::*;
use zebra_core::*;
use zebra_render::{CaptureTarget, Renderer};

/// A `w`x`h` grey PNG of one-pixel vertical stripes (black, white),
/// written row by row.
fn stripes_png(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![];
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().unwrap();
        let mut s = wr.stream_writer().unwrap();
        let row: Vec<u8> = (0..w).map(|x| if x % 2 == 0 { 0 } else { 255 }).collect();
        for _ in 0..h {
            std::io::Write::write_all(&mut s, &row).unwrap();
        }
        s.finish().unwrap();
    }
    out
}

/// The same stripes, two pixels wide, as a grey JPEG (one-pixel stripes
/// don't survive JPEG).
fn stripes_jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::GrayImage::from_fn(w, h, |x, _| image::Luma([if x / 2 % 2 == 0 { 0 } else { 255 }]));
    let mut out = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 100).encode_image(&img).unwrap();
    out
}

fn deck(bytes: Vec<u8>, w: f64, h: f64) -> (Renderer, Editor) {
    let r = Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    // One capture pixel per screen point.
    ed.view.viewport = Vec2::new(256.0, 64.0);
    let key = ed.add_asset(bytes).unwrap();
    add(&mut ed, "p", 0.0, 0.0, w, h, 1.0, 0.0, ElementKind::Image(ImageData { asset: key, alt: String::new(), crop: None, mask: ImageMask::Rectangle, symbol: None, page: None }));
    (r, ed)
}

/// The grey of row `y` of a capture centred on world (cx, 0) at `zoom`
/// screen pixels per picture pixel.
fn row(r: &Renderer, ed: &Editor, cx: f64, zoom: f64, y: u32) -> Vec<u8> {
    let cam = Camera { cx, cy: 0.0, zoom, rotation: 0.0 };
    let png = r.capture(ed, &CaptureTarget::Camera { camera: cam }, 256, 64).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    (0..img.width()).map(|x| img.get_pixel(x, y)[0]).collect()
}

/// Whether `row` is stripes `period` screen pixels apart: each pixel
/// matches the one a period on, and each period spans most of black to
/// white (a soft picture is grey; moiré doesn't repeat this exactly).
fn striped(row: &[u8], period: usize) -> bool {
    let n = row.len() - period;
    let same = (0..n).filter(|&i| row[i].abs_diff(row[i + period]) < 40).count();
    let deep = (0..n).filter(|&i| {
        let w = &row[i..i + period];
        w.iter().max().unwrap() - w.iter().min().unwrap() > 150
    });
    same * 10 >= n * 9 && deep.count() * 10 >= n * 9
}

/// A 20,000 x 600 PNG zoomed so each of its pixels is 4 on screen: its
/// one-pixel stripes show. From its 8,192 top level they were grey.
#[test]
fn a_wide_png_is_sharp_at_its_own_pixels() {
    let (r, ed) = deck(stripes_png(20_000, 600), 20_000.0, 600.0);
    // Zoomed out it is the pyramid, as before: grey at under 1:2.
    let far = row(&r, &ed, 3000.0, 0.25, 32);
    assert!(!striped(&far, 8), "zoomed out it shouldn't show stripes");
    // Left part, middle and right end: every tile comes from the file.
    for cx in [-9000.0, 0.5, 9_900.25] {
        let near = row(&r, &ed, cx, 4.0, 32);
        assert!(striped(&near, 8), "at {cx}: {:?}", &near[..32]);
    }
}

/// The same for a JPEG (decoded whole, as its top level is, in builds
/// without ImageIO).
#[test]
fn a_big_jpeg_is_sharp_at_its_own_pixels() {
    let (r, ed) = deck(stripes_jpeg(10_000, 400), 10_000.0, 400.0);
    let near = row(&r, &ed, 1234.0, 2.0, 32);
    assert!(striped(&near, 8), "{:?}", &near[..32]);
}

/// A picture under 8,192 pixels has no detail tiles: drawn as before.
#[test]
fn small_pictures_draw_as_before() {
    let (r, ed) = deck(stripes_png(4000, 100), 4000.0, 100.0);
    let near = row(&r, &ed, 10.0, 4.0, 32);
    assert!(striped(&near, 8));
}

/// On screen (pictures decoded on workers) the top level shows at once
/// and the tiles land after; every tile on screen is made, from one job.
#[test]
fn on_screen_the_tiles_land_after_the_top_level() {
    use zebra_render::{display::Item, SceneOptions};
    let (r, ed) = deck(stripes_png(20_000, 600), 20_000.0, 600.0);
    let o = SceneOptions { async_images: true, ..Default::default() };
    let cam = Camera { cx: 5000.0, cy: 0.0, zoom: 4.0, rotation: 0.0 };
    let sizes = |l: &zebra_render::DisplayList| l.items.iter().filter_map(|i| if let Item::Image { image, .. } = i { Some(image.width) } else { None }).collect::<Vec<_>>();
    let t0 = std::time::Instant::now();
    let mut list = r.build_display_list(&ed, &cam, [1024.0, 512.0], 1.0, &o);
    while list.images_pending || sizes(&list).len() < 2 {
        assert!(t0.elapsed().as_secs() < 30, "tiles never landed: {:?}", sizes(&list));
        std::thread::sleep(std::time::Duration::from_millis(5));
        list = r.build_display_list(&ed, &cam, [1024.0, 512.0], 1.0, &o);
    }
    let s = sizes(&list);
    // The 8,192 top level under, then tiles covering 1024 x 512 screen
    // pixels at 1:4 (256 x 128 of the picture's own).
    assert_eq!(s[0], 8192, "{s:?}");
    assert!(s.len() >= 2 && s[1..].iter().all(|&w| w < 8192), "{s:?}");
}
