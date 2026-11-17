//! Memory used by undo history on a dense deck.
//!
//!   cargo run --release -p zebra-core --example history_mem
//!
//! Builds a deck of ~2,000 elements (frames, paragraphs, shapes, ink strokes
//! with 200 points each and a 4 MB image), then makes 500 separate human
//! edits of the usual kinds (move, retype, restyle, reorder, path step) and
//! reports how much heap the history holds.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;

use serde_json::json;
use zebra_core::command::Command;
use zebra_core::{Editor, Origin};

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        LIVE.fetch_add(n as isize - l.size() as isize, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static A: Counting = Counting;

fn live() -> f64 {
    LIVE.load(Ordering::Relaxed) as f64 / 1_048_576.0
}
fn c(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut img = image::RgbImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgb([(x * 7 ^ y * 13) as u8, (x * y) as u8, (x + y * 3) as u8]);
    }
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn main() {
    let h = Origin::Human;
    let mut ed = Editor::default();
    let mut cmds = vec![];
    let lorem = "Zooming presentations let the audience see how ideas connect. ".repeat(4);
    for i in 0..400 {
        let (x, y) = ((i % 20) as f64 * 1200.0, (i / 20) as f64 * 900.0);
        cmds.push(c(json!({"op":"add_frame","id":format!("f{i}"),"x":x,"y":y,"w":1000,"h":750,"add_to_path":true})));
        cmds.push(c(json!({"op":"add_text","id":format!("t{i}"),"text":format!("Slide {i}"),"style":"title","x":x,"y":y-250.0})));
        cmds.push(c(json!({"op":"add_text","id":format!("p{i}"),"text":lorem,"style":"body","x":x,"y":y+50.0})));
        cmds.push(c(json!({"op":"add_shape","id":format!("s{i}"),"shape":"rectangle","x":x+300.0,"y":y+250.0})));
        let pts: Vec<[f64; 2]> = (0..200).map(|k| [x + k as f64, y + (k as f64 * 0.3).sin() * 40.0]).collect();
        cmds.push(c(json!({"op":"add_ink","points":pts})));
    }
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png(1200, 900));
    cmds.push(c(json!({"op":"add_image","id":"img","data_base64":b64,"x":-2000,"y":0})));
    drop(b64);
    ed.apply_batch(cmds, Some("build".into()), h.clone()).unwrap();
    // Start measuring with an empty history.
    let doc = ed.doc.clone();
    ed.load(doc, None);
    time_between(&ed);
    let original = ed.doc.clone();
    let base = live();
    println!("deck: {} elements, {} path steps, heap {:.1} MB", ed.doc.elements.len(), ed.doc.path.len(), base);

    let t = Instant::now();
    for k in 0..500usize {
        let i = (k * 37) % 400;
        let cmd = match k % 5 {
            0 => c(json!({"op":"move","ids":[format!("f{i}")],"dx":15,"dy":-5})),
            1 => c(json!({"op":"update","id":format!("p{i}"),"patch":{"text":format!("{lorem} edit {k}")}})),
            2 => c(json!({"op":"update","id":format!("s{i}"),"patch":{"fill":"#ff8800","rotation":k as f64}})),
            3 => c(json!({"op":"z_order","ids":[format!("t{i}")],"order":"front"})),
            _ => c(json!({"op":"move_step","from":i,"to":(i + 7) % 400})),
        };
        ed.apply(cmd, h.clone()).unwrap();
        ed.view.now += 3.0;
    }
    let per_edit = t.elapsed().as_secs_f64() * 1000.0 / 500.0;
    let after = live();
    println!("after 500 edits: heap {:.1} MB, history {:.1} MB, {:.3} ms/edit", after, after - base, per_edit);

    let t = Instant::now();
    let mut n = 0;
    while ed.undo(h.clone()).unwrap().is_some() {
        n += 1;
    }
    println!("undid {n} steps in {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    assert!(ed.doc == original, "undoing everything must restore the original deck exactly");
    let t = Instant::now();
    while ed.redo(h.clone()).unwrap().is_some() {}
    println!("redid them in {:.1} ms; round trip exact", t.elapsed().as_secs_f64() * 1000.0);
    media();
}

/// Heap held by media after opening a saved deck: copied into RAM (the old
/// loader) versus memory-mapped from the file (now).
fn media() {
    let dir = std::env::temp_dir().join(format!("zebra-media-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("media.zebra");
    let mut ed = Editor::default();
    let mut cmds = vec![];
    for i in 0..12u32 {
        // Noise doesn't compress, so each PNG is ~6 MB, like a photo.
        let mut img = image::RgbImage::new(1600, 1200);
        let mut x: u32 = 0x9e3779b9 ^ i;
        for p in img.pixels_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *p = image::Rgb([x as u8, (x >> 8) as u8, (x >> 16) as u8]);
        }
        let mut out = std::io::Cursor::new(vec![]);
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, out.into_inner());
        cmds.push(c(json!({"op":"add_image","data_base64":b64,"x":i as f64 * 2000.0})));
    }
    ed.apply_batch(cmds, None, Origin::Human).unwrap();
    zebra_core::io::save(&mut ed.doc, &path).unwrap();
    let size = std::fs::metadata(&path).unwrap().len() as f64 / 1_048_576.0;
    drop(ed);

    let base = live();
    let copied = zebra_core::io::load_bytes(&std::fs::read(&path).unwrap()).unwrap();
    let copied_mb = live() - base;
    drop(copied);
    let base = live();
    let mapped = zebra_core::io::load(&path).unwrap();
    let mapped_mb = live() - base;
    println!(
        "media deck ({size:.0} MB file, {} images): heap after open {copied_mb:.1} MB copied vs {mapped_mb:.1} MB mapped",
        mapped.assets.len()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[allow(dead_code)]
fn time_between(ed: &Editor) {
    let a = ed.doc.clone();
    let mut b = a.clone();
    b.title.push('x');
    let t = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(zebra_core::delta::Delta::between(&a, &b));
    }
    println!("Delta::between, no element change: {:.3} ms", t.elapsed().as_secs_f64() * 10.0);
    let t = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(a.clone());
    }
    println!("Document clone: {:.3} ms", t.elapsed().as_secs_f64() * 10.0);
}
