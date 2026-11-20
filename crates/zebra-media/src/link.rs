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
use std::path::{Path, PathBuf};
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
/// A download that receives nothing for this long has stalled.
pub const DOWNLOAD_IDLE: Duration = Duration::from_secs(30);
/// Space a download leaves free on the disk, and below which it stops.
pub const SPACE_MARGIN: u64 = 512 * 1024 * 1024;
/// How often (bytes written) a download looks at the free space again.
const SPACE_EVERY: u64 = 16 * 1024 * 1024;
/// Redirects followed before a link counts as unreachable (a loop).
const MAX_REDIRECTS: u32 = 5;

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
    /// for the web. Zebra itself only ever opens https links. It also
    /// keeps the requests on the portable transport (ureq) on the Mac,
    /// where the system's would refuse plain http.
    #[doc(hidden)]
    pub allow_http: bool,
    /// A download's stall time (`DOWNLOAD_IDLE`; tests shorten it).
    #[doc(hidden)]
    pub idle: Duration,
    /// Free bytes on the disk holding a path (tests stand in a full disk).
    #[doc(hidden)]
    pub free_space: fn(&Path) -> Option<u64>,
}

impl Default for Checker {
    fn default() -> Self {
        Checker {
            timeout: Duration::from_secs(10),
            youtube_oembed: "https://www.youtube.com/oembed".into(),
            vimeo_oembed: "https://vimeo.com/api/oembed.json".into(),
            youtube_thumbs: "https://i.ytimg.com/vi/".into(),
            allow_http: false,
            idle: DOWNLOAD_IDLE,
            free_space,
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
                c.idle = Duration::from_millis(ms);
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

/// Why a download stopped. The part file is gone in every case.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadError {
    /// The link itself: moved, private, not a video, unreachable, stalled.
    Link(LinkError),
    /// Not enough space on the disk for it (before or while saving).
    NoSpace,
    /// The disk refused the file.
    Disk,
    Cancelled,
}

impl DownloadError {
    pub fn code(&self) -> &'static str {
        match self {
            DownloadError::Link(e) => e.code(),
            DownloadError::NoSpace => "no_space",
            DownloadError::Disk => "io",
            DownloadError::Cancelled => "cancelled",
        }
    }
    pub fn message(&self) -> &'static str {
        match self {
            DownloadError::Link(e) => e.message(),
            DownloadError::NoSpace => "There isn't enough space on the disk for this video.",
            DownloadError::Disk => "Couldn't save the video.",
            DownloadError::Cancelled => "Download cancelled.",
        }
    }
}

impl From<LinkError> for DownloadError {
    fn from(e: LinkError) -> Self {
        DownloadError::Link(e)
    }
}

/// An answer's status line and headers, after redirects.
#[derive(Debug, Clone, Default)]
pub(crate) struct Head {
    pub status: u16,
    /// Lower case, without parameters.
    pub content_type: String,
    /// Where the answer came from after redirects.
    pub final_url: String,
    pub redirected: bool,
    pub length: Option<u64>,
}

/// An answer whose body is still to be read.
pub(crate) struct Opened {
    pub head: Head,
    pub body: Box<dyn Read + Send>,
}

/// One answer, with at most a few kilobytes of its body.
struct Answer {
    head: Head,
    body: Vec<u8>,
}

/// How long a request may take: the whole of it (a check) or between
/// two reads (a download).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub deadline: Option<Instant>,
    pub idle: Duration,
}

impl Limits {
    /// Time left, or Unreachable when it is up.
    pub fn left(&self) -> Result<Duration, LinkError> {
        match self.deadline {
            Some(d) => d.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).map(|d| d.min(self.idle)).ok_or(LinkError::Unreachable),
            None => Ok(self.idle),
        }
    }
}

/// An https link that lands on http has been downgraded: refused, even
/// when the http page would answer.
pub fn downgraded(from: &str, to: &str) -> bool {
    let scheme = |u: &str| u.trim().split_once("://").map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default();
    scheme(from) == "https" && scheme(to) != "https"
}

