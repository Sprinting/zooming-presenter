//! Checking a web video link (Insert › Web Video…, `add_web_video`).
//!
//! Plain Rust over HTTPS (ureq with the platform's TLS: Security.framework
//! on the Mac), so it runs and is tested the same everywhere. No cookies
//! are kept and nothing signs in; the only requests are to the link itself
//! and, for YouTube and Vimeo, the site's oEmbed lookup of that link and
//! its thumbnail. The whole check shares one deadline (10 s).
//!
//! * A file link: the first 64 KB tell an MP4, MOV or M4V apart from a web
//!   page or a picture, and give its size and length when the file starts
//!   with its header.
//! * An HLS stream (.m3u8): the playlist gives the size; its first variant
//!   says whether it is live (no end).
//! * YouTube and Vimeo: the oEmbed lookup says whether the video is there
//!   and public, and gives its title and thumbnail (the card's poster).
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub use zebra_core::weblink::LinkError;
use zebra_core::weblink::{self, Site};
use zebra_core::LinkKind;

/// How much of a file link is read to know what it is.
const HEAD_BYTES: u64 = 64 * 1024;
/// Most of a playlist or an oEmbed answer that is read.
const TEXT_BYTES: u64 = 512 * 1024;
/// Most of a thumbnail that is read.
const IMAGE_BYTES: u64 = 8 * 1024 * 1024;

/// Where the checks go. `Default` is the real web; tests point it at a
/// local server.
#[derive(Debug, Clone)]
pub struct Checker {
    /// For the whole check (the sheet's "Checking…" is never longer).
    pub timeout: Duration,
    pub youtube_oembed: String,
    pub vimeo_oembed: String,
    /// Where YouTube keeps a video's thumbnails: `{this}{id}/maxresdefault.jpg`.
    pub youtube_thumbs: String,
    /// Tests only: let http links through so a local server can stand in
    /// for the web. Zebra itself only ever opens https links.
    #[doc(hidden)]
    pub allow_http: bool,
}

impl Default for Checker {
    fn default() -> Self {
        Checker {
            timeout: Duration::from_secs(10),
            youtube_oembed: "https://www.youtube.com/oembed".into(),
            vimeo_oembed: "https://vimeo.com/api/oembed.json".into(),
            youtube_thumbs: "https://i.ytimg.com/vi/".into(),
            allow_http: false,
        }
    }
}

impl Checker {
    /// The real web. Debug builds only: `ZEBRA_TEST_WEB` (a local
    /// `http://127.0.0.1:port`) stands in for YouTube's and Vimeo's
    /// lookups (`/oembed/youtube`, `/oembed/vimeo`, `/yt/`) and lets http
    /// links through, and `ZEBRA_TEST_WEB_TIMEOUT_MS` shortens the wait,
    /// so tests never touch the internet.
    pub fn from_env() -> Checker {
        let mut c = Checker::default();
        #[cfg(debug_assertions)]
        if let Some(base) = std::env::var("ZEBRA_TEST_WEB").ok().filter(|b| b.starts_with("http://127.0.0.1:")) {
            c.youtube_oembed = format!("{base}/oembed/youtube");
            c.vimeo_oembed = format!("{base}/oembed/vimeo");
            c.youtube_thumbs = format!("{base}/yt/");
            c.allow_http = true;
            if let Some(ms) = std::env::var("ZEBRA_TEST_WEB_TIMEOUT_MS").ok().and_then(|m| m.parse().ok()) {
                c.timeout = Duration::from_millis(ms);
            }
        }
        c
    }
}

/// What a link that plays is.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub kind: LinkKind,
    /// Pixels; 0 when the link doesn't say (the poster or the player will).
    pub width: u32,
    pub height: u32,
    /// Seconds; 0 when unknown or live.
    pub duration: f64,
    /// A card's poster: the site's thumbnail, JPEG or PNG.
    pub poster: Option<Vec<u8>>,
    /// A card's title on its site.
    pub title: Option<String>,
}

