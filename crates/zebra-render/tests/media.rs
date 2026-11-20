//! Crops, masks, GIF frames and video stand-ins in the display list and
//! the CPU renderer.
mod common;

use std::sync::Arc;

use base64::Engine;
use common::*;
use serde_json::json;
use zebra_core::*;
use zebra_render::display::{ImageLevel, Item};
use zebra_render::{CaptureTarget, Renderer, SceneOptions};

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}
fn decode(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory(png).unwrap().to_rgba8()
}
fn px(img: &image::RgbaImage, x: u32, y: u32) -> [u8; 4] {
    img.get_pixel(x, y).0
}
fn close(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}
/// Left half red, right half blue, 200x100.
fn halves_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(200, 100, |x, _| if x < 100 { image::Rgba([230, 0, 0, 255]) } else { image::Rgba([0, 0, 230, 255]) });
    let mut v = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
    v
}
fn editor(r: &Renderer) -> Editor {
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.view.viewport = Vec2::new(400.0, 400.0);
    ed
}
fn cam(zoom: f64) -> CaptureTarget {
    CaptureTarget::Camera { camera: Camera { cx: 0.0, cy: 0.0, zoom, rotation: 0.0 } }
}

#[test]
fn crop_shows_only_the_cropped_part_and_circle_mask_clips_corners() {
    let r = Renderer::new();
    let mut ed = editor(&r);
    let id = ed.apply(serde_json::from_value(json!({"op":"add_image","data_base64":b64(&halves_png()),"x":0,"y":0})).unwrap(), Origin::Human).unwrap().created[0].clone();
    // Keep only the blue half: the box becomes 100x100 centred at x=50.
    ed.apply(serde_json::from_value(json!({"op":"update","id":id,"patch":{"crop":{"x":0.5,"y":0,"w":0.5,"h":1}}})).unwrap(), Origin::Human).unwrap();
    ed.apply(serde_json::from_value(json!({"op":"update","id":id,"patch":{"x":0}})).unwrap(), Origin::Human).unwrap();
    let img = decode(&r.capture(&ed, &cam(2.0), 400, 400).unwrap());
    std::fs::write(out_dir().join("crop.png"), r.capture(&ed, &cam(2.0), 400, 400).unwrap()).unwrap();
    // The box spans 100..300 px; all of it is blue, nothing red.
    assert!(close(px(&img, 110, 200), [0, 0, 230, 255], 10), "{:?}", px(&img, 110, 200));
    assert!(close(px(&img, 290, 200), [0, 0, 230, 255], 10));
    assert!(!img.pixels().any(|p| p.0[0] > 150 && p.0[2] < 80), "red leaked past the crop");
    // Circle mask: corners show the background, the centre the picture.
    ed.apply(serde_json::from_value(json!({"op":"update","id":id,"patch":{"mask":"circle"}})).unwrap(), Origin::Human).unwrap();
    let png = r.capture(&ed, &cam(2.0), 400, 400).unwrap();
    std::fs::write(out_dir().join("crop-circle.png"), &png).unwrap();
    let img = decode(&png);
    let bg = px(&img, 2, 2);
    assert!(close(px(&img, 105, 105), bg, 6), "corner should be clipped: {:?}", px(&img, 105, 105));
    assert!(close(px(&img, 200, 200), [0, 0, 230, 255], 10));
    // PDF export accepts clips.
    assert!(r.export_pdf(&ed).unwrap().starts_with(b"%PDF"));
}

#[test]
fn small_on_screen_images_use_small_levels() {
    let r = Renderer::new();
    let mut ed = editor(&r);
    let big = image::RgbaImage::from_pixel(2048, 1024, image::Rgba([1, 2, 3, 255]));
    let mut v = vec![];
    big.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
    ed.apply(serde_json::from_value(json!({"op":"add_image","data_base64":b64(&v),"width":2048})).unwrap(), Origin::Human).unwrap();
    let opts = SceneOptions::default();
    let at = |zoom: f64| {
        let list = r.build_display_list(&ed, &Camera { cx: 0.0, cy: 0.0, zoom, rotation: 0.0 }, [400.0, 400.0], 1.0, &opts);
        list.items.iter().find_map(|i| if let Item::Image { image, .. } = i { Some(image.width) } else { None }).unwrap()
    };
    assert_eq!(at(0.1), 256); // 205 px on screen
    assert_eq!(at(1.0), 2048);
    assert!(r.images().stats().bytes < 16 << 20);
}