impl Checker {
    /// Open `url`: the system's transport on the Mac (its proxies and App
    /// Transport Security), ureq elsewhere and in tests. Never follows a
    /// redirect from https to http, nor more than five.
    fn open(&self, url: &str, range: Option<u64>, limits: Limits) -> Result<Opened, LinkError> {
        if !(weblink::is_https_link(url) || self.allow_http) {
            return Err(LinkError::Http);
        }
        #[cfg(target_os = "macos")]
        let o = if self.allow_http { ureq_open(url, range, limits, true)? } else { crate::mac_net::open(url, range, limits)? };
        #[cfg(not(target_os = "macos"))]
        let o = ureq_open(url, range, limits, self.allow_http)?;
        if downgraded(url, &o.head.final_url) {
            return Err(LinkError::Http);
        }
        Ok(o)
    }

    fn get(&self, url: &str, max: u64, range: bool, deadline: Instant) -> Result<Answer, LinkError> {
        let limits = Limits { deadline: Some(deadline), idle: self.timeout };
        let o = self.open(url, range.then_some(max), limits)?;
        let mut body = vec![];
        if o.head.status < 400 {
            o.body.take(max).read_to_end(&mut body).map_err(|_| LinkError::Unreachable)?;
        }
        Ok(Answer { head: o.head, body })
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
        status(a.head.status)?;
        if is_hls(&a.head.content_type, &a.body) {
            return self.hls(&a, deadline);
        }
        if is_movie(&a.body) || (a.body.is_empty() && is_movie_type(&a.head.content_type)) {
            let info = zebra_core::media::info(&a.body).ok().filter(|i| i.kind == zebra_core::MediaKind::Video);
            // A header with a nonsense size (0, or past what any screen
            // shows) says nothing; the player will.
            let (w, h) = info.as_ref().map(|i| (i.width, i.height)).filter(|(w, h)| weblink::poster_fits(*w, *h)).unwrap_or((0, 0));
            return Ok(Checked {
                kind: LinkKind::File,
                width: w,
                height: h,
                duration: info.as_ref().map(|i| i.duration).filter(|d| d.is_finite() && *d > 0.0).unwrap_or(0.0),
                poster: None,
                title: None,
            });
        }
        if is_html(&a.head.content_type, &a.body) && a.head.redirected && looks_like_sign_in(&a.head.final_url) {
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
                let vurl = resolve(&a.head.final_url, &v);
                match self.get(&vurl, TEXT_BYTES, false, deadline) {
                    Ok(b) if b.head.status < 400 => {
                        let p = parse_playlist(&String::from_utf8_lossy(&b.body));
                        if (w, h) == (0, 0) {
                            (w, h) = p.resolution.unwrap_or((0, 0));
                        }
                        Some(p)
                    }
                    // A downgrade is refused outright; anything else the
                    // player will find out (the stream itself answered).
                    Err(LinkError::Http) => return Err(LinkError::Http),
                    _ => None,
                }
            }
            None => Some(pl),
        };
        if !weblink::poster_fits(w, h) {
            (w, h) = (0, 0);
        }
        let live = media.as_ref().is_some_and(|m| !m.ends);
        let duration = media.filter(|m| m.ends).map_or(0.0, |m| m.duration);
        Ok(Checked { kind: if live { LinkKind::Live } else { LinkKind::Stream }, width: w, height: h, duration, poster: None, title: None })
    }

    /// A thumbnail at `url`, when it is a picture of a sensible size.
    /// Err(NotVideo) for one too big to decode safely.
    fn thumbnail(&self, url: &str, deadline: Instant) -> Result<Option<image::DynamicImage>, LinkError> {
        match self.get(url, IMAGE_BYTES, false, deadline) {
            Ok(b) if b.head.status < 400 => decode_thumbnail(&b.body),
            Err(LinkError::Http) => Err(LinkError::Http),
            _ => Ok(None),
        }
    }

    fn card(&self, kind: LinkKind, url: &str, youtube_id: Option<&str>, deadline: Instant) -> Result<Checked, LinkError> {
        let base = if kind == LinkKind::Youtube { &self.youtube_oembed } else { &self.vimeo_oembed };
        // The lookup carries the video's link and nothing else.
        let mut lookup = format!("{base}?format=json&url={}", weblink::encode(url));
        if kind == LinkKind::Vimeo {
            lookup.push_str("&width=1280");
        }
        let a = self.get(&lookup, TEXT_BYTES, false, deadline)?;
        match a.head.status {
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
            poster = self.thumbnail(&format!("{}{id}/maxresdefault.jpg", self.youtube_thumbs), deadline)?;
        }
        if poster.is_none() {
            if let Some(t) = &o.thumbnail_url {
                poster = self.thumbnail(t, deadline)?.map(trim_letterbox);
            }
        }
        let (mut w, mut h) = poster.as_ref().map(|i| (i.width(), i.height())).unwrap_or((0, 0));
        if (w, h) == (0, 0) {
            if let (Some(ow), Some(oh)) = (o.width, o.height) {
                if ow > 0 && oh > 0 && weblink::poster_fits(ow, oh) {
                    (w, h) = (1280, (1280u64 * oh as u64 / ow as u64).clamp(1, 8192) as u32);
                }
            }
        }
        let poster = poster.and_then(|p| jpeg(&p));
        Ok(Checked { kind, width: w, height: h, duration: o.duration.unwrap_or(0.0), poster, title: o.title })
    }

    /// Save a file link's video to `to` (Download in the inspector),
    /// streaming it to `to` + ".part" and renaming that when it is all
    /// there. `progress` hears (bytes so far, size when the server says).
    /// Stops when `cancel` is set, when nothing arrives for `idle`
    /// (30 s), or when the disk would be left with less than
    /// `SPACE_MARGIN`. The part file is deleted whatever stops it.
    pub fn download(&self, url: &str, to: &Path, cancel: &AtomicBool, progress: &dyn Fn(u64, Option<u64>)) -> Result<u64, DownloadError> {
        let part = PartFile::new(to);
        let dir = to.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let o = self.open(url, None, Limits { deadline: None, idle: self.idle })?;
        status(o.head.status)?;
        let need = o.head.length.unwrap_or(0).saturating_add(SPACE_MARGIN);
        if (self.free_space)(dir).is_some_and(|free| free < need) {
            return Err(DownloadError::NoSpace);
        }
        let mut f = std::fs::File::create(&part.0).map_err(|_| DownloadError::Disk)?;
        let mut r = o.body;
        let mut buf = vec![0u8; 256 * 1024];
        let mut head: Vec<u8> = Vec::with_capacity(16);
        let (mut n, mut next_space) = (0u64, SPACE_EVERY);
        progress(0, o.head.length);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
            let k = match r.read(&mut buf) {
                Ok(k) => k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(if cancel.load(Ordering::Relaxed) { DownloadError::Cancelled } else { LinkError::Unreachable.into() }),
            };
            if k == 0 {
                break;
            }
            // Only a movie is saved: its first box says so.
            if head.len() < 16 {
                head.extend_from_slice(&buf[..k.min(16 - head.len())]);
                if head.len() >= 16 && !is_movie(&head) {
                    return Err(LinkError::NotVideo.into());
                }
            }
            std::io::Write::write_all(&mut f, &buf[..k]).map_err(|e| if is_disk_full(&e) { DownloadError::NoSpace } else { DownloadError::Disk })?;
            n += k as u64;
            if n >= next_space {
                next_space = n + SPACE_EVERY;
                if (self.free_space)(dir).is_some_and(|free| free < SPACE_MARGIN) {
                    return Err(DownloadError::NoSpace);
                }
            }
            progress(n, o.head.length);
        }
        if head.len() < 16 && !is_movie(&head) {
            return Err(LinkError::NotVideo.into());
        }
        if o.head.length.is_some_and(|len| n < len) {
            // The connection closed early: a part of a file isn't a video.
            return Err(LinkError::Unreachable.into());
        }
        f.sync_all().map_err(|_| DownloadError::Disk)?;
        drop(f);
        part.finish(to).map_err(|_| DownloadError::Disk)?;
        Ok(n)
    }
}