/// One answer, with at most a few kilobytes of its body.
struct Answer {
    status: u16,
    content_type: String,
    /// Where the answer came from after redirects.
    final_url: String,
    redirected: bool,
    body: Vec<u8>,
}

fn net(_: ureq::Error) -> LinkError {
    // No connection, no such host, a time-out, a bad certificate, a
    // redirect loop: all "couldn't reach".
    LinkError::Unreachable
}

impl Checker {
    fn agent(&self, deadline: Instant) -> Result<ureq::Agent, LinkError> {
        let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or(LinkError::Unreachable)?;
        Ok(agent_with(Some(left)))
    }

    fn get(&self, url: &str, max: u64, range: bool, deadline: Instant) -> Result<Answer, LinkError> {
        let agent = self.agent(deadline)?;
        let mut req = agent.get(url).header("Accept", "*/*");
        if range {
            // Ask for the start only; a server that can't still sends all
            // of it, of which only `max` bytes are read.
            req = req.header("Range", &format!("bytes=0-{}", max - 1));
        }
        let mut resp = req.call().map_err(net)?;
        use ureq::ResponseExt;
        let final_url = resp.get_uri().to_string();
        let redirected = resp.get_redirect_history().is_some_and(|h| h.len() > 1) || final_url != url;
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
            .unwrap_or_default();
        let mut body = vec![];
        if status < 400 {
            resp.body_mut().as_reader().take(max).read_to_end(&mut body).map_err(|_| LinkError::Unreachable)?;
        }
        Ok(Answer { status, content_type, final_url, redirected, body })
    }

    /// Check `url` as the Insert Web Video sheet does. Never longer than
    /// `timeout`.
    pub fn check(&self, url: &str) -> Result<Checked, LinkError> {
        let deadline = Instant::now() + self.timeout;
        let (_, site) = match weblink::classify(url) {
            Err(LinkError::Http) if self.allow_http => {
                let u = weblink::parse(url).ok_or(LinkError::NotLink)?;
                let s = weblink::site(&u);
                (u, s)
            }
            r => r?,
        };
        match site {
            Site::Youtube { id } => self.card(LinkKind::Youtube, &format!("https://www.youtube.com/watch?v={id}"), Some(&id), deadline),
            Site::Vimeo { .. } => self.card(LinkKind::Vimeo, url.trim(), None, deadline),
            Site::Web => self.web(url.trim(), deadline),
        }
    }

    fn web(&self, url: &str, deadline: Instant) -> Result<Checked, LinkError> {
        let a = self.get(url, HEAD_BYTES, true, deadline)?;
        status(a.status)?;
        if is_hls(&a.content_type, &a.body) {
            return self.hls(&a, deadline);
        }
        if is_movie(&a.body) || (a.body.is_empty() && is_movie_type(&a.content_type)) {
            let info = zebra_core::media::info(&a.body).ok().filter(|i| i.kind == zebra_core::MediaKind::Video);
            return Ok(Checked {
                kind: LinkKind::File,
                width: info.as_ref().map_or(0, |i| i.width),
                height: info.as_ref().map_or(0, |i| i.height),
                duration: info.as_ref().map_or(0.0, |i| i.duration),
                poster: None,
                title: None,
            });
        }
        if is_html(&a.content_type, &a.body) && a.redirected && looks_like_sign_in(&a.final_url) {
            return Err(LinkError::SignIn);
        }
        Err(LinkError::NotVideo)
    }

    fn hls(&self, a: &Answer, deadline: Instant) -> Result<Checked, LinkError> {
        let text = String::from_utf8_lossy(&a.body);
        let pl = parse_playlist(&text);
        let (mut w, mut h) = pl.resolution.unwrap_or((0, 0));
        let media = match pl.first_variant {
            Some(v) => {
                // A master playlist: its first variant says whether it ends.
                let vurl = resolve(&a.final_url, &v);
                match self.get(&vurl, TEXT_BYTES, false, deadline) {
                    Ok(b) if b.status < 400 => {
                        let p = parse_playlist(&String::from_utf8_lossy(&b.body));
                        if (w, h) == (0, 0) {
                            (w, h) = p.resolution.unwrap_or((0, 0));
                        }
                        Some(p)
                    }
                    // The player will find out; the stream itself answered.
                    _ => None,
                }
            }
            None => Some(pl),
        };
        let live = media.as_ref().is_some_and(|m| !m.ends);
        let duration = media.filter(|m| m.ends).map_or(0.0, |m| m.duration);
        Ok(Checked { kind: if live { LinkKind::Live } else { LinkKind::Stream }, width: w, height: h, duration, poster: None, title: None })
    }

