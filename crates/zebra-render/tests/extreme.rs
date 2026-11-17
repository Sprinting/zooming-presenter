//! Regression tests for extreme geometry: huge on-screen frames and very deep
//! zoom (bugs found while recording the demo video).
#![cfg(feature = "gpu")]

mod common;

use std::time::Instant;

use common::*;
use egui_wgpu::wgpu;
use zebra_core::*;
use zebra_render::cpu::render_rgba;
use zebra_render::gpu::GpuRenderer;
use zebra_render::{CaptureTarget, Renderer};

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() })).ok()
}

fn cam(cx: f64, cy: f64, zoom: f64) -> Camera {
    Camera { cx, cy, zoom, rotation: 0.0 }
}

fn nonblank_fraction(px: &[u8], bg: [u8; 4]) -> f64 {
    let n = px.len() / 4;
    px.chunks(4).filter(|p| (0..3).any(|i| (p[i] as i32 - bg[i] as i32).abs() > 8)).count() as f64 / n as f64
}

#[test]
fn huge_frames_on_screen() {
    let r = Renderer::new();
    let mut ed = sample(&r, "classic");
    // Each frame style, enormous, centred near the camera; plus a huge rotated shape.
    for (i, st) in FrameStyle::ALL.iter().enumerate() {
        add(&mut ed, &format!("big{i}"), 10.0 * i as f64, 0.0, 1e6, 7e5, 1.0, 3.0 * i as f64, frame(*st));
    }
    add(&mut ed, "bigs", 0.0, 0.0, 9e5, 9e5, 1.0, 17.0, shape(ShapeKind::Ellipse, Some("theme:shape1"), Some("theme:title"), 20.0));
    let gpu = device();
    let mut g = gpu.as_ref().map(|(d, _)| GpuRenderer::new(d).unwrap());
    for zoom in [1e-3, 1.0, 50.0, 1e3] {
        let ed2 = { let mut e = sample(&r, "classic"); e.doc = ed.doc.clone(); e.view.viewport = ed.view.viewport; e };
        let t = Instant::now();
        let list = r.capture_display_list(&ed2, &CaptureTarget::Camera { camera: cam(3.0, 2.0, zoom) }, 1280, 800).unwrap();
        let built = t.elapsed();
        let _ = render_rgba(&list);
        let cpu = t.elapsed();
        if let (Some((d, q)), Some(g)) = (&gpu, g.as_mut()) {
            g.render_to_rgba(d, q, &list).expect("gpu render");
        }
        let all = t.elapsed();
        eprintln!("zoom {zoom}: build {built:?} cpu {cpu:?} total {all:?}");
        assert!(all.as_secs_f64() < 5.0, "too slow at zoom {zoom}: {all:?}");
    }
}