/// `to` + ".part" while a download writes it. Deleted on drop unless the
/// download finished and renamed it.
struct PartFile(PathBuf);

impl PartFile {
    fn new(to: &Path) -> PartFile {
        let mut p = to.as_os_str().to_owned();
        p.push(".part");
        PartFile(PathBuf::from(p))
    }
    fn finish(self, to: &Path) -> std::io::Result<()> {
        std::fs::rename(&self.0, to)
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        // Gone already after a rename: nothing to do.
        let _ = std::fs::remove_file(&self.0);
    }
}

fn is_disk_full(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::StorageFull || e.raw_os_error() == Some(28) /* ENOSPC */
}

/// Free bytes on the disk holding `dir` (for the user, not root). None
/// when it can't be told.
pub fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a NUL-terminated path and `st` a writable statvfs.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

fn tls() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).root_certs(ureq::tls::RootCerts::PlatformVerifier).build()
}

/// An agent with no cookie store (ureq keeps none without its `cookies`
/// feature), the platform's TLS and certificates, https only (every
/// redirect too) unless a test allows http, and HTTP errors as answers
/// rather than failures.
fn ureq_open(url: &str, range: Option<u64>, limits: Limits, allow_http: bool) -> Result<Opened, LinkError> {
    let left = limits.left()?;
    let mut cfg = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .https_only(!allow_http)
        .max_redirects(MAX_REDIRECTS)
        .save_redirect_history(true)
        .user_agent("Zebra")
        .tls_config(tls())
        .timeout_connect(Some(left))
        .timeout_recv_response(Some(left))
        .timeout_per_read(Some(limits.idle));
    if limits.deadline.is_some() {
        cfg = cfg.timeout_global(Some(left));
    }
    let agent: ureq::Agent = cfg.build().into();
    let mut req = agent.get(url).header("Accept", "*/*");
    if let Some(max) = range {
        // Ask for the start only; a server that can't still sends all of
        // it, of which only `max` bytes are read.
        req = req.header("Range", &format!("bytes=0-{}", max.saturating_sub(1)));
    }
    let resp = req.call().map_err(net)?;
    use ureq::ResponseExt;
    let final_url = resp.get_uri().to_string();
    let redirected = resp.get_redirect_history().is_some_and(|h| h.len() > 1) || final_url != url;
    let header = |k: &str| resp.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let head = Head {
        status: resp.status().as_u16(),
        content_type: header("content-type").map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase()).unwrap_or_default(),
        final_url,
        redirected,
        length: header("content-length").and_then(|v| v.trim().parse().ok()),
    };
    let body = resp.into_body().into_with_config().limit(u64::MAX).reader();
    Ok(Opened { head, body: Box::new(body) })
}