    fn card(&self, kind: LinkKind, url: &str, youtube_id: Option<&str>, deadline: Instant) -> Result<Checked, LinkError> {
        let base = if kind == LinkKind::Youtube { &self.youtube_oembed } else { &self.vimeo_oembed };
        // The lookup carries the video's link and nothing else.
        let mut lookup = format!("{base}?format=json&url={}", weblink::encode(url));
        if kind == LinkKind::Vimeo {
            lookup.push_str("&width=1280");
        }
        let a = self.get(&lookup, TEXT_BYTES, false, deadline)?;
        match a.status {
            401 | 403 => return Err(LinkError::SignIn),
            s if s >= 500 => return Err(LinkError::Unreachable),
            s if s >= 400 => return Err(LinkError::NotFound),
            _ => {}
        }
        let o = parse_oembed(&a.body).ok_or(LinkError::NotFound)?;
        // The biggest thumbnail without black bars: YouTube's maxres one
        // where the video has it, else the lookup's (bars cut off).
        let mut poster = None;
        if let Some(id) = youtube_id {
            let big = format!("{}{id}/maxresdefault.jpg", self.youtube_thumbs);
            if let Ok(b) = self.get(&big, IMAGE_BYTES, false, deadline) {
                if b.status < 400 && image::guess_format(&b.body).is_ok() {
                    poster = Some(b.body);
                }
            }
        }
        if poster.is_none() {
            if let Some(t) = &o.thumbnail_url {
                if let Ok(b) = self.get(t, IMAGE_BYTES, false, deadline) {
                    if b.status < 400 && image::guess_format(&b.body).is_ok() {
                        poster = Some(trim_letterbox(b.body));
                    }
                }
            }
        }
        let (mut w, mut h) = poster.as_deref().and_then(|p| image::load_from_memory(p).ok()).map(|i| (i.width(), i.height())).unwrap_or((0, 0));
        if (w, h) == (0, 0) {
            if let (Some(ow), Some(oh)) = (o.width, o.height) {
                if ow > 0 && oh > 0 {
                    (w, h) = (1280, (1280 * oh / ow).max(1));
                }
            }
        }
        Ok(Checked { kind, width: w, height: h, duration: o.duration.unwrap_or(0.0), poster, title: o.title })
    }

    /// Save a file link's video to `to` (Download in the inspector). Stops
    /// early, leaving no file, when `cancel` is set.
    pub fn download(&self, url: &str, to: &Path, cancel: &AtomicBool) -> Result<u64, LinkError> {
        if !(weblink::is_https_link(url) || self.allow_http) {
            return Err(LinkError::Http);
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(self.timeout))
            .timeout_recv_response(Some(self.timeout))
            .http_status_as_error(false)
            .max_redirects(5)
            .user_agent("Zebra")
            .tls_config(tls())
            .build()
            .into();
        let mut resp = agent.get(url).call().map_err(net)?;
        status(resp.status().as_u16())?;
        let tmp = to.with_extension("part");
        let result = (|| -> Result<u64, LinkError> {
            let mut f = std::fs::File::create(&tmp).map_err(|_| LinkError::Unreachable)?;
            let mut r = resp.body_mut().with_config().limit(u64::MAX).reader();
            let mut buf = vec![0u8; 256 * 1024];
            let mut n = 0u64;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(LinkError::Unreachable);
                }
                let k = r.read(&mut buf).map_err(|_| LinkError::Unreachable)?;
                if k == 0 {
                    break;
                }
                if n == 0 && !is_movie(&buf[..k]) {
                    return Err(LinkError::NotVideo);
                }
                std::io::Write::write_all(&mut f, &buf[..k]).map_err(|_| LinkError::Unreachable)?;
                n += k as u64;
            }
            f.sync_all().map_err(|_| LinkError::Unreachable)?;
            Ok(n)
        })();
        match result {
            Ok(n) => {
                std::fs::rename(&tmp, to).map_err(|_| LinkError::Unreachable)?;
                Ok(n)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }
}

