//! Fixture for the browser check (`tests/browser/check.js`): the app's
//! sample deck (overview, fade-in builds, a rotated frame) plus a picture
//! and a video step, exported as a single page and as a folder, with a
//! `Renderer::capture` reference PNG for every step and build the player
//! goes through.
//!
//!   cargo run -p zebra-web --example browser_fixture -- OUT_DIR [video.mp4]
use std::path::{Path, PathBuf};

use base64::Engine;
use zebra_core::model::StepTarget;
use zebra_core::{Command, Editor, Origin};
use zebra_render::{CaptureTarget, Renderer};

const H: Origin = Origin::Human;
pub const W: u32 = 1280;
pub const HT: u32 = 800;

/// English words for the sample's `sample-*` keys, from the app's .ftl.
fn words() -> std::collections::HashMap<String, String> {
    include_str!("../../zebra-app/i18n/en-GB/zebra-app.ftl")
        .lines()
        .filter_map(|l| l.split_once(" = "))
        .filter(|(k, _)| k.starts_with("sample-"))
        .map(|(k, v)| (k.trim().to_string(), v.to_string()))
        .collect()
}

fn fill(v: &mut serde_json::Value, w: &std::collections::HashMap<String, String>) {
    match v {
        serde_json::Value::String(s) => {
            if let Some(t) = w.get(s.as_str()) {
                *s = t.clone();
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(|x| fill(x, w)),
        serde_json::Value::Object(o) => o.values_mut().for_each(|x| fill(x, w)),
        _ => {}
    }
}

pub fn deck(r: &Renderer, video: Option<&Path>) -> Editor {
    let mut v: serde_json::Value = serde_json::from_str(include_str!("../../zebra-app/data/sample.json")).unwrap();
    fill(&mut v, &words());
    let cmds: Vec<Command> = serde_json::from_value(v["commands"].clone()).unwrap();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    for c in cmds {
        ed.apply(c, H).unwrap();
    }
    let b = ed.doc.bounds().unwrap();
    // A picture: a colour ramp.
    let img = image::RgbImage::from_fn(320, 200, |x, y| image::Rgb([(x * 255 / 320) as u8, (y * 255 / 200) as u8, 160]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let b64 = |d: &[u8]| base64::engine::general_purpose::STANDARD.encode(d);
    let (x, y) = (b.x1 + 600.0, (b.y0 + b.y1) / 2.0);
    ed.apply(Command::AddImage { data_base64: b64(&png), x, y: y - 300.0, width: Some(480.0), alt: "ramp".into(), id: Some("ramp".into()) }, H).unwrap();
    ed.apply(Command::AddStep { target: StepTarget::Element { id: "ramp".into() }, index: None, builds: vec![], notes: String::new() }, H).unwrap();
    if let Some(p) = video {
        let data = std::fs::read(p).unwrap();
        ed.apply(
            Command::AddVideo { data_base64: b64(&data), path: None, x, y: y + 300.0, width: Some(480.0), alt: "clip".into(), poster_base64: None, autoplay: true, looping: true, muted: true, id: Some("clip".into()) },
            H,
        )
        .unwrap();
        ed.apply(Command::AddStep { target: StepTarget::Element { id: "clip".into() }, index: None, builds: vec![], notes: String::new() }, H).unwrap();
    }
    ed.doc.title = "Sample".into();
    ed
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(&args[0]);
    let video = args.get(1).map(PathBuf::from);
    let r = Renderer::new();
    let mut ed = deck(&r, video.as_deref());
    let _ = ed.view(zebra_core::ViewCommand::SetViewport { width: W as f64, height: HT as f64 }, H);
    std::fs::create_dir_all(out.join("ref")).unwrap();
    std::fs::create_dir_all(out.join("folder")).unwrap();
    let labels = Default::default();
    let single = zebra_web::export::export(&r, &ed, &labels, &out.join("sample.html")).unwrap();
    let folder = zebra_web::export::export_with_limit(&r, &ed, &labels, &out.join("folder/sample.html"), 1).unwrap();
    // The positions Next goes through, from the first step: (step, build).
    let mut seq = vec![];
    for (i, s) in ed.doc.path.iter().enumerate() {
        for b in 0..=s.builds.len() {
            let png = r.capture(&ed, &CaptureTarget::Step { index: i, build: Some(b) }, W, HT).unwrap();
            let name = format!("s{i}b{b}.png");
            std::fs::write(out.join("ref").join(&name), png).unwrap();
            seq.push(serde_json::json!({"step": i, "build": b, "ref": name, "overview": matches!(s.target, StepTarget::Overview)}));
        }
    }
    let info = serde_json::json!({
        "single": single.page, "single_folder": single.folder, "single_bytes": single.bytes,
        "folder": folder.page, "folder_folder": folder.folder,
        "width": W, "height": HT, "sequence": seq, "video": video.is_some(),
    });
    std::fs::write(out.join("fixture.json"), serde_json::to_string_pretty(&info).unwrap()).unwrap();
    println!("{}", info);
}
