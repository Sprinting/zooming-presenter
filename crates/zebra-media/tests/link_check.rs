//! Checking web video links against a local server that stands in for the
//! web (the tests never touch the internet): a file, a page, a picture,
//! sign-in, missing, slow, streams, and YouTube's and Vimeo's lookups.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zebra_core::LinkKind;
use zebra_media::link::{Checker, LinkError};

/// Requests the server saw: (path with query, had a Cookie header).
type Log = Arc<Mutex<Vec<(String, bool)>>>;

struct Server {
    base: String,
    log: Log,
}

fn png(w: u32, h: u32, bars: u32) -> Vec<u8> {
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([0, 0, 0]));
    for y in bars..h - bars {
        for x in 0..w {
            img.put_pixel(x, y, image::Rgb([220, 40, 40]));
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn respond(mut s: TcpStream, base: &str, log: &Log) {
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
    let cookie = req.lines().any(|l| l.to_ascii_lowercase().starts_with("cookie:"));
    log.lock().unwrap().push((target.clone(), cookie));
    let path = target.split('?').next().unwrap_or("").to_string();
    let query = target.split_once('?').map(|(_, q)| q.to_string()).unwrap_or_default();
    let mp4 = zebra_core::media::fixtures::tiny_mp4(1280, 720, 30, true);
    let (status, ctype, body, extra): (&str, &str, Vec<u8>, String) = match path.as_str() {
        "/talk.mp4" => ("200 OK", "video/mp4", mp4, "Set-Cookie: session=abc\r\n".into()),
        "/octet.mov" => ("206 Partial Content", "application/octet-stream", mp4, String::new()),
        "/page.html" => ("200 OK", "text/html; charset=utf-8", b"<!doctype html><html><body>Hi</body></html>".to_vec(), String::new()),
        "/pic.png" => ("200 OK", "image/png", png(8, 8, 0), String::new()),
        "/private.mp4" => ("403 Forbidden", "text/plain", b"no".to_vec(), String::new()),
        "/locked.mp4" => ("401 Unauthorized", "text/plain", b"no".to_vec(), String::new()),
        "/gone.mp4" => ("404 Not Found", "text/plain", b"no".to_vec(), String::new()),
        "/broken.mp4" => ("503 Service Unavailable", "text/plain", b"no".to_vec(), String::new()),
        "/slow.mp4" => {
            std::thread::sleep(Duration::from_secs(3));
            ("200 OK", "video/mp4", mp4, String::new())
        }
        "/members/talk.mp4" => ("302 Found", "text/plain", vec![], format!("Location: {base}/login?next=/members/talk.mp4\r\n")),
        "/moved/talk.mp4" => ("301 Moved Permanently", "text/plain", vec![], format!("Location: {base}/talk.mp4\r\n")),
        "/login" => ("200 OK", "text/html", b"<html><form>Sign in</form></html>".to_vec(), String::new()),
        "/master.m3u8" => (
            "200 OK",
            "application/vnd.apple.mpegurl",
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nv/low.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=4000000,RESOLUTION=1920x1080\nv/high.m3u8\n".to_vec(),
            String::new(),
        ),
        "/v/low.m3u8" => ("200 OK", "application/vnd.apple.mpegurl", b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\na.ts\n#EXTINF:6.0,\nb.ts\n#EXT-X-ENDLIST\n".to_vec(), String::new()),
        "/live.m3u8" => ("200 OK", "application/x-mpegURL", b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=1280x720\nv/live.m3u8\n".to_vec(), String::new()),
        "/v/live.m3u8" => ("200 OK", "application/x-mpegURL", b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:120\n#EXTINF:6.0,\nx.ts\n".to_vec(), String::new()),
        "/oembed/youtube" if query.contains("privateVid1") => ("401 Unauthorized", "text/plain", b"Unauthorized".to_vec(), String::new()),
        "/oembed/youtube" if query.contains("missingVid1") => ("404 Not Found", "text/plain", b"Not Found".to_vec(), String::new()),
        "/oembed/youtube" => (
            "200 OK",
            "application/json",
            format!(r#"{{"type":"video","title":"Launch talk","width":200,"height":113,"thumbnail_url":"{base}/thumb/hq.png"}}"#).into_bytes(),
            String::new(),
        ),
        "/yt/dQw4w9WgXcQ/maxresdefault.jpg" => ("404 Not Found", "text/plain", vec![], String::new()),
        "/yt/bigThumb012/maxresdefault.jpg" => ("200 OK", "image/png", png(160, 90, 0), String::new()),
        "/thumb/hq.png" => ("200 OK", "image/png", png(48, 36, 4), String::new()),
        "/oembed/vimeo" if query.contains("999999") => ("403 Forbidden", "text/plain", vec![], String::new()),
        "/oembed/vimeo" => (
            "200 OK",
            "application/json",
            format!(r#"{{"type":"video","title":"Vimeo talk","width":1280,"height":720,"duration":95,"thumbnail_url":"{base}/thumb/vimeo.png"}}"#).into_bytes(),
            String::new(),
        ),
        "/thumb/vimeo.png" => ("200 OK", "image/png", png(64, 36, 0), String::new()),
        _ => ("404 Not Found", "text/plain", vec![], String::new()),
    };
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(&body);
}

fn server() -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let log: Log = Default::default();
    let (b2, l2) = (base.clone(), log.clone());
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (b, lg) = (b2.clone(), l2.clone());
            std::thread::spawn(move || respond(s, &b, &lg));
        }
    });
    Server { base, log }
}

fn checker(s: &Server) -> Checker {
    Checker {
        timeout: Duration::from_millis(1500),
        youtube_oembed: format!("{}/oembed/youtube", s.base),
        vimeo_oembed: format!("{}/oembed/vimeo", s.base),
        youtube_thumbs: format!("{}/yt/", s.base),
        allow_http: true,
    }
}

#[test]
fn an_mp4_link_is_a_video_file() {
    let s = server();
    let c = checker(&s).check(&format!("{}/talk.mp4", s.base)).unwrap();
    assert_eq!((c.kind, c.width, c.height, c.duration), (LinkKind::File, 1280, 720, 30.0));
    assert!(c.poster.is_none(), "a file's poster is its first frame, taken by the player");
    // By its bytes when the server doesn't say.
    assert_eq!(checker(&s).check(&format!("{}/octet.mov", s.base)).unwrap().kind, LinkKind::File);
    // A plain redirect to the file is fine.
    assert_eq!(checker(&s).check(&format!("{}/moved/talk.mp4", s.base)).unwrap().kind, LinkKind::File);
    // Only the start of the file was asked for.
    assert!(s.log.lock().unwrap().iter().all(|(_, cookie)| !cookie), "no cookies, even after Set-Cookie");
}

#[test]
fn an_http_link_is_refused_before_any_request() {
    let s = server();
    let real = Checker { allow_http: false, ..checker(&s) };
    assert_eq!(real.check(&format!("{}/talk.mp4", s.base)), Err(LinkError::Http));
    assert_eq!(LinkError::Http.message(), "Use an https link.");
    assert!(s.log.lock().unwrap().is_empty());
}

#[test]
fn pages_and_pictures_are_not_videos() {
    let s = server();
    let c = checker(&s);
    for p in ["/page.html", "/pic.png"] {
        let e = c.check(&format!("{}{p}", s.base)).unwrap_err();
        assert_eq!(e, LinkError::NotVideo, "{p}");
        assert_eq!(e.message(), "This link isn't a video.");
    }
}

#[test]
fn sign_in_is_401_403_or_a_redirect_to_a_sign_in_page() {
    let s = server();
    let c = checker(&s);
    for p in ["/private.mp4", "/locked.mp4", "/members/talk.mp4"] {
        let e = c.check(&format!("{}{p}", s.base)).unwrap_err();
        assert_eq!(e, LinkError::SignIn, "{p}");
    }
    assert_eq!(LinkError::SignIn.message(), "This video needs a sign-in.");
}

#[test]
fn missing_and_unreachable() {
    let s = server();
    let c = checker(&s);
    assert_eq!(c.check(&format!("{}/gone.mp4", s.base)), Err(LinkError::NotFound));
    assert_eq!(LinkError::NotFound.message(), "Nothing found at this link.");
    assert_eq!(c.check(&format!("{}/broken.mp4", s.base)), Err(LinkError::Unreachable));
    // Nobody listening.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    assert_eq!(c.check(&format!("http://127.0.0.1:{port}/talk.mp4")), Err(LinkError::Unreachable));
    assert_eq!(LinkError::Unreachable.message(), "Couldn't reach this link. Check your connection.");
}

/// The check gives up at its deadline, whatever the server does.
#[test]
fn a_slow_server_times_out() {
    let s = server();
    let c = Checker { timeout: Duration::from_millis(300), ..checker(&s) };
    let t = std::time::Instant::now();
    assert_eq!(c.check(&format!("{}/slow.mp4", s.base)), Err(LinkError::Unreachable));
    assert!(t.elapsed() < Duration::from_millis(1500), "{:?}", t.elapsed());
}

#[test]
fn hls_streams_and_live_streams() {
    let s = server();
    let c = checker(&s);
    let vod = c.check(&format!("{}/master.m3u8", s.base)).unwrap();
    assert_eq!((vod.kind, vod.width, vod.height, vod.duration), (LinkKind::Stream, 1920, 1080, 12.0));
    let live = c.check(&format!("{}/live.m3u8", s.base)).unwrap();
    assert_eq!((live.kind, live.width, live.height, live.duration), (LinkKind::Live, 1280, 720, 0.0));
}

/// YouTube and Vimeo: never the page itself, only the site's oEmbed
/// lookup of the link (nothing else in its query) and the thumbnail.
#[test]
fn youtube_and_vimeo_are_cards_with_the_sites_thumbnail() {
    let s = server();
    let c = checker(&s);
    let yt = c.check("https://youtu.be/dQw4w9WgXcQ?t=5").unwrap();
    assert_eq!(yt.kind, LinkKind::Youtube);
    assert_eq!(yt.title.as_deref(), Some("Launch talk"));
    // No maxres thumbnail here: the lookup's, with its black bars cut.
    let poster = image::load_from_memory(yt.poster.as_ref().unwrap()).unwrap();
    assert_eq!((poster.width(), poster.height()), (48, 28));
    assert_eq!((yt.width, yt.height), (48, 28));
    let big = c.check("https://www.youtube.com/watch?v=bigThumb012").unwrap();
    assert_eq!((big.width, big.height), (160, 90), "the maxres thumbnail when there is one");

    let vm = c.check("https://vimeo.com/76979871").unwrap();
    assert_eq!((vm.kind, vm.duration, vm.title.as_deref()), (LinkKind::Vimeo, 95.0, Some("Vimeo talk")));
    assert_eq!((vm.width, vm.height), (64, 36));

    assert_eq!(c.check("https://youtu.be/privateVid1"), Err(LinkError::SignIn));
    assert_eq!(c.check("https://youtu.be/missingVid1"), Err(LinkError::NotFound));
    assert_eq!(c.check("https://vimeo.com/999999"), Err(LinkError::SignIn));

    let log = s.log.lock().unwrap();
    let lookups: Vec<&String> = log.iter().map(|(p, _)| p).filter(|p| p.starts_with("/oembed/")).collect();
    assert!(!lookups.is_empty());
    for l in lookups {
        let q = l.split_once('?').unwrap().1;
        let keys: Vec<&str> = q.split('&').map(|kv| kv.split('=').next().unwrap()).collect();
        assert!(keys.iter().all(|k| ["format", "url", "width"].contains(k)), "only the link goes in the lookup: {l}");
    }
    assert!(log.iter().all(|(p, _)| p.starts_with("/oembed/") || p.starts_with("/yt/") || p.starts_with("/thumb/")), "only the lookups and thumbnails: {log:?}");
}

/// Download saves the whole file and leaves no part file behind.
#[test]
fn download_streams_to_a_file() {
    let s = server();
    let dir = std::env::temp_dir().join(format!("zebra-link-dl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let to = dir.join("talk.mp4");
    let stop = std::sync::atomic::AtomicBool::new(false);
    let n = checker(&s).download(&format!("{}/talk.mp4", s.base), &to, &stop).unwrap();
    assert_eq!(std::fs::read(&to).unwrap(), zebra_core::media::fixtures::tiny_mp4(1280, 720, 30, true));
    assert_eq!(n as usize, std::fs::metadata(&to).unwrap().len() as usize);
    let page = dir.join("page.mp4");
    assert_eq!(checker(&s).download(&format!("{}/page.html", s.base), &page, &stop), Err(LinkError::NotVideo));
    assert!(!page.exists() && !page.with_extension("part").exists());
    assert_eq!(checker(&s).download(&format!("{}/gone.mp4", s.base), &page, &stop), Err(LinkError::NotFound));
}