fn net(e: ureq::Error) -> LinkError {
    match e {
        // A redirect to http (ureq checks every hop).
        ureq::Error::RequireHttpsOnly(_) => LinkError::Http,
        // No connection, no such host, a time-out, a bad certificate, a
        // redirect loop: all "couldn't reach".
        _ => LinkError::Unreachable,
    }
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

/// Decode a thumbnail, refusing one too big to be a thumbnail (more than
/// 8192 pixels a side or `weblink::POSTER_MAX_PIXELS` in all): its size
/// is read from the header before anything is decoded, and the decoder
/// is held to it. Ok(None) when it isn't a picture.
pub fn decode_thumbnail(bytes: &[u8]) -> Result<Option<image::DynamicImage>, LinkError> {
    let Ok(fmt) = image::guess_format(bytes) else { return Ok(None) };
    if !matches!(fmt, image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP) {
        return Ok(None);
    }
    let reader = || image::ImageReader::with_format(std::io::Cursor::new(bytes), fmt);
    let Ok((w, h)) = reader().into_dimensions() else { return Ok(None) };
    if w == 0 || h == 0 {
        return Ok(None);
    }
    if !weblink::poster_fits(w, h) {
        return Err(LinkError::NotVideo);
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(weblink::POSTER_MAX_SIDE);
    limits.max_image_height = Some(weblink::POSTER_MAX_SIDE);
    limits.max_alloc = Some(weblink::POSTER_MAX_PIXELS * 4 + (16 << 20));
    let mut r = reader();
    r.limits(limits);
    match r.decode() {
        Ok(i) => Ok(Some(i)),
        Err(image::ImageError::Limits(_)) => Err(LinkError::NotVideo),
        Err(_) => Ok(None),
    }
}

/// A poster as JPEG (what goes in the deck).
fn jpeg(img: &image::DynamicImage) -> Option<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut out, image::ImageFormat::Jpeg).ok()?;
    Some(out.into_inner())
}

