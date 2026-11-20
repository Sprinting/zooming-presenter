//! Web videos (spec 18e): a link and a poster in the deck, never the
//! video; YouTube and Vimeo links become cards; Download keeps the clip.
use base64::Engine;
use serde_json::json;
use zebra_core::media::fixtures::tiny_mp4;
use zebra_core::*;

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}
fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([20, 120, 200, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}
/// A PNG whose header claims `w` x `h` over one pixel of data (with the
/// header's checksum fixed, so only its size is a lie).
fn lying_png(w: u32, h: u32) -> Vec<u8> {
    let mut b = png(1, 1);
    b[16..20].copy_from_slice(&w.to_be_bytes());
    b[20..24].copy_from_slice(&h.to_be_bytes());
    let mut crc = !0u32;
    for &x in &b[12..29] {
        crc ^= x as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    b[29..33].copy_from_slice(&(!crc).to_be_bytes());
    b
}
fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("zebra-webvideo-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    unsafe { std::env::set_var("ZEBRA_MEDIA_CACHE", d.join("cache")) };
    d
}
fn video(ed: &Editor, id: &str) -> VideoData {
    match &ed.doc.get(id).unwrap().kind {
        ElementKind::Video(v) => v.clone(),
        k => panic!("{k:?}"),
    }
}

/// The deck keeps the link and the poster: saving writes no video, and
/// the file reads back the same.
#[test]
fn a_web_video_keeps_only_its_link_and_poster() {
    let dir = tmp("keep");
    let mut ed = Editor::default();
    let out = ed
        .apply(
            cmd(json!({"op":"add_web_video","url":"https://cdn.example.com/talk.mp4","x":5,"y":6,"width":640,
                       "poster_base64":b64(&png(32,18)),"found":{"kind":"file","width":1920,"height":1080,"duration":75.5}})),
            Origin::Human,
        )
        .unwrap();
    let id = out.created[0].clone();
    assert!(id.starts_with('v'));
    assert_eq!(ed.history().last().unwrap().label, "Insert video", "one undo step");
    let e = ed.doc.get(&id).unwrap();
    assert_eq!((e.size.w, e.size.h), (1920.0, 1080.0), "its own shape");
    assert!((e.world_size().0 - 640.0).abs() < 1e-9);
    let v = video(&ed, &id);
    assert_eq!(v.link, Some(VideoLink { url: "https://cdn.example.com/talk.mp4".into(), kind: LinkKind::File }));
    assert!(v.asset.is_empty());
    assert!(v.poster.is_some() && v.autoplay && !v.is_card());
    assert_eq!(v.duration, 75.5);
    // Only the poster is used.
    assert_eq!(ed.doc.used_assets().len(), 1);

    let file = dir.join("deck.zebra");
    io::save(&mut ed.doc, &file).unwrap();
    let back = io::load(&file).unwrap();
    let ElementKind::Video(v2) = &back.get(&id).unwrap().kind else { panic!() };
    assert_eq!(*v2, v);
    assert_eq!(back.assets.len(), 1, "no video in the file");
    let json = io::to_json(&ed.doc);
    let saved: serde_json::Value = serde_json::from_str(&json).unwrap();
    let kind = &saved["elements"][id.as_str()]["kind"];
    assert_eq!(kind["link"], json!({"url":"https://cdn.example.com/talk.mp4","kind":"file"}), "{kind}");
    assert!(kind.get("asset").is_none(), "an empty asset key isn't written");
    let d = describe::describe(&ed);
    assert!(d.contains("web file link=\"https://cdn.example.com/talk.mp4\""), "{d}");

    // Undo takes it away in one step.
    ed.undo(Origin::Human).unwrap();
    assert!(ed.doc.get(&id).is_none());
}

#[test]
fn links_are_checked_as_text() {
    let mut ed = Editor::default();
    let err = |ed: &mut Editor, url: &str| ed.apply(cmd(json!({"op":"add_web_video","url":url})), Origin::Human).unwrap_err();
    let e = err(&mut ed, "http://cdn.example.com/talk.mp4");
    assert_eq!((e.code, e.message.as_str()), ("not_https", "Use an https link."));
    assert_eq!(err(&mut ed, "talk.mp4").code, "not_a_link");
    assert_eq!(err(&mut ed, "https://me:secret@example.com/a.mp4").code, "password_in_link");
    assert!(ed.doc.elements.is_empty());
    // Without a check (core alone): the kind from the link, 16:9.
    let id = ed.apply(cmd(json!({"op":"add_web_video","url":"https://example.com/live/index.m3u8"})), Origin::Human).unwrap().created[0].clone();
    assert_eq!(video(&ed, &id).link.unwrap().kind, LinkKind::Stream);
    assert_eq!((ed.doc.get(&id).unwrap().size.w, ed.doc.get(&id).unwrap().size.h), (640.0, 360.0));
}

/// YouTube and Vimeo links are cards whatever the check says; a card
/// never plays by itself, keeps a start and an end, and takes its start
/// from the link.
#[test]
fn youtube_and_vimeo_links_are_cards() {
    let _ = tmp("cards");
    let mut ed = Editor::default();
    let id = ed
        .apply(cmd(json!({"op":"add_web_video","url":"https://youtu.be/dQw4w9WgXcQ?t=42","autoplay":true,"loop":true,
                           "poster_base64":b64(&png(64,36)),"found":{"kind":"file"}})), Origin::Human)
        .unwrap()
        .created[0]
        .clone();
    let v = video(&ed, &id);
    assert_eq!(v.link.as_ref().unwrap().kind, LinkKind::Youtube);
    assert!(v.is_card() && !v.autoplay && !v.looping);
    assert_eq!(v.start, 42.0, "the link's own t=");
    let e = ed.doc.get(&id).unwrap();
    assert_eq!((e.size.w, e.size.h), (64.0, 36.0), "the poster's shape");
    // Start and End are kept in the deck; auto-play, loop and mute don't apply.
    ed.apply(cmd(json!({"op":"update","id":id,"patch":{"start":10,"end":95}})), Origin::Human).unwrap();
    assert_eq!((video(&ed, &id).start, video(&ed, &id).end), (10.0, Some(95.0)));
    let e = ed.apply(cmd(json!({"op":"update","id":id,"patch":{"loop":true}})), Origin::Human).unwrap_err();
    assert!(e.message.contains("video card"), "{}", e.message);
    // Vimeo.
    let vm = ed.apply(cmd(json!({"op":"add_web_video","url":"https://vimeo.com/76979871","start":3})), Origin::Human).unwrap().created[0].clone();
    assert_eq!(video(&ed, &vm).link.unwrap().kind, LinkKind::Vimeo);
    assert_eq!(video(&ed, &vm).start, 3.0);
    assert!(describe::describe(&ed).contains("youtube card link="));
    // A card can't be downloaded.
    let e = ed.apply(cmd(json!({"op":"download_video","id":vm})), Origin::Human).unwrap_err();
    assert!(e.message.contains("only a link to a video file"), "{}", e.message);
}

#[test]
fn live_streams_have_no_start_end_or_loop() {
    let mut ed = Editor::default();
    let id = ed
        .apply(cmd(json!({"op":"add_web_video","url":"https://example.com/live.m3u8","start":5,"end":9,"found":{"kind":"live","width":1280,"height":720}})), Origin::Human)
        .unwrap()
        .created[0]
        .clone();
    let v = video(&ed, &id);
    assert!(v.is_live());
    assert_eq!((v.start, v.end, v.duration), (0.0, None, 0.0));
    for patch in [json!({"start":1}), json!({"end":3}), json!({"loop":true})] {
        assert!(ed.apply(cmd(json!({"op":"update","id":id,"patch":patch})), Origin::Human).is_err());
    }
    ed.apply(cmd(json!({"op":"update","id":id,"patch":{"muted":true}})), Origin::Human).unwrap();
}

/// Download turns a web video into an ordinary one in place: same size,
/// poster, trim and settings, the clip in the deck, no link.
#[test]
fn download_keeps_the_clip_in_the_deck() {
    let dir = tmp("download");
    let mut ed = Editor::default();
    let id = ed
        .apply(cmd(json!({"op":"add_web_video","url":"https://cdn.example.com/talk.mp4","width":500,"muted":true,"start":1,"end":4,
                           "poster_base64":b64(&png(32,18)),"found":{"kind":"file","width":320,"height":180}})), Origin::Human)
        .unwrap()
        .created[0]
        .clone();
    let before = ed.doc.get(&id).unwrap().clone();
    // Without a file, the core can't fetch it.
    assert_eq!(ed.apply(cmd(json!({"op":"download_video","id":id})), Origin::Human).unwrap_err().code, "unsupported");
    let clip = dir.join("talk.mp4");
    std::fs::write(&clip, tiny_mp4(320, 180, 6, true)).unwrap();
    ed.apply(cmd(json!({"op":"download_video","id":id,"path":clip})), Origin::Human).unwrap();
    assert_eq!(ed.history().last().unwrap().label, "Download video");
    let after = ed.doc.get(&id).unwrap();
    assert_eq!((after.transform, after.size), (before.transform, before.size));
    let v = video(&ed, &id);
    let ElementKind::Video(old) = &before.kind else { panic!() };
    assert!(v.link.is_none() && !v.asset.is_empty());
    assert_eq!(ed.doc.assets[&v.asset].ext, "mp4");
    assert_eq!((v.poster.clone(), v.start, v.end, v.muted), (old.poster.clone(), 1.0, Some(4.0), true));
    assert_eq!(v.duration, 6.0);
    // Already in the deck.
    assert!(ed.apply(cmd(json!({"op":"download_video","id":id,"path":clip})), Origin::Human).is_err());
    // Undo brings the link back.
    ed.undo(Origin::Human).unwrap();
    assert!(video(&ed, &id).link.is_some());
}

/// A hand-edited or damaged file can't hold a video with both a clip and
/// a link, or a card on the wrong site.
#[test]
fn inconsistent_web_videos_are_refused() {
    let mut ed = Editor::default();
    let bad = json!({"op":"add_element","element":{"x":0,"y":0,"w":160,"h":90,"kind":{"type":"video","link":{"url":"https://example.com/a.mp4","kind":"youtube"}}}});
    assert!(ed.apply(cmd(bad), Origin::Human).is_err());
    let http = json!({"op":"add_element","element":{"x":0,"y":0,"w":160,"h":90,"kind":{"type":"video","link":{"url":"http://example.com/a.mp4","kind":"file"}}}});
    assert_eq!(ed.apply(cmd(http), Origin::Human).unwrap_err().code, "not_https");
    let ok = json!({"op":"add_element","element":{"x":0,"y":0,"w":160,"h":90,"kind":{"type":"video","link":{"url":"https://vimeo.com/1234","kind":"vimeo"}}}});
    ed.apply(cmd(ok), Origin::Human).unwrap();
}

/// Copy and paste between decks carries the poster, not a missing clip.
#[test]
fn copying_a_web_video_carries_its_poster() {
    let _ = tmp("copy");
    let mut a = Editor::default();
    let id = a.apply(cmd(json!({"op":"add_web_video","url":"https://youtu.be/dQw4w9WgXcQ","poster_base64":b64(&png(16,9))})), Origin::Human).unwrap().created[0].clone();
    let clip = clip::Clip::of(&a, std::slice::from_ref(&id));
    let mut b = Editor::default();
    let out = b.paste_clip(&clip, 0.0, 0.0, None, Origin::Human).unwrap();
    let v = video(&b, &out.created[0]);
    assert!(b.doc.assets.contains_key(v.poster.as_ref().unwrap()));
    assert!(v.is_card());
}

/// A deck saves as the lowest format that can read it: 1 without web
/// videos (older builds still open it), 2 with one. A format 1 build
/// refuses a format 2 file as newer, never dropping the web video.
#[test]
fn the_format_version_says_web_videos() {
    assert!(FORMAT_VERSION >= 2);
    let version = |ed: &Editor| serde_json::from_str::<serde_json::Value>(&io::to_json(&ed.doc)).unwrap()["format_version"].as_u64().unwrap();
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"add_text","text":"Hello"})), Origin::Human).unwrap();
    assert_eq!(version(&ed), 1);
    assert_eq!(io::required_version(&ed.doc), 1);
    let id = ed.apply(cmd(json!({"op":"add_web_video","url":"https://vimeo.com/76979871"})), Origin::Human).unwrap().created[0].clone();
    assert_eq!(version(&ed), 2);
    let json = io::to_json(&ed.doc);
    // This build reads it back.
    assert!(io::from_json(&json).unwrap().get(&id).is_some());
    // An old reader: the same check with FORMAT_VERSION 1.
    let ver = serde_json::from_str::<serde_json::Value>(&json).unwrap()["format_version"].as_u64().unwrap() as u32;
    assert!(ver > 1, "a format 1 build refuses it");
    let mut newer: serde_json::Value = serde_json::from_str(&json).unwrap();
    newer["format_version"] = json!(FORMAT_VERSION + 1);
    let e = io::from_json(&newer.to_string()).unwrap_err();
    assert_eq!(e.code, "bad_file");
    assert!(e.message.contains("newer"), "{}", e.message);
    // Deleting the web video makes it a format 1 deck again.
    ed.apply(cmd(json!({"op":"delete","ids":[id]})), Origin::Human).unwrap();
    assert_eq!(version(&ed), 1);
    // And a saved .zebra carries it too.
    let dir = tmp("version");
    ed.apply(cmd(json!({"op":"add_web_video","url":"https://youtu.be/dQw4w9WgXcQ"})), Origin::Human).unwrap();
    let file = dir.join("v.zebra");
    io::save(&mut ed.doc, &file).unwrap();
    let mut z = zip::ZipArchive::new(std::fs::File::open(&file).unwrap()).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name("document.json").unwrap(), &mut s).unwrap();
    assert!(s.starts_with("{\n  \"format_version\": 2"), "{}", &s[..40]);
}