fn tls() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).root_certs(ureq::tls::RootCerts::PlatformVerifier).build()
}

/// An agent with no cookie store (ureq keeps none without its `cookies`
/// feature), the platform's TLS and certificates, and HTTP errors as
/// answers rather than failures.
fn agent_with(timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(timeout)
        .http_status_as_error(false)
        .max_redirects(5)
        .save_redirect_history(true)
        .user_agent("Zebra")
        .tls_config(tls())
        .build()
        .into()
}

/// What an HTTP status says about the link.
fn status(s: u16) -> Result<(), LinkError> {
    match s {
        200..=299 => Ok(()),
        401 | 403 | 407 => Err(LinkError::SignIn),
        429 | 500..=599 => Err(LinkError::Unreachable),
        _ => Err(LinkError::NotFound),
    }
}

/// An MP4, MOV or M4V by its first box.
pub fn is_movie(b: &[u8]) -> bool {
    if b.len() >= 12 && &b[4..8] == b"ftyp" {
        // Audio-only brands are not a video.
        return !matches!(&b[8..12], b"M4A " | b"M4B " | b"M4P ");
    }
    b.len() >= 8 && matches!(&b[4..8], b"moov" | b"mdat" | b"wide" | b"free" | b"skip" | b"pnot")
}

fn is_movie_type(ct: &str) -> bool {
    matches!(ct, "video/mp4" | "video/quicktime" | "video/x-m4v")
}

fn is_hls(ct: &str, body: &[u8]) -> bool {
    let typed = matches!(ct, "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "audio/mpegurl" | "audio/x-mpegurl");
    let text = String::from_utf8_lossy(&body[..body.len().min(64)]);
    text.trim_start_matches('\u{feff}').trim_start().starts_with("#EXTM3U") || (typed && body.is_empty())
}

fn is_html(ct: &str, body: &[u8]) -> bool {
    if ct == "text/html" || ct == "application/xhtml+xml" {
        return true;
    }
    let head = String::from_utf8_lossy(&body[..body.len().min(512)]).to_ascii_lowercase();
    let head = head.trim_start();
    head.starts_with("<!doctype html") || head.starts_with("<html")
}

/// A sign-in page by its address (where a private file link redirects).
pub fn looks_like_sign_in(url: &str) -> bool {
    let Some(u) = weblink::parse(url) else { return false };
    let path = u.path.to_ascii_lowercase();
    let host = u.host.as_str();
    ["login", "log-in", "signin", "sign-in", "sign_in", "logon", "/auth", "/sso", "/oauth", "/saml", "/account/", "/accounts/"]
        .iter()
        .any(|w| path.contains(w))
        || host.starts_with("login.")
        || host.starts_with("signin.")
        || host.starts_with("accounts.")
        || host.starts_with("auth.")
        || host.starts_with("sso.")
}

/// What an HLS playlist says.
#[derive(Debug, Default, PartialEq)]
pub struct Playlist {
    /// A master playlist: the first variant's link, as written.
    pub first_variant: Option<String>,
    /// The largest RESOLUTION named.
    pub resolution: Option<(u32, u32)>,
    /// A media playlist with an end (VOD), not live.
    pub ends: bool,
    /// Sum of the segments' lengths, seconds.
    pub duration: f64,
}

pub fn parse_playlist(text: &str) -> Playlist {
    let mut p = Playlist::default();
    let mut want_uri = false;
    for line in text.lines().map(str::trim) {
        if let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            want_uri = true;
            if let Some(r) = attr(attrs, "RESOLUTION").and_then(|r| r.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))) {
                let (w, h): (u32, u32) = r;
                if p.resolution.is_none_or(|(pw, ph)| w * h > pw * ph) {
                    p.resolution = Some((w, h));
                }
            }
        } else if line == "#EXT-X-ENDLIST" || line == "#EXT-X-PLAYLIST-TYPE:VOD" {
            p.ends = true;
        } else if let Some(d) = line.strip_prefix("#EXTINF:") {
            p.duration += d.split(',').next().and_then(|d| d.trim().parse::<f64>().ok()).filter(|d| d.is_finite() && *d > 0.0).unwrap_or(0.0);
        } else if want_uri && !line.is_empty() && !line.starts_with('#') {
            want_uri = false;
            if p.first_variant.is_none() {
                p.first_variant = Some(line.to_string());
            }
        }
    }
    p
}

