//! Video, audio, crop and replace through the command bus.
use base64::Engine;
use serde_json::json;
use zebra_core::media::fixtures::{tiny_mp4, tiny_wav};
use zebra_core::*;

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}
fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}
fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zebra-media-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    // Keep the cache out of the user's real cache directory.
    unsafe { std::env::set_var("ZEBRA_MEDIA_CACHE", d.join("cache")) };
    d
}

#[test]
fn video_from_path_is_mapped_and_survives_save() {
    let dir = tmp("video");
    let src = dir.join("clip.mp4");
    std::fs::write(&src, tiny_mp4(1280, 720, 9, true)).unwrap();
    let mut ed = Editor::default();
    let out = ed
        .apply(
            cmd(json!({"op":"add_video","path":src,"x":10,"y":20,"width":640,"alt":"Demo","poster_base64":b64(&png(16,9))})),
            Origin::Human,
        )
        .unwrap();
    let id = out.created[0].clone();
    assert!(id.starts_with('v'));
    let e = ed.doc.get(&id).unwrap();
    assert_eq!((e.size.w, e.size.h), (1280.0, 720.0));
    assert!((e.world_size().0 - 640.0).abs() < 1e-9);
    let ElementKind::Video(v) = e.kind.clone() else { panic!() };
    let v = &v;
    assert!(v.autoplay && !v.looping && !v.muted);
    assert!((v.duration - 9.0).abs() < 1e-9);
    assert!(v.poster.is_some());
    // The video is a mapping of the cache copy, never heap.
    assert!(ed.doc.assets[&v.asset].bytes.is_mapped());
    assert_eq!(ed.doc.assets[&v.asset].ext, "mp4");
    // The original can go away.
    std::fs::remove_file(&src).unwrap();

    let file = dir.join("deck.zebra");
    io::save(&mut ed.doc, &file).unwrap();
    let back = io::load(&file).unwrap();
    let ElementKind::Video(v2) = &back.get(&id).unwrap().kind else { panic!() };
    assert_eq!(v2, v);
    assert!(back.assets[&v.asset].bytes.is_mapped());
    assert_eq!(back.assets[&v.asset].width, 1280);
    assert!(back.assets.contains_key(v.poster.as_ref().unwrap()));
    assert!(zebra_core::describe::describe(&ed).contains("video alt=\"Demo\" 9s"));
}

#[test]
fn video_options_trim_and_errors() {
    let _ = tmp("opts");
    let mut ed = Editor::default();
    let id = ed.apply(cmd(json!({"op":"add_video","data_base64":b64(&tiny_mp4(320,240,10,true))})), Origin::Human).unwrap().created[0].clone();
    ed.apply(cmd(json!({"op":"update","id":id,"patch":{"loop":true,"muted":true,"start":2,"end":8,"autoplay":false}})), Origin::Human).unwrap();
    let ElementKind::Video(v) = &ed.doc.get(&id).unwrap().kind else { panic!() };
    assert!(v.looping && v.muted && !v.autoplay);
    assert_eq!((v.start, v.end), (2.0, Some(8.0)));
    // end before start is refused and changes nothing
    assert!(ed.apply(cmd(json!({"op":"update","id":id,"patch":{"end":1}})), Origin::Human).is_err());
    assert!(ed.apply(cmd(json!({"op":"update","id":id,"patch":{"start":12, "end": -1}})), Origin::Human).is_err());
    ed.apply(cmd(json!({"op":"update","id":id,"patch":{"end":-1}})), Origin::Human).unwrap();
    let ElementKind::Video(v) = &ed.doc.get(&id).unwrap().kind else { panic!() };
    assert_eq!(v.end, None);
    // wrong kinds
    assert!(ed.apply(cmd(json!({"op":"add_video","data_base64":b64(&png(4,4))})), Origin::Human).is_err());
    assert!(ed.apply(cmd(json!({"op":"add_image","data_base64":b64(&tiny_mp4(4,4,1,true))})), Origin::Human).is_err());
    assert!(ed.apply(cmd(json!({"op":"update","id":id,"patch":{"crop":{"x":0,"y":0,"w":0.5,"h":0.5}}})), Origin::Human).is_err());
    // poster set and cleared
    ed.apply(cmd(json!({"op":"set_poster","id":id,"data_base64":b64(&png(8,6))})), Origin::Human).unwrap();
    ed.apply(cmd(json!({"op":"set_poster","id":id})), Origin::Human).unwrap();
    let ElementKind::Video(v) = &ed.doc.get(&id).unwrap().kind else { panic!() };
    assert!(v.poster.is_none());
}