/// Hostile input over the API: a password in the link, a poster too big
/// to keep (bytes or pixels), sizes no screen has, NaN times. Each is
/// refused or ignored, and the deck is as it was.
#[test]
fn hostile_web_videos_are_refused() {
    let mut ed = Editor::default();
    let add = |ed: &mut Editor, extra: serde_json::Value| {
        let mut j = json!({"op":"add_web_video","url":"https://cdn.example.com/talk.mp4","x":0,"y":0});
        for (k, v) in extra.as_object().unwrap() {
            j[k] = v.clone();
        }
        ed.apply(cmd(j), Origin::Human)
    };
    let e = add(&mut ed, json!({"url":"https://me:secret@cdn.example.com/talk.mp4"})).unwrap_err();
    assert_eq!((e.code, e.message.as_str()), ("password_in_link", "Links with a password in them can't be used."));

    // Over 8 MB of poster: refused before it is decoded.
    let big = "A".repeat(12 * 1024 * 1024);
    assert_eq!(add(&mut ed, json!({"poster_base64": big})).unwrap_err().code, "too_large");
    // A small file that claims to be 60000 pixels a side.
    assert_eq!(add(&mut ed, json!({"poster_base64": b64(&lying_png(60000, 60000))})).unwrap_err().code, "too_large");
    assert!(add(&mut ed, json!({"poster_base64": "not base64!"})).is_err());
    assert!(add(&mut ed, json!({"poster_base64": b64(b"")})).is_ok(), "an empty poster is no poster");
    ed.undo(Origin::Human).unwrap();
    assert!(ed.doc.elements.is_empty() && ed.doc.used_assets().is_empty(), "nothing left behind");

    // A size the link claims past any screen is not believed.
    let out = add(&mut ed, json!({"found":{"kind":"file","width":4000000000u32,"height":3,"duration":5}})).unwrap();
    let el = ed.doc.get(&out.created[0]).unwrap();
    assert_eq!((el.size.w, el.size.h), (640.0, 360.0));
    assert!(add(&mut ed, json!({"start": -1})).is_err());
    assert!(add(&mut ed, json!({"start": 5, "end": 2})).is_err());
    // Megabytes of link text.
    let long = format!("https://cdn.example.com/{}.mp4", "a".repeat(2 * 1024 * 1024));
    let _ = add(&mut ed, json!({"url": long}));
    assert!(add(&mut ed, json!({"url": ""})).is_err());
}