/// A playlist attribute's value (`KEY=value` or `KEY="value"`).
fn attr(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    while !rest.is_empty() {
        let (k, after) = rest.split_once('=')?;
        let (v, next) = if let Some(q) = after.strip_prefix('"') {
            let (v, n) = q.split_once('"').unwrap_or((q, ""));
            (v, n.strip_prefix(',').unwrap_or(n))
        } else {
            after.split_once(',').unwrap_or((after, ""))
        };
        if k.trim() == key {
            return Some(v.to_string());
        }
        rest = next;
    }
    None
}

/// A link in a playlist, against the playlist's own address.
pub fn resolve(base: &str, rel: &str) -> String {
    if weblink::parse(rel).is_some() {
        return rel.to_string();
    }
    let Some(b) = weblink::parse(base) else { return rel.to_string() };
    let origin = match b.port {
        Some(p) => format!("{}://{}:{p}", b.scheme, b.host),
        None => format!("{}://{}", b.scheme, b.host),
    };
    if let Some(r) = rel.strip_prefix("//") {
        return format!("{}://{r}", b.scheme);
    }
    if rel.starts_with('/') {
        return format!("{origin}{rel}");
    }
    let dir = b.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    format!("{origin}{dir}/{rel}")
}

/// What an oEmbed answer gives a card.
#[derive(Debug, Default, PartialEq)]
pub struct OEmbed {
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
}

/// Read an oEmbed JSON answer (YouTube's or Vimeo's). None when it isn't
/// one: no type, or not a video.
pub fn parse_oembed(body: &[u8]) -> Option<OEmbed> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let o = v.as_object()?;
    if o.get("type").and_then(|t| t.as_str()).is_some_and(|t| t != "video") {
        return None;
    }
    let num = |k: &str| o.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
    Some(OEmbed {
        title: o.get("title").and_then(|t| t.as_str()).map(str::to_string).filter(|t| !t.trim().is_empty()),
        thumbnail_url: o.get("thumbnail_url").and_then(|t| t.as_str()).filter(|t| weblink::parse(t).is_some()).map(str::to_string),
        width: num("width").filter(|w| *w > 0.0).map(|w| w as u32),
        height: num("height").filter(|h| *h > 0.0).map(|h| h as u32),
        duration: num("duration").filter(|d| d.is_finite() && *d > 0.0),
    })
}

