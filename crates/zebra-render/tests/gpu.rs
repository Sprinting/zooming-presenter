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
        assert!(far_frac < 0.01, "{theme}: too many differing pixels ({far_frac})");
    }
}