#[test]
fn crop_keeps_visible_content_in_place_and_undoes() {
    let mut ed = Editor::default();
    let id = ed
        .apply(cmd(json!({"op":"add_image","data_base64":b64(&png(400,200)),"x":100,"y":50,"width":800})), Origin::Human)
        .unwrap()
        .created[0]
        .clone();
    ed.apply(cmd(json!({"op":"rotate","ids":[id],"degrees":30})), Origin::Human).unwrap();
    let before = ed.doc.clone();
    // World position of source pixel (300, 150) before the crop.
    let world_of = |ed: &Editor, u: f64, v: f64| {
        let e = ed.doc.get(&id).unwrap();
        let ElementKind::Image(i) = &e.kind else { panic!() };
        let c = i.crop.unwrap_or(Crop::FULL);
        let (pw, ph) = (400.0, 200.0);
        let kx = e.size.w / (pw * c.w);
        let ky = e.size.h / (ph * c.h);
        let lx = (u - (c.x + c.w / 2.0) * pw) * kx;
        let ly = (v - (c.y + c.h / 2.0) * ph) * ky;
        e.transform.apply(Vec2::new(lx, ly))
    };
    let p0 = world_of(&ed, 300.0, 150.0);
    ed.apply(cmd(json!({"op":"update","id":id,"patch":{"crop":{"x":0.5,"y":0.25,"w":0.5,"h":0.75}}})), Origin::Human).unwrap();
    let e = ed.doc.get(&id).unwrap();
    assert_eq!((e.size.w, e.size.h), (200.0, 150.0));
    let p1 = world_of(&ed, 300.0, 150.0);
    assert!((p0 - p1).len() < 1e-6, "{p0:?} vs {p1:?}");
    assert_eq!(ed.undo_label().as_deref(), Some("Crop (You)"));
    ed.apply(cmd(json!({"op":"update","id":id,"patch":{"mask":"circle"}})), Origin::Human).unwrap();
    // Uncrop restores the full box exactly.
    ed.apply(cmd(json!({"op":"update","id":id,"patch":{"crop":{"x":0,"y":0,"w":1,"h":1}}})), Origin::Human).unwrap();
    let e = ed.doc.get(&id).unwrap();
    let b = before.get(&id).unwrap();
    assert!((e.transform.x - b.transform.x).abs() < 1e-9 && (e.transform.y - b.transform.y).abs() < 1e-9);
    assert_eq!(e.size, b.size);
    ed.undo(Origin::Human).unwrap();
    ed.undo(Origin::Human).unwrap();
    ed.undo(Origin::Human).unwrap();
    assert_eq!(ed.doc, before);
}

#[test]
fn replace_image_keeps_width() {
    let mut ed = Editor::default();
    let id = ed.apply(cmd(json!({"op":"add_image","data_base64":b64(&png(400,200)),"width":800})), Origin::Human).unwrap().created[0].clone();
    ed.apply(cmd(json!({"op":"update","id":id,"patch":{"crop":{"x":0.1,"y":0.1,"w":0.5,"h":0.5},"mask":"rounded"}})), Origin::Human).unwrap();
    let w0 = ed.doc.get(&id).unwrap().world_size().0;
    ed.apply(cmd(json!({"op":"replace_image","id":id,"data_base64":b64(&png(100,100))})), Origin::Human).unwrap();
    let e = ed.doc.get(&id).unwrap();
    let (w, h) = e.world_size();
    assert!((w - w0).abs() < 1e-9 && (h - w0).abs() < 1e-9);
    let ElementKind::Image(i) = &e.kind else { panic!() };
    assert_eq!((i.crop, i.mask), (None, ImageMask::Rounded));
    assert!(ed.apply(cmd(json!({"op":"replace_image","id":id,"data_base64":b64(&tiny_wav(0.1))})), Origin::Human).is_err());
}