#[test]
fn async_decode_reports_pending_then_completes() {
    let r = Renderer::new();
    let mut ed = editor(&r);
    ed.apply(serde_json::from_value(json!({"op":"add_image","data_base64":b64(&checker_png()),"width":100})).unwrap(), Origin::Human).unwrap();
    let opts = SceneOptions { async_images: true, ..Default::default() };
    let c = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let first = r.build_display_list(&ed, &c, [200.0, 200.0], 1.0, &opts);
    assert!(first.images_pending);
    let t0 = std::time::Instant::now();
    loop {
        let l = r.build_display_list(&ed, &c, [200.0, 200.0], 1.0, &opts);
        if !l.images_pending {
            assert!(l.items.iter().any(|i| matches!(i, Item::Image { .. })));
            break;
        }
        assert!(t0.elapsed().as_secs() < 10, "decode never finished");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn animated_gif_follows_the_clock() {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};
    let mut gif = vec![];
    {
        let mut enc = GifEncoder::new(&mut gif);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for c in [[230u8, 0, 0, 255], [0, 200, 0, 255]] {
            let img = image::RgbaImage::from_pixel(50, 50, image::Rgba(c));
            enc.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(500, 1))).unwrap();
        }
    }
    let r = Renderer::new();
    let mut ed = editor(&r);
    ed.apply(serde_json::from_value(json!({"op":"add_image","data_base64":b64(&gif),"width":100})).unwrap(), Origin::Human).unwrap();
    let c = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let color_at = |t: Option<f64>| {
        let list = r.build_display_list(&ed, &c, [200.0, 200.0], 1.0, &SceneOptions { time: t, ..Default::default() });
        (decode_list(&list), list.next_frame_in)
    };
    let (a, next) = color_at(Some(0.1));
    assert!(close(a, [230, 0, 0, 255], 12), "{a:?}");
    assert!((next.unwrap() - 0.4).abs() < 1e-6);
    let (b, _) = color_at(Some(0.7));
    assert!(close(b, [0, 200, 0, 255], 12), "{b:?}");
    // Captures (no clock) show frame 1 and don't ask for repaints.
    let (c0, n0) = color_at(None);
    assert!(close(c0, [230, 0, 0, 255], 12) && n0.is_none());
}

fn decode_list(list: &zebra_render::DisplayList) -> [u8; 4] {
    let rgba = zebra_render::cpu::render_rgba(list);
    let (w, h) = (list.width_px as usize, list.height_px as usize);
    let i = (h / 2 * w + w / 2) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

#[test]
fn video_shows_poster_placeholder_or_live_frame() {
    unsafe { std::env::set_var("ZEBRA_MEDIA_CACHE", std::env::temp_dir().join(format!("zebra-rt-{}", std::process::id()))) };
    let r = Renderer::new();
    let mut ed = editor(&r);
    let mp4 = zebra_core::media::fixtures::tiny_mp4(160, 90, 4, true);
    let id = ed
        .apply(serde_json::from_value(json!({"op":"add_video","data_base64":b64(&mp4),"width":320})).unwrap(), Origin::Human)
        .unwrap()
        .created[0]
        .clone();
    let c = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
    let opts = SceneOptions::default();
    let list = r.build_display_list(&ed, &c, [400.0, 400.0], 1.0, &opts);
    // No poster: dark box plus the play badge, no image.
    assert!(!list.items.iter().any(|i| matches!(i, Item::Image { .. })));
    assert!(close(decode_list(&list), [255, 255, 255, 255], 40), "badge should sit in the middle");
    let png = r.capture(&ed, &cam(1.0), 400, 400).unwrap();
    std::fs::write(out_dir().join("video-placeholder.png"), &png).unwrap();
    assert!(close(px(&decode(&png), 60, 200), [29, 29, 31, 255], 4));

    // Poster.
    ed.apply(serde_json::from_value(json!({"op":"set_poster","id":id,"data_base64":b64(&halves_png())})).unwrap(), Origin::Human).unwrap();
    let png = r.capture(&ed, &cam(1.0), 400, 400).unwrap();
    std::fs::write(out_dir().join("video-poster.png"), &png).unwrap();
    let img = decode(&png);
    assert!(close(px(&img, 60, 200), [230, 0, 0, 255], 10));
    assert!(close(px(&img, 340, 200), [0, 0, 230, 255], 10));

    // A live frame replaces the poster; playing hides the badge.
    let frame = Arc::new(ImageLevel::from_rgba("frame", 4, 4, [0u8, 180, 0, 255].repeat(16)));
    let mut opts = SceneOptions::default();
    opts.video_frames.insert(id.clone(), frame);
    opts.playing.insert(id.clone());
    let list = r.build_display_list(&ed, &c, [400.0, 400.0], 1.0, &opts);
    assert!(close(decode_list(&list), [0, 180, 0, 255], 4));
    assert!(r.export_pdf(&ed).unwrap().starts_with(b"%PDF"));
}

/// Cloud pass: PDF export decoded every PNG whole to embed it, so a
/// 70,000-pixel one needed 19 GB. Now a picture whose decoded size is
/// past the export's budget goes in as the pixels drawn for the page (as
/// Keynote's Reduce File Size scales to the resolution shown), and the
/// export works. 12,000 x 12,000 (576 MB decoded) is past it.
#[test]
fn a_huge_png_exports_to_pdf_at_the_size_shown() {
    let (w, h) = (12_000u32, 12_000u32);
    let mut file = vec![];
    {
        let mut enc = png::Encoder::new(&mut file, w, h);
        enc.set_color(png::ColorType::Rgb);
        let mut wr = enc.write_header().unwrap();
        let mut sw = wr.stream_writer().unwrap();
        let row: Vec<u8> = (0..w).flat_map(|x| if x < w / 2 { [230u8, 0, 0] } else { [0, 0, 230] }).collect();
        for _ in 0..h {
            std::io::Write::write_all(&mut sw, &row).unwrap();
        }
        sw.finish().unwrap();
    }
    assert!((w as u64 * h as u64 * 4) > zebra_render::PDF_DECODE_BUDGET);
    let r = Renderer::new();
    let mut ed = editor(&r);
    ed.apply(serde_json::from_value(json!({"op":"add_image","data_base64":b64(&file),"x":0,"y":0,"width":400})).unwrap(), Origin::Human).unwrap();
    let pdf = r.export_pdf(&ed).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    let text = String::from_utf8_lossy(&pdf);
    assert!(!text.contains("/Width 12000"), "the original went in whole");
    assert!(text.contains("/Subtype /Image"), "a picture went in");
}