/// Cut black bars off the top and bottom of a thumbnail (YouTube's 4:3
/// thumbnails of 16:9 videos have them). The picture as it was when it
/// has none or can't be read.
pub fn trim_letterbox(bytes: Vec<u8>) -> Vec<u8> {
    let Ok(img) = image::load_from_memory(&bytes) else { return bytes };
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h < 8 {
        return bytes;
    }
    let dark = |y: u32| (0..w).all(|x| rgb.get_pixel(x, y).0.iter().all(|c| *c < 24));
    let top = (0..h / 3).take_while(|y| dark(*y)).count() as u32;
    let bottom = (0..h / 3).take_while(|y| dark(h - 1 - *y)).count() as u32;
    if top < h / 50 || bottom < h / 50 {
        return bytes;
    }
    let cut = image::DynamicImage::ImageRgb8(image::imageops::crop_imm(&rgb, 0, top, w, h - top - bottom).to_image());
    let mut out = std::io::Cursor::new(Vec::new());
    match cut.write_to(&mut out, image::ImageFormat::Jpeg) {
        Ok(()) => out.into_inner(),
        Err(_) => bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlists() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.4d401e,mp4a.40.2\"\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\nhigh/index.m3u8\n";
        let p = parse_playlist(master);
        assert_eq!(p.first_variant.as_deref(), Some("low/index.m3u8"));
        assert_eq!(p.resolution, Some((1920, 1080)));
        let vod = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\na.ts\n#EXTINF:4.5,\nb.ts\n#EXT-X-ENDLIST\n";
        let p = parse_playlist(vod);
        assert!(p.ends && p.first_variant.is_none());
        assert_eq!(p.duration, 10.5);
        assert!(!parse_playlist("#EXTM3U\n#EXTINF:6.0,\na.ts\n").ends, "no end: live");
    }

    #[test]
    fn links_in_playlists() {
        let base = "https://cdn.example.com:8443/streams/talk/master.m3u8?token=1";
        assert_eq!(resolve(base, "low/index.m3u8"), "https://cdn.example.com:8443/streams/talk/low/index.m3u8");
        assert_eq!(resolve(base, "/x.m3u8"), "https://cdn.example.com:8443/x.m3u8");
        assert_eq!(resolve(base, "//other.example.com/y.m3u8"), "https://other.example.com/y.m3u8");
        assert_eq!(resolve(base, "https://z.example.com/z.m3u8"), "https://z.example.com/z.m3u8");
    }

    #[test]
    fn sign_in_pages() {
        assert!(looks_like_sign_in("https://example.com/login?next=/v.mp4"));
        assert!(looks_like_sign_in("https://accounts.example.com/o/oauth2"));
        assert!(looks_like_sign_in("https://example.com/users/sign_in"));
        assert!(!looks_like_sign_in("https://example.com/videos/talk.mp4"));
    }

    #[test]
    fn oembed_answers() {
        let yt = br#"{"title":"Never Gonna","author_name":"Rick","type":"video","height":113,"width":200,"version":"1.0","provider_name":"YouTube","thumbnail_height":360,"thumbnail_width":480,"thumbnail_url":"https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg","html":"<iframe></iframe>"}"#;
        let o = parse_oembed(yt).unwrap();
        assert_eq!(o.title.as_deref(), Some("Never Gonna"));
        assert_eq!(o.thumbnail_url.as_deref(), Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"));
        assert_eq!((o.width, o.height, o.duration), (Some(200), Some(113), None));
        let vm = br#"{"type":"video","title":"Talk","width":1280,"height":720,"duration":95,"thumbnail_url":"https://i.vimeocdn.com/video/1-d_1280x720"}"#;
        assert_eq!(parse_oembed(vm).unwrap().duration, Some(95.0));
        assert_eq!(parse_oembed(br#"{"type":"photo"}"#), None);
        assert_eq!(parse_oembed(b"<html>"), None);
    }

    #[test]
    fn letterbox_bars_are_cut() {
        let mut img = image::RgbImage::from_pixel(48, 36, image::Rgb([0, 0, 0]));
        for y in 4..32 {
            for x in 0..48 {
                img.put_pixel(x, y, image::Rgb([200, 100, 50]));
            }
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let out = image::load_from_memory(&trim_letterbox(png.into_inner())).unwrap();
        assert_eq!((out.width(), out.height()), (48, 28));
    }

    #[test]
    fn movies_by_their_first_box() {
        assert!(is_movie(&zebra_core::media::fixtures::tiny_mp4(16, 16, 1, true)));
        assert!(!is_movie(b"\0\0\0\x20ftypM4A \0\0\0\0"));
        assert!(!is_movie(b"<!doctype html><html>"));
        assert!(!is_movie(b"\x89PNG\r\n\x1a\n...."));
    }
}
