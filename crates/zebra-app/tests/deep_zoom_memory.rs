//! Regression: zooming deep into a deck must not grow memory. Invisible
//! frames' dashed outlines (and the agent's dashed selection, and path view
//! boxes) were dashed whole every frame, even far off screen: at zoom 10,000
//! that was millions of egui shapes a frame, and Zebra reached 12 GB.
//!
//! Its own test binary: it counts every allocation.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use egui_kittest::Harness;
use serde_json::json;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Camera, Command, Mode, Origin};

struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Past this the test can't pass; stop before the machine starts swapping.
const ABORT_AT: usize = 3 << 30;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let n = NOW.fetch_add(l.size(), Relaxed) + l.size();
        if n > ABORT_AT {
            NOW.fetch_sub(l.size(), Relaxed);
            return std::ptr::null_mut();
        }
        PEAK.fetch_max(n, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        NOW.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static A: Counting = Counting;

const MB: usize = 1 << 20;

fn apply(h: &mut Harness<'static, ZebraApp>, j: serde_json::Value) -> Vec<String> {
    let cmd: Command = serde_json::from_value(j).unwrap();
    h.state_mut().editor.apply(cmd, Origin::Human).unwrap().created
}

/// Every element kind, near a point deep inside a nest of frames and far
/// from it; invisible frames of every size; view and overview steps.
fn build(h: &mut Harness<'static, ZebraApp>) -> (f64, f64, Vec<String>) {
    let mut far = vec![];
    for (i, st) in ["bracket", "circle", "rectangle", "invisible"].iter().enumerate() {
        for k in 0..6 {
            let x = 15000.0 + 1100.0 * k as f64;
            far.extend(apply(h, json!({"op":"add_frame","style":st,"x":x,"y":-3000.0 + 900.0 * i as f64,"w":900,"h":700,"rotation":15 * k,"add_to_path":true})));
        }
    }
    for (i, s) in ["rectangle", "ellipse", "star", "cloud", "arrow", "line", "speech_bubble", "heart"].iter().enumerate() {
        far.extend(apply(h, json!({"op":"add_shape","shape":s,"x":-9000.0 - 400.0 * i as f64,"y":4000,"w":300,"h":200,"fill":"#e33","stroke":"#333"})));
    }
    let id = far.last().unwrap().clone();
    apply(h, json!({"op":"update","id":id,"patch":{"dash":"dashed","stroke_width":6}}));
    far.extend(apply(h, json!({"op":"add_text","text":"Far away text 🦓 with an emoji","x":-8000,"y":-5000,"style":"title"})));
    far.extend(apply(h, json!({"op":"add_icon","name":"lightbulb","x":-7000,"y":-5000,"size":200})));
    far.extend(apply(h, json!({"op":"add_ink","points":[[0,0],[300,200],[600,0]],"width":12})));
    far.extend(apply(h, json!({"op":"add_step","target":{"kind":"view","view":{"cx":20000,"cy":0,"width":4000,"height":2500,"rotation":30}}})));
    far.extend(apply(h, json!({"op":"add_step","target":{"kind":"overview"}})));
    // The nest: 9 levels, each 1/6 of the last, text inside each.
    let (mut cx, mut cy, mut w) = (-14000.0f64, -6000.0f64, 3000.0f64);
    for l in 0..9 {
        let st = ["rectangle", "circle", "bracket", "invisible"][l % 4];
        apply(h, json!({"op":"add_frame","style":st,"x":cx,"y":cy,"w":w,"h":w * 0.7,"rotation":12 * l,"add_to_path":true}));
        apply(h, json!({"op":"add_text","text":format!("Level {l}"),"x":cx,"y":cy - w * 0.2,"style":"title","scale":w / 1500.0}));
        cx += w * 0.25;
        cy += w * 0.12;
        w /= 6.0;
    }
    (cx, cy, far)
}

fn settle(h: &mut Harness<'static, ZebraApp>) {
    for _ in 0..4 {
        h.step();
    }
}

#[test]
fn deep_zoom_keeps_memory_flat() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    let (cx, cy, far) = build(&mut h);
    // The agent has every far element selected: its dashed selection too.
    h.state_mut().editor.view.agent_selection = far.clone();
    settle(&mut h);
    let base = NOW.load(Relaxed);
    for mode in [Mode::Edit, Mode::PathEdit] {
        h.state_mut().editor.view.mode = mode;
        let mut zoom = 1.0;
        while zoom <= 1e9 {
            h.state_mut().editor.view.camera = Camera { cx, cy, zoom, rotation: 40.0 };
            PEAK.store(NOW.load(Relaxed), Relaxed);
            settle(&mut h);
            let peak = PEAK.load(Relaxed).saturating_sub(base);
            let now = NOW.load(Relaxed).saturating_sub(base);
            eprintln!("{mode:?} zoom {zoom:e}: peak +{} MB, now +{} MB", peak / MB, now / MB);
            assert!(peak < 400 * MB, "{mode:?} at zoom {zoom:e}: frame used {} MB", peak / MB);
            assert!(now < 200 * MB, "{mode:?} at zoom {zoom:e}: {} MB kept", now / MB);
            zoom *= 10.0;
        }
    }
    // Back out: what was used comes back.
    h.state_mut().editor.view.mode = Mode::Edit;
    h.state_mut().editor.view.camera = Camera { cx, cy, zoom: 0.02, rotation: 0.0 };
    settle(&mut h);
    let now = NOW.load(Relaxed).saturating_sub(base);
    assert!(now < 150 * MB, "{} MB still held after zooming back out", now / MB);
}