#[test]
fn deep_zoom_and_back_keeps_images() {
    let Some((d, q)) = device() else { return };
    let r = Renderer::new();
    let mut ed = sample(&r, "classic");
    // A tiny detail hidden inside the image area.
    add(&mut ed, "tiny", 450.0, -120.0, 0.0, 0.0, 1e-7, 0.0, text("secret", TextStyle::Body));
    let mut g = GpuRenderer::new(&d).unwrap();
    let (w, h) = (960u32, 600u32);
    let shot = |g: &mut GpuRenderer, c: Camera| {
        let list = r.capture_display_list(&ed, &CaptureTarget::Camera { camera: c }, w, h).unwrap();
        (g.render_to_rgba(&d, &q, &list).expect("gpu render"), render_rgba(&list), list.background)
    };
    let overview = ed.fit_all_camera();
    let (before, _, bg) = shot(&mut g, overview);
    let frac_before = nonblank_fraction(&before, bg);
    // Fly in to ~1.7e7:1 via intermediate zooms, like a camera flight does.
    let f = CameraFlight::new(overview, cam(450.0, -120.0, 1.7e7), 0.0, Vec2::new(w as f64, h as f64), false);
    for i in 0..=40 {
        let c = f.sample(f.duration * i as f64 / 40.0, Vec2::new(w as f64, h as f64));
        let (gp, cp, bg) = shot(&mut g, c);
        assert!(c.zoom.is_finite() && c.cx.is_finite());
        // GPU must agree with CPU on how much is drawn.
        let (a, b) = (nonblank_fraction(&gp, bg), nonblank_fraction(&cp, bg));
        assert!((a - b).abs() < 0.05, "step {i} zoom {}: gpu {a} vs cpu {b}", c.zoom);
    }
    let back = CameraFlight::new(cam(450.0, -120.0, 1.7e7), overview, 0.0, Vec2::new(w as f64, h as f64), false);
    for i in 0..=40 {
        let c = back.sample(back.duration * i as f64 / 40.0, Vec2::new(w as f64, h as f64));
        let (gp, cp, bg) = shot(&mut g, c);
        let (a, b) = (nonblank_fraction(&gp, bg), nonblank_fraction(&cp, bg));
        assert!((a - b).abs() < 0.05, "back step {i} zoom {}: gpu {a} vs cpu {b}", c.zoom);
    }
    let (after, _, bg) = shot(&mut g, overview);
    image::RgbaImage::from_raw(w, h, after.clone()).unwrap().save(out_dir().join("deepzoom_after.png")).unwrap();
    let frac_after = nonblank_fraction(&after, bg);
    assert!((frac_after - frac_before).abs() < 0.01, "content lost after deep zoom: {frac_before} -> {frac_after}");
}

#[test]
fn deep_zoom_into_big_title_and_image() {
    let Some((d, q)) = device() else { return };
    let r = Renderer::new();
    let ed = sample(&r, "classic");
    let mut g = GpuRenderer::new(&d).unwrap();
    // t1 is a 72pt title at (0,-340); i1 an image at (450,-120).
    for (cx, cy) in [(-200.0, -340.0), (450.0, -120.0)] {
        for zoom in [10.0, 1e3, 1e5, 1e6] {
            let list = r.capture_display_list(&ed, &CaptureTarget::Camera { camera: cam(cx, cy, zoom) }, 960, 600).unwrap();
            let t = Instant::now();
            g.render_to_rgba(&d, &q, &list).expect("gpu render");
            let _ = render_rgba(&list);
            eprintln!("({cx},{cy}) zoom {zoom}: {:?}", t.elapsed());
        }
    }
}

/// A display list that bypasses clipping and overflows vello's buffers must
/// return an error (not panic), and the next normal frame must render fully,
/// images included.
#[test]
fn gpu_recovers_after_overflow() {
    use kurbo::{Rect, Shape, Stroke};
    use zebra_render::display::{DisplayList, Item};
    let Some((d, q)) = device() else { return };
    let r = Renderer::new();
    let ed = sample(&r, "classic");
    let mut g = GpuRenderer::new(&d).unwrap();
    let good = r.capture_display_list(&ed, &CaptureTarget::Overview, 640, 400).unwrap();
    let before = g.render_to_rgba(&d, &q, &good).unwrap();
    let bad = DisplayList {
        width_px: 640,
        height_px: 400,
        background: [255; 4],
        items: vec![Item::Stroke {
            path: Rect::new(-5e7, -5e7, 5e7, 200.0).to_path(0.1),
            color: [0, 0, 0, 255],
            stroke: Stroke::new(1.0).with_dashes(0.0, [6.0, 4.0]),
        }],
    };
    let res = g.render_to_rgba(&d, &q, &bad);
    eprintln!("overflow render: {:?}", res.as_ref().err());
    let after = g.render_to_rgba(&d, &q, &good).unwrap();
    let diff = before.iter().zip(&after).filter(|(a, b)| (**a as i32 - **b as i32).abs() > 8).count();
    assert!(diff < before.len() / 1000, "frame after the failure differs: {diff} bytes");
}
