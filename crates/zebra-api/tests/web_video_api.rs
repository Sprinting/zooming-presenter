//! Web videos through the API (`add_web_video`, `download_video`), with
//! the same checks and messages as the Insert Web Video sheet. A local
//! server stands in for the web (`ZEBRA_TEST_WEB`), so nothing here
//! touches the internet. Its own test binary: it sets the environment.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission};
use zebra_media::link::STAND_IN;

fn rpc_err(r: Result<Value, ClientError>) -> zebra_api::RpcError {
    match r {
        Err(ClientError::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 40, 40]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

type Log = Arc<Mutex<Vec<String>>>;

fn respond(mut s: TcpStream, log: &Log) {
    let mut buf = vec![];
    let mut chunk = [0u8; 4096];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let req = String::from_utf8_lossy(&buf).to_string();
    let target = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    log.lock().unwrap().push(target.clone());
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let mp4 = zebra_core::media::fixtures::tiny_mp4(1280, 720, 30, true);
    let (status, ctype, body): (&str, &str, Vec<u8>) = match path {
        "/talk.mp4" => ("200 OK", "video/mp4", mp4),
        "/page.html" => ("200 OK", "text/html", b"<!doctype html><p>hi".to_vec()),
        "/private.mp4" => ("403 Forbidden", "text/plain", vec![]),
        "/live.m3u8" => ("200 OK", "application/x-mpegURL", b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nx.ts\n".to_vec()),
        "/oembed/youtube" => (
            "200 OK",
            "application/json",
            format!(r#"{{"type":"video","title":"Launch talk","width":200,"height":113,"thumbnail_url":"{STAND_IN}/thumb.png"}}"#).into_bytes(),
        ),
        "/thumb.png" => ("200 OK", "image/png", png(64, 36)),
        _ => {
            let _ = query;
            ("404 Not Found", "text/plain", vec![])
        }
    };
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(&body);
}

fn server() -> (String, Log) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let log: Log = Default::default();
    let (b2, l2) = (base.clone(), log.clone());
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (b, lg) = (b2.clone(), l2.clone());
            let _ = b;
            std::thread::spawn(move || respond(s, &lg));
        }
    });
    (base, log)
}

fn element(c: &mut Client, id: &str) -> Value {
    c.call("query", json!({"query":{"q":"element","id":id}})).unwrap()
}

#[test]
fn web_videos_over_the_api() {
    let (base, log) = server();
    // The links are https on a made-up host the local server answers for.
    let web = STAND_IN;
    let dir = std::env::temp_dir().join(format!("zebra-web-video-api-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: the only test in this binary, before any thread starts
    // (the server thread reads no environment).
    unsafe {
        std::env::set_var("ZEBRA_DATA_DIR", dir.join("data"));
        std::env::set_var("ZEBRA_MEDIA_CACHE", dir.join("cache"));
        std::env::set_var("ZEBRA_FAKE_MEDIA", "1");
        std::env::set_var("ZEBRA_TEST_WEB", &base);
        std::env::set_var("ZEBRA_TEST_WEB_TIMEOUT_MS", "2000");
    }
    let h = HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Full }, None).unwrap();
    let mut c = Client::connect_hello(&h.socket, "web", &h.token).unwrap();

    // A file link: its own shape, a poster from the clip, one undo step.
    let out = c.call("apply", json!({"command":{"op":"add_web_video","url":format!("{web}/talk.mp4"),"width":640}})).unwrap();
    let id = out["created"][0].as_str().unwrap().to_string();
    let el = element(&mut c, &id);
    let s = el.to_string();
    assert!(s.contains("https://zebra-test.invalid/talk.mp4") && s.contains("\"file\""), "{el}");
    assert!(s.contains("poster"), "a poster from the clip: {el}");
    assert!(s.contains("1280") && s.contains("720"), "its own shape: {el}");

    // Each refusal has the sheet's message.
    let cases = [
        ("https://me:pw@example.com/v.mp4".to_string(), "password_in_link", "Links with a password in them can't be used."),
        (format!("{web}/page.html"), "not_a_video", "This link isn't a video."),
        (format!("{web}/private.mp4"), "sign_in", "This video needs a sign-in."),
        (format!("{web}/gone.mp4"), "not_found", "Nothing found at this link."),
        (format!("{base}/talk.mp4"), "not_https", "Use an https link."),
        ("https://127.0.0.1:1/v.mp4".to_string(), "unreachable", "Couldn't reach this link. Check your connection."),
        ("talk.mp4".to_string(), "not_a_link", "This isn't a link. Give an https link to a video."),
    ];
    for (url, code, message) in cases {
        let e = rpc_err(c.call("apply", json!({"command":{"op":"add_web_video","url":url}})));
        assert_eq!((e.kind(), e.to_string().contains(message)), (code, true), "{url}: {e}");
    }

    // A live stream: no start, end or loop.
    let out = c.call("apply", json!({"command":{"op":"add_web_video","url":format!("{web}/live.m3u8")}})).unwrap();
    let live = out["created"][0].as_str().unwrap().to_string();
    assert!(element(&mut c, &live).to_string().contains("\"live\""));
    let e = rpc_err(c.call("apply", json!({"command":{"op":"add_web_video","url":format!("{web}/live.m3u8"),"loop":true}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");

    // YouTube: a card with the site's thumbnail and title; the lookup is
    // the local stand-in, never the internet.
    let out = c.call("apply", json!({"command":{"op":"add_web_video","url":"https://youtu.be/dQw4w9WgXcQ?t=1m5s"}})).unwrap();
    let card = out["created"][0].as_str().unwrap().to_string();
    let s = element(&mut c, &card).to_string();
    assert!(s.contains("youtube") && s.contains("Launch talk") && s.contains("poster"), "{s}");
    assert!(log.lock().unwrap().iter().any(|p| p.starts_with("/oembed/youtube?format=json&url=")));

    // A poster over 8 MB is refused before it is read.
    let big = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 9 << 20]);
    let e = rpc_err(c.call("apply", json!({"command":{"op":"add_web_video","url":format!("{web}/talk.mp4"),"poster_base64":big}})));
    assert_eq!(e.kind(), "too_large", "{e}");

    // Download keeps the clip in the deck: an ordinary video now.
    c.call("apply", json!({"command":{"op":"download_video","id":id}})).unwrap();
    let s = element(&mut c, &id).to_string();
    assert!(!s.contains("/talk.mp4"), "the link is gone: {s}");
    let e = rpc_err(c.call("apply", json!({"command":{"op":"download_video","id":card}})));
    assert_eq!(e.kind(), "invalid_params", "a card can't be downloaded: {e}");
    let e = rpc_err(c.call("apply", json!({"command":{"op":"download_video","id":live}})));
    assert_eq!(e.kind(), "invalid_params", "nor a stream: {e}");
    // The temporary download is gone.
    let downloads = dir.join("cache").join("downloads");
    assert!(std::fs::read_dir(&downloads).map(|mut d| d.next().is_none()).unwrap_or(true));
    h.stop().unwrap();

    // Observe permission: refused before any request goes out.
    let n = log.lock().unwrap().len();
    let h = HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Observe }, None).unwrap();
    let mut c = Client::connect_hello(&h.socket, "web-read", &h.token).unwrap();
    let e = rpc_err(c.call("apply", json!({"command":{"op":"add_web_video","url":format!("{web}/talk.mp4")}})));
    assert_eq!(e.kind(), "permission", "{e}");
    assert_eq!(log.lock().unwrap().len(), n, "no request without permission");
    h.stop().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