/// Cut black bars off the top and bottom of a thumbnail (YouTube's 4:3
/// thumbnails of 16:9 videos have them). The picture as it was when it
/// has none.
pub fn trim_letterbox(img: image::DynamicImage) -> image::DynamicImage {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h < 8 {
        return img;
    }
    let dark = |y: u32| (0..w).all(|x| rgb.get_pixel(x, y).0.iter().all(|c| *c < 24));
    let top = (0..h / 3).take_while(|y| dark(*y)).count() as u32;
    let bottom = (0..h / 3).take_while(|y| dark(h - 1 - *y)).count() as u32;
    // Bars on both sides, leaving at least half: a dark picture (or an
    // all-black one) isn't letterboxed.
    if top < h / 50 || bottom < h / 50 || top + bottom > h / 2 {
        return img;
    }
    image::DynamicImage::ImageRgb8(image::imageops::crop_imm(&rgb, 0, top, w, h - top - bottom).to_image())
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
        let out = trim_letterbox(image::DynamicImage::ImageRgb8(img));
        assert_eq!((out.width(), out.height()), (48, 28));
        // All black: nothing left to keep, so it stays as it was.
        let black = trim_letterbox(image::DynamicImage::ImageRgb8(image::RgbImage::new(16, 9)));
        assert_eq!((black.width(), black.height()), (16, 9));
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image::GrayImage::new(w, h)).write_to(&mut out, image::ImageFormat::Png).unwrap();
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

    /// A thumbnail's size is read from its header before it is decoded;
    /// one past the limits is refused, whatever the bytes claim.
    #[test]
    fn thumbnails_are_size_checked_before_decoding() {
        assert_eq!(decode_thumbnail(&png(64, 36)).unwrap().map(|i| (i.width(), i.height())), Some((64, 36)));
        assert_eq!(decode_thumbnail(&png(8193, 1)).unwrap_err(), LinkError::NotVideo);
        // A header that claims a huge picture over a few bytes of data:
        // past 8192 a side, and (8000 x 8000) under it but past the count.
        for (w, h) in [(60000u32, 60000u32), (8000, 8000)] {
            assert_eq!(decode_thumbnail(&lying_png(w, h)).unwrap_err(), LinkError::NotVideo, "{w}x{h}");
        }
        // Within the limits but a lie all the same: no poster, no panic.
        assert!(decode_thumbnail(&lying_png(4000, 3000)).unwrap().is_none());
        // Empty, truncated, not a picture, a format not taken: no poster.
        assert!(decode_thumbnail(b"").unwrap().is_none());
        assert!(decode_thumbnail(&png(64, 36)[..40]).unwrap().is_none());
        assert!(decode_thumbnail(b"<html>").unwrap().is_none());
        assert!(decode_thumbnail(b"GIF89a\x01\0\x01\0\0\0\0;").unwrap().is_none());
    }

    #[test]
    fn https_never_lands_on_http() {
        assert!(downgraded("https://example.com/a.mp4", "http://example.com/a.mp4"));
        assert!(downgraded("HTTPS://example.com/a.mp4", "HTTP://cdn.example.com/a.mp4"));
        assert!(!downgraded("https://example.com/a.mp4", "https://cdn.example.com/a.mp4"));
        // Tests' local http server: nothing to downgrade from.
        assert!(!downgraded("http://127.0.0.1:1/a.mp4", "http://127.0.0.1:1/b.mp4"));
    }

    /// The real transport is https-only on every hop: ureq refuses an http
    /// address before connecting (port 1 would otherwise be "couldn't
    /// reach"), which is the same check it makes on each redirect.
    #[test]
    fn the_real_transport_refuses_http_on_every_hop() {
        let limits = Limits { deadline: Some(Instant::now() + Duration::from_secs(2)), idle: Duration::from_secs(2) };
        assert_eq!(ureq_open("http://127.0.0.1:1/a.mp4", None, limits, false).err(), Some(LinkError::Http));
        assert_eq!(ureq_open("http://127.0.0.1:1/a.mp4", None, limits, true).err(), Some(LinkError::Unreachable));
        assert_eq!(Checker::default().download("http://127.0.0.1:1/a.mp4", Path::new("/nonexistent/a.mp4"), &AtomicBool::new(false), &|_, _| {}), Err(DownloadError::Link(LinkError::Http)));
    }

    #[test]
    fn free_space_is_known_here() {
        assert!(free_space(&std::env::temp_dir()).is_some_and(|f| f > 0));
        assert_eq!(free_space(Path::new("/no/such/folder/anywhere")), None);
    }

    #[test]
    fn movies_by_their_first_box() {
        assert!(is_movie(&zebra_core::media::fixtures::tiny_mp4(16, 16, 1, true)));
        assert!(!is_movie(b"\0\0\0\x20ftypM4A \0\0\0\0"));
        assert!(!is_movie(b"<!doctype html><html>"));
        assert!(!is_movie(b"\x89PNG\r\n\x1a\n...."));
    }
}
