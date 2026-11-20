//! `zebractl add web-video`: the same checks and messages as the Insert
//! Web Video sheet, against a local server that answers for the made-up
//! host `https://zebra-test.invalid` (nothing reaches the internet). Its
//! own test binary: it sets the environment of the in-process instance.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn ctl(h: &HeadlessHandle, data: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_zebractl"))
        .args(args)
        .env("ZEBRA_DATA_DIR", data)
        .env("ZEBRA_TOKEN", &h.token)
        .arg("--socket")
        .arg(&h.socket)
        .output()
        .unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

/// A tiny web: an MP4, a page, and YouTube's lookup.
fn server() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut s in l.incoming().flatten() {
            std::thread::spawn(move || {
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
                let path = target.split('?').next().unwrap_or("");
                let (status, ctype, body) = match path {
                    "/talk.mp4" => ("200 OK", "video/mp4", zebra_core::media::fixtures::tiny_mp4(640, 360, 12, true)),
                    "/page.html" => ("200 OK", "text/html", b"<!doctype html>".to_vec()),
                    "/oembed/youtube" => ("200 OK", "application/json", br#"{"type":"video","title":"Keynote","width":480,"height":270}"#.to_vec()),
                    _ => ("404 Not Found", "text/plain", vec![]),
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            });
        }
    });
    base
}

#[test]
fn zebractl_adds_web_videos() {
    let base = server();
    let dir = std::env::temp_dir().join(format!("zebractl-web-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data = dir.join("data");
    // SAFETY: the only test in this binary, before the instance starts.
    unsafe {
        std::env::set_var("ZEBRA_DATA_DIR", &data);
        std::env::set_var("ZEBRA_MEDIA_CACHE", dir.join("cache"));
        std::env::set_var("ZEBRA_FAKE_MEDIA", "1");
        std::env::set_var("ZEBRA_TEST_WEB", &base);
    }
    let h = HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Full }, None).unwrap();
    let web = zebra_media::link::STAND_IN;

    let (ok, out, err) = ctl(&h, &data, &["--agent", "t", "add", "web-video", &format!("{web}/talk.mp4"), "--width", "640", "--start", "2"]);
    assert!(ok, "{err}");
    assert!(out.contains("created"), "{out}");
    let (ok, out, err) = ctl(&h, &data, &["--agent", "t", "add", "web-video", "https://youtu.be/dQw4w9WgXcQ", "--x", "900"]);
    assert!(ok, "a card: {err}");
    assert!(out.contains("created"), "{out}");
    let (_, out, _) = ctl(&h, &data, &["--agent", "t", "describe"]);
    assert!(out.contains("web file") && out.contains("youtube card"), "{out}");

    for (url, message) in [
        (format!("{base}/talk.mp4"), "Use an https link."),
        (format!("{web}/page.html"), "This link isn't a video."),
        (format!("{web}/gone.mp4"), "Nothing found at this link."),
        ("https://me:pw@example.com/v.mp4".to_string(), "Links with a password in them can't be used."),
    ] {
        let (ok, _, err) = ctl(&h, &data, &["--agent", "t", "add", "web-video", &url]);
        assert!(!ok && err.contains(message), "{url}: {err}");
    }
    h.stop().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