#[test]
fn soundtrack_and_voiceover_undo_and_save() {
    let dir = tmp("audio");
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"add_frame","add_to_path":true})), Origin::Human).unwrap();
    assert!(ed.apply(cmd(json!({"op":"set_soundtrack","volume":0.2})), Origin::Human).is_err());
    let song = dir.join("Song.wav");
    std::fs::write(&song, tiny_wav(1.0)).unwrap();
    ed.apply(cmd(json!({"op":"set_soundtrack","path":song})), Origin::Human).unwrap();
    let t = ed.doc.soundtrack.clone().unwrap();
    assert_eq!((t.name.as_str(), t.looping, t.volume), ("Song", true, 0.6));
    ed.apply(cmd(json!({"op":"set_soundtrack","volume":3,"loop":false})), Origin::Human).unwrap();
    assert_eq!(ed.doc.soundtrack.as_ref().unwrap().volume, 1.0);
    ed.apply(cmd(json!({"op":"set_voiceover","index":0,"data_base64":b64(&tiny_wav(0.3))})), Origin::Human).unwrap();
    assert!(ed.doc.path[0].voiceover.is_some());
    assert!(ed.apply(cmd(json!({"op":"set_voiceover","index":0,"data_base64":b64(&png(2,2))})), Origin::Human).is_err());
    assert!(describe::describe(&ed).contains("Soundtrack: \"Song\""));

    let file = dir.join("a.zebra");
    io::save(&mut ed.doc, &file).unwrap();
    let back = io::load(&file).unwrap();
    assert_eq!(back.soundtrack, ed.doc.soundtrack);
    assert_eq!(back.path[0].voiceover, ed.doc.path[0].voiceover);
    assert_eq!(back.used_assets().len(), 2);

    ed.apply(cmd(json!({"op":"remove_soundtrack"})), Origin::Human).unwrap();
    assert!(ed.doc.soundtrack.is_none());
    ed.undo(Origin::Human).unwrap();
    assert_eq!(ed.doc.soundtrack.as_ref().unwrap().volume, 1.0);
    ed.undo(Origin::Human).unwrap(); // voice-over
    assert!(ed.doc.path[0].voiceover.is_none());
    ed.undo(Origin::Human).unwrap(); // volume
    ed.undo(Origin::Human).unwrap(); // add
    assert!(ed.doc.soundtrack.is_none());
    ed.redo(Origin::Human).unwrap();
    assert_eq!(ed.doc.soundtrack.as_ref().unwrap().name, "Song");
}

#[test]
fn old_files_without_media_fields_load() {
    let json = r#"{"format_version":1,"title":"x","theme":"classic","elements":{},"z_order":[],"path":[{"target":{"kind":"overview"}}],"next_id":1}"#;
    let d = io::from_json(json).unwrap();
    assert!(d.soundtrack.is_none() && d.path[0].voiceover.is_none());
}

#[test]
fn replacing_a_video_keeps_its_place_width_and_settings() {
    let _ = tmp("replace");
    let mut ed = Editor::default();
    let id = ed.apply(cmd(json!({"op":"add_video","data_base64":b64(&tiny_mp4(320,240,10,true)),"x":100,"y":50,"width":640})), Origin::Human).unwrap().created[0].clone();
    ed.apply(cmd(json!({"op":"update","id":id,"patch":{"loop":true,"muted":true,"start":2,"end":8,"autoplay":false,"rotation":15}})), Origin::Human).unwrap();
    ed.apply(cmd(json!({"op":"replace_video","id":id,"data_base64":b64(&tiny_mp4(1280,720,4,true))})), Origin::Human).unwrap();
    let e = ed.doc.get(&id).unwrap();
    let ElementKind::Video(v) = &e.kind else { panic!() };
    assert!(v.looping && v.muted && !v.autoplay, "settings kept");
    assert_eq!((v.start, v.end), (0.0, None), "the trim starts over");
    assert!((v.duration - 4.0).abs() < 0.1, "{}", v.duration);
    assert_eq!((e.transform.x, e.transform.y, e.transform.rotation), (100.0, 50.0, 15.0));
    let (w, h) = e.world_size();
    assert!((w - 640.0).abs() < 1e-6 && (h - 360.0).abs() < 1e-6, "same width, height from the new clip: {w}x{h}");
    // Only videos.
    let img = ed.apply(cmd(json!({"op":"add_image","data_base64":b64(&png(4,4))})), Origin::Human).unwrap().created[0].clone();
    assert!(ed.apply(cmd(json!({"op":"replace_video","id":img,"data_base64":b64(&tiny_mp4(320,240,1,true))})), Origin::Human).is_err());
}

/// A saved video whose length came from a lying header (20 Nov audit, W10)
/// is repaired to "unknown" when the document is opened.
#[test]
fn an_absurd_video_length_is_repaired_on_open() {
    let mut ed = Editor::default();
    let id = ed.apply(cmd(json!({"op":"add_video","data_base64":b64(&tiny_mp4(320,240,10,true))})), Origin::Human).unwrap().created[0].clone();
    for (said, kept) in [(4_294_967_295.0, 0.0), (86_401.0, 0.0), (-3.0, 0.0), (7200.0, 7200.0)] {
        let mut doc = ed.doc.clone();
        if let Some(ElementKind::Video(v)) = doc.elements.get_mut(&id).map(|e| &mut e.kind) {
            v.duration = said;
        }
        zebra_core::check::repair(&mut doc);
        let ElementKind::Video(v) = &doc.get(&id).unwrap().kind else { panic!() };
        assert_eq!(v.duration, kept, "{said}");
    }
}
