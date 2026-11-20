//! vello GPU path on a headless wgpu device (lavapipe here; Metal on macOS),
//! compared against the vello_cpu reference.
#![cfg(feature = "gpu")]

mod common;

use common::*;
use egui_wgpu::wgpu;
use zebra_render::cpu::render_rgba;
use zebra_render::gpu::GpuRenderer;
use zebra_render::{CaptureTarget, Renderer};

fn device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }))
    .ok()?;
    let name = format!("{:?} {}", adapter.get_info().backend, adapter.get_info().name);
    let (d, q) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("zebra test"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()?;
    Some((d, q, name))
}

#[test]
fn gpu_matches_cpu() {
    let Some((device, queue, name)) = device() else {
        eprintln!("no wgpu adapter available; skipping GPU check");
        return;
    };
    eprintln!("GPU adapter: {name}");
    let r = Renderer::new();
    let mut gpu = GpuRenderer::new(&device).expect("vello init");
    for theme in ["classic", "chalkboard"] {
        let ed = sample(&r, theme);
        let (w, h) = (640u32, 400u32);
        let list = r.capture_display_list(&ed, &CaptureTarget::Overview, w, h).unwrap();
        let gpu_px = gpu.render_to_rgba(&device, &queue, &list).unwrap();
        let cpu_px = render_rgba(&list);
        assert_eq!(gpu_px.len(), cpu_px.len());
        let img = image::RgbaImage::from_raw(w, h, gpu_px.clone()).unwrap();
        img.save(out_dir().join(format!("gpu_{theme}.png"))).unwrap();
        let cimg = image::RgbaImage::from_raw(w, h, cpu_px.clone()).unwrap();
        cimg.save(out_dir().join(format!("cpu_{theme}.png"))).unwrap();
        // Compare: mean abs difference small, and nearly all pixels close
        // (anti-aliasing differs slightly between the two rasterisers).
        let mut sum = 0u64;
        let mut far = 0usize;
        for (a, b) in gpu_px.chunks(4).zip(cpu_px.chunks(4)) {
            let d = a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs()).max().unwrap();
            sum += d as u64;
            if d > 48 {
                far += 1;
            }
        }
        let n = (w * h) as f64;
        let mean = sum as f64 / n;
        let far_frac = far as f64 / n;
        eprintln!("{theme}: mean max-channel diff {mean:.3}, pixels off by >48: {:.4}%", far_frac * 100.0);
        assert!(mean < 2.0, "{theme}: GPU and CPU disagree (mean {mean})");
        assert!(far_frac < 0.001, "{theme}: too many differing pixels ({far_frac})");
    }
}

/// Many small faded groups (placeholder text greeked to bars at fit-all on
/// a big deck) used to open one full-screen layer each, overflowing vello's
/// fixed buffers: the whole frame came out black.
#[test]
fn many_opacity_groups_dont_black_out_the_frame() {
    use kurbo::{Rect, Shape};
    use zebra_render::display::{DisplayList, Item};
    let Some((device, queue, _)) = device() else { return };
    let mut gpu = GpuRenderer::new(&device).expect("vello init");
    let (w, h) = (2680u32, 1740u32);
    let mut items = Vec::new();
    for i in 0..300 {
        let (x, y) = ((i % 20) as f64 * 130.0 + 20.0, (i / 20) as f64 * 110.0 + 20.0);
        // Two items per group, so the layer can't be folded away.
        items.push(Item::PushOpacity(0.5));
        items.push(Item::Fill { path: Rect::new(x, y, x + 40.0, y + 3.0).to_path(0.1), color: [20, 20, 20, 255] });
        items.push(Item::Fill { path: Rect::new(x, y + 6.0, x + 30.0, y + 9.0).to_path(0.1), color: [20, 20, 20, 255] });
        items.push(Item::PopOpacity);
        // And single-item groups, which are folded into the colour.
        items.push(Item::PushOpacity(0.5));
        items.push(Item::Fill { path: Rect::new(x, y + 20.0, x + 40.0, y + 23.0).to_path(0.1), color: [20, 20, 20, 255] });
        items.push(Item::PopOpacity);
    }
    let list = DisplayList { width_px: w, height_px: h, background: [255, 255, 255, 255], items, ..Default::default() };
    let g = gpu.render_to_rgba(&device, &queue, &list).expect("render");
    let c = render_rgba(&list);
    let dark = g.chunks(4).filter(|p| p[0] < 4 && p[1] < 4 && p[2] < 4).count();
    assert!(dark * 10 < (w * h) as usize, "GPU frame is mostly black: {dark} px");
    let diff = g.chunks(4).zip(c.chunks(4)).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 24)).count();
    assert!(diff * 1000 < (w * h) as usize, "GPU differs from CPU in {diff} px");
}

/// Zooming a big PDF page swaps whole sets of large tiles. vello keeps the
/// last frames' pictures in one 8192² atlas and silently leaves out what
/// doesn't fit; the renderer asks for more frames until the new set shows.
#[test]
fn a_new_set_of_big_pictures_shows_after_settling() {
    use std::sync::Arc;
    use zebra_render::display::{DisplayList, Item};
    use zebra_render::images::ImageLevel;
    let Some((device, queue, _)) = device() else { return };
    let mut gpu = GpuRenderer::new(&device).expect("vello init");
    let (w, h) = (1600u32, 800u32);
    // Ten 2048² pictures (42M pixels) per set, each drawn into a 160 px cell.
    let set = |shade: u8| -> DisplayList {
        let items = (0..10u32)
            .map(|i| {
                let px = [shade, 255 - shade, (i * 20) as u8, 255].repeat(2048 * 2048);
                let image = Arc::new(ImageLevel::from_rgba(&format!("set{shade}-{i}"), 2048, 2048, px));
                let s = 160.0 / 2048.0;
                Item::Image { image, transform: kurbo::Affine::new([s, 0.0, 0.0, s, (i % 10) as f64 * 160.0, 0.0]) }
            })
            .collect();
        DisplayList { width_px: w, height_px: h, background: [0, 0, 0, 255], items, ..Default::default() }
    };
    let lit = |v: &[u8]| v.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 > 200).count();
    let a = set(40);
    gpu.render_to_rgba(&device, &queue, &a).unwrap();
    while gpu.needs_another_frame() {
        gpu.render_to_rgba(&device, &queue, &a).unwrap();
    }
    let b = set(200);
    let mut g = gpu.render_to_rgba(&device, &queue, &b).unwrap();
    let mut frames = 1;
    while gpu.needs_another_frame() {
        g = gpu.render_to_rgba(&device, &queue, &b).unwrap();
        frames += 1;
    }
    assert!(frames <= 3, "{frames} frames");
    assert_eq!(lit(&g), 1600 * 160, "every picture of the new set shows");
}
