//! Web video links as text: is it an https link, which site is it on
//! (YouTube and Vimeo videos become cards that open the site's own
//! player), the site's name, and the site's embed player link with a start
//! and an end. Nothing here touches the network: checking a link is
//! `zebra_media::link`, which builds on this.
use crate::model::{LinkKind, VideoLink};

/// Why a link can't be a web video. The messages are the ones the Insert
/// Web Video sheet shows (the app shows its own translations of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkError {
    /// Not a link at all (or one with a sign-in name in it).
    NotLink,
    /// An http link: only https plays.
    Http,
    /// A web page, a picture, or a file that isn't a video.
    NotVideo,
    /// 401, 403, or a redirect to a sign-in page.
    SignIn,
    /// 404 or another "not here" answer.
    NotFound,
    /// No connection, a time-out, or a server error.
    Unreachable,
}

impl LinkError {
    pub fn message(self) -> &'static str {
        match self {
            LinkError::NotLink => "This isn't a link. Give an https link to a video.",
            LinkError::Http => "Use an https link.",
            LinkError::NotVideo => "This link isn't a video.",
            LinkError::SignIn => "This video needs a sign-in.",
            LinkError::NotFound => "Nothing found at this link.",
            LinkError::Unreachable => "Couldn't reach this link. Check your connection.",
        }
    }
    /// Error code for the API.
    pub fn code(self) -> &'static str {
        match self {
            LinkError::NotLink => "not_a_link",
            LinkError::Http => "not_https",
            LinkError::NotVideo => "not_a_video",
            LinkError::SignIn => "sign_in",
            LinkError::NotFound => "not_found",
            LinkError::Unreachable => "unreachable",
        }
    }
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for LinkError {}

/// A link taken apart. Scheme and host are lower case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    /// From the first `/` (empty when the link has none).
    pub path: String,
    /// Without the `?`.
    pub query: String,
    /// Without the `#`.
    pub fragment: String,
}

impl Url {
    /// A query parameter, percent-decoded.
    pub fn param(&self, name: &str) -> Option<String> {
        self.query.split('&').filter_map(|kv| kv.split_once('=').or(Some((kv, "")))).find(|(k, _)| *k == name).map(|(_, v)| decode(v))
    }
    /// The path's segments, empty ones left out.
    pub fn segments(&self) -> Vec<&str> {
        self.path.split('/').filter(|s| !s.is_empty()).collect()
    }
    /// The host without a leading `www.`, `m.` or `player.`.
    pub fn site_host(&self) -> &str {
        let h = self.host.as_str();
        h.strip_prefix("www.").or_else(|| h.strip_prefix("m.")).unwrap_or(h)
    }
}

/// Take a link apart. None for anything that isn't one: no `scheme://`,
/// no host, spaces, or a user name or password in it.
pub fn parse(s: &str) -> Option<Url> {
    let s = s.trim();
    if s.is_empty() || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (scheme, rest) = s.split_once("://")?;
    if scheme.is_empty() || !scheme.starts_with(|c: char| c.is_ascii_alphabetic()) || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    // A sign-in name or password in a link would be kept in the deck.
    if authority.contains('@') {
        return None;
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p.parse::<u16>().ok()?),
            None if after.is_empty() => None,
            None => return None,
        };
        (format!("[{h}]"), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.parse::<u16>().ok()?)),
            None => (authority.to_string(), None),
        }
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() || host.starts_with('.') || host.ends_with("..") || !host.chars().all(|c| c.is_alphanumeric() || "-.[]:".contains(c)) {
        return None;
    }
    let tail = &rest[end..];
    let (before, fragment) = tail.split_once('#').unwrap_or((tail, ""));
    let (path, query) = before.split_once('?').unwrap_or((before, ""));
    Some(Url { scheme: scheme.to_ascii_lowercase(), host, port, path: path.to_string(), query: query.to_string(), fragment: fragment.to_string() })
}

/// The site a video link is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Site {
    Youtube { id: String },
    /// `hash`: the private part of an unlisted video's link.
    Vimeo { id: String, hash: Option<String> },
    /// Any other site: the link must be a video file or an HLS stream.
    Web,
}

impl Site {
    pub fn kind(&self) -> Option<LinkKind> {
        match self {
            Site::Youtube { .. } => Some(LinkKind::Youtube),
            Site::Vimeo { .. } => Some(LinkKind::Vimeo),
            Site::Web => None,
        }
    }
}

fn youtube_id(s: &str) -> Option<String> {
    (s.len() == 11 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then(|| s.to_string())
}

fn vimeo_id(s: &str) -> Option<String> {
    (!s.is_empty() && s.len() <= 12 && s.chars().all(|c| c.is_ascii_digit())).then(|| s.to_string())
}

/// Which site a link is on: YouTube (youtube.com, youtu.be,
/// youtube-nocookie.com) and Vimeo (vimeo.com, player.vimeo.com) videos
/// by their usual link shapes; anything else, including YouTube and Vimeo
/// pages that aren't one video, is `Web`.
pub fn site(u: &Url) -> Site {
    let host = u.site_host();
    let seg = u.segments();
    match host {
        "youtu.be" => seg.first().and_then(|s| youtube_id(s)).map(|id| Site::Youtube { id }).unwrap_or(Site::Web),
        "youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            let id = match seg.as_slice() {
                ["watch", ..] => u.param("v").and_then(|v| youtube_id(&v)),
                ["embed" | "shorts" | "live" | "v" | "e", id, ..] => youtube_id(id),
                _ => None,
            };
            id.map(|id| Site::Youtube { id }).unwrap_or(Site::Web)
        }
        "vimeo.com" | "player.vimeo.com" => {
            let (id, hash) = match seg.as_slice() {
                ["video", id, ..] => (vimeo_id(id), None),
                [id, hash] if vimeo_id(hash).is_none() && hash.chars().all(|c| c.is_ascii_hexdigit()) => (vimeo_id(id), Some(hash.to_string())),
                [id] => (vimeo_id(id), None),
                // channels/name/123, groups/name/videos/123, showcase/1/video/123
                [.., last] if matches!(seg[0], "channels" | "groups" | "showcase" | "album") => (vimeo_id(last), None),
                _ => (None, None),
            };
            let hash = hash.or_else(|| u.param("h")).filter(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()));
            id.map(|id| Site::Vimeo { id, hash }).unwrap_or(Site::Web)
        }
        _ => Site::Web,
    }
}

/// Check a link as text: an https link, and which site. `Http` for an
/// http link, `NotLink` for anything else that isn't https.
pub fn classify(s: &str) -> Result<(Url, Site), LinkError> {
    let u = parse(s).ok_or(LinkError::NotLink)?;
    match u.scheme.as_str() {
        "https" => {}
        "http" => return Err(LinkError::Http),
        _ => return Err(LinkError::NotLink),
    }
    let site = site(&u);
    Ok((u, site))
}

/// Whether the text is an https link (Insert is dimmed until it is).
pub fn is_https_link(s: &str) -> bool {
    parse(s).is_some_and(|u| u.scheme == "https")
}

/// What kind a link is from its text alone: the site's card, an HLS
/// stream by its `.m3u8` path, otherwise a file. Checking the link over
/// the network knows better (a live stream, a stream without `.m3u8`).
pub fn guess_kind(u: &Url, site: &Site) -> LinkKind {
    if let Some(k) = site.kind() {
        return k;
    }
    if u.path.to_ascii_lowercase().ends_with(".m3u8") {
        LinkKind::Stream
    } else {
        LinkKind::File
    }
}

/// A start time written into the link: YouTube's `t=1m30s`, `t=90` or
/// `start=90`, Vimeo's `#t=90s`.
pub fn link_start(u: &Url) -> Option<f64> {
    let t = u.param("t").or_else(|| u.param("start")).or_else(|| u.fragment.strip_prefix("t=").map(str::to_string))?;
    let t = t.trim();
    if let Ok(s) = t.trim_end_matches('s').parse::<f64>() {
        return (s.is_finite() && s > 0.0).then_some(s);
    }
    // 1h2m3s
    let (mut total, mut num) = (0.0, String::new());
    for c in t.chars() {
        match c {
            '0'..='9' | '.' => num.push(c),
            'h' | 'm' | 's' => {
                let n: f64 = num.parse().ok()?;
                total += n * match c {
                    'h' => 3600.0,
                    'm' => 60.0,
                    _ => 1.0,
                };
                num.clear();
            }
            _ => return None,
        }
    }
    (num.is_empty() && total > 0.0).then_some(total)
}

/// The name shown for where a web video comes from: "YouTube", "Vimeo",
/// or the link's host without `www.`.
pub fn site_name(link: &VideoLink) -> String {
    match link.kind {
        LinkKind::Youtube => "YouTube".into(),
        LinkKind::Vimeo => "Vimeo".into(),
        _ => parse(&link.url).map(|u| u.site_host().to_string()).unwrap_or_default(),
    }
}

/// The site's own embed player for a card, starting at `start` seconds
/// and stopping at `end`. `origin`: the page that holds the player (the
/// YouTube player asks for it to answer its JavaScript API).
pub fn embed_url(link: &VideoLink, start: f64, end: Option<f64>, autoplay: bool, origin: Option<&str>) -> Option<String> {
    let u = parse(&link.url)?;
    let start = start.max(0.0).floor() as u64;
    match site(&u) {
        Site::Youtube { id } => {
            let mut q = vec![format!("autoplay={}", autoplay as u8), "playsinline=1".into(), "rel=0".into(), "enablejsapi=1".into()];
            if start > 0 {
                q.push(format!("start={start}"));
            }
            if let Some(e) = end.filter(|e| e.is_finite() && *e > 0.0) {
                q.push(format!("end={}", e.ceil() as u64));
            }
            if let Some(o) = origin {
                q.push(format!("origin={}", encode(o)));
            }
            Some(format!("https://www.youtube-nocookie.com/embed/{id}?{}", q.join("&")))
        }
        Site::Vimeo { id, hash } => {
            let mut q = vec![format!("autoplay={}", autoplay as u8), "dnt=1".into()];
            if let Some(h) = hash {
                q.insert(0, format!("h={h}"));
            }
            let t = if start > 0 { format!("#t={start}s") } else { String::new() };
            Some(format!("https://player.vimeo.com/video/{id}?{}{t}", q.join("&")))
        }
        Site::Web => None,
    }
}

/// Percent-encode for a query value (RFC 3986 unreserved stay as they are).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3 / 2);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-decode (and `+` as a space, as forms write it).
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                let hex = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
                out.push(hex(b[i + 1]) * 16 + hex(b[i + 2]));
                i += 3;
                continue;
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(s: &str) -> Result<Site, LinkError> {
        classify(s).map(|(_, s)| s)
    }

    #[test]
    fn https_only() {
        assert_eq!(kind("http://example.com/a.mp4"), Err(LinkError::Http));
        assert_eq!(kind("HTTP://example.com/a.mp4"), Err(LinkError::Http));
        assert_eq!(kind("ftp://example.com/a.mp4"), Err(LinkError::NotLink));
        assert_eq!(kind("example.com/a.mp4"), Err(LinkError::NotLink));
        assert_eq!(kind("https://"), Err(LinkError::NotLink));
        assert_eq!(kind("https://user:pw@example.com/a.mp4"), Err(LinkError::NotLink), "no sign-in details kept in a deck");
        assert_eq!(kind("https://exa mple.com/a.mp4"), Err(LinkError::NotLink));
        assert_eq!(kind("  https://example.com/a.mp4 \n"), Ok(Site::Web), "spaces around are trimmed");
        assert!(is_https_link("https://cdn.example.com:8443/v.m3u8?x=1#y"));
        assert!(!is_https_link("https:/example.com"));
    }

    #[test]
    fn parts() {
        let u = parse("https://CDN.Example.com:8443/a/b.MP4?x=1&y=a%20b#frag").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str(), u.fragment.as_str()), ("cdn.example.com", Some(8443), "/a/b.MP4", "frag"));
        assert_eq!(u.param("y").as_deref(), Some("a b"));
        assert_eq!(guess_kind(&u, &Site::Web), LinkKind::File);
        let s = parse("https://example.com/live/index.M3U8").unwrap();
        assert_eq!(guess_kind(&s, &Site::Web), LinkKind::Stream);
        assert_eq!(parse("https://[::1]:9000/v.mp4").unwrap().host, "[::1]");
    }

    #[test]
    fn youtube_and_vimeo() {
        let yt = |id: &str| Ok(Site::Youtube { id: id.into() });
        assert_eq!(kind("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s"), yt("dQw4w9WgXcQ"));
        assert_eq!(kind("https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ"), yt("dQw4w9WgXcQ"));
        assert_eq!(kind("https://youtu.be/dQw4w9WgXcQ?si=abc"), yt("dQw4w9WgXcQ"));
        assert_eq!(kind("https://www.youtube.com/shorts/dQw4w9WgXcQ"), yt("dQw4w9WgXcQ"));
        assert_eq!(kind("https://www.youtube.com/embed/dQw4w9WgXcQ"), yt("dQw4w9WgXcQ"));
        assert_eq!(kind("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ"), yt("dQw4w9WgXcQ"));
        assert_eq!(kind("https://www.youtube.com/@somechannel"), Ok(Site::Web), "a channel is a page");
        assert_eq!(kind("https://www.youtube.com/watch?v=short"), Ok(Site::Web));
        assert_eq!(kind("https://vimeo.com/76979871"), Ok(Site::Vimeo { id: "76979871".into(), hash: None }));
        assert_eq!(kind("https://vimeo.com/76979871/a1b2c3d4e5"), Ok(Site::Vimeo { id: "76979871".into(), hash: Some("a1b2c3d4e5".into()) }));
        assert_eq!(kind("https://player.vimeo.com/video/76979871?h=a1b2c3"), Ok(Site::Vimeo { id: "76979871".into(), hash: Some("a1b2c3".into()) }));
        assert_eq!(kind("https://vimeo.com/channels/staffpicks/76979871"), Ok(Site::Vimeo { id: "76979871".into(), hash: None }));
        assert_eq!(kind("https://vimeo.com/about"), Ok(Site::Web));
        assert_eq!(kind("http://youtu.be/dQw4w9WgXcQ"), Err(LinkError::Http), "even YouTube needs https");
    }

    #[test]
    fn start_in_the_link() {
        let t = |s: &str| link_start(&parse(s).unwrap());
        assert_eq!(t("https://youtu.be/dQw4w9WgXcQ?t=42"), Some(42.0));
        assert_eq!(t("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1m30s"), Some(90.0));
        assert_eq!(t("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1h0m5s"), Some(3605.0));
        assert_eq!(t("https://vimeo.com/76979871#t=12s"), Some(12.0));
        assert_eq!(t("https://vimeo.com/76979871"), None);
        assert_eq!(t("https://youtu.be/dQw4w9WgXcQ?t=abc"), None);
    }

    #[test]
    fn embeds_and_names() {
        let yt = VideoLink { url: "https://youtu.be/dQw4w9WgXcQ".into(), kind: LinkKind::Youtube };
        assert_eq!(
            embed_url(&yt, 12.7, Some(30.2), true, Some("https://app.zebra.zebra")).unwrap(),
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1&playsinline=1&rel=0&enablejsapi=1&start=12&end=31&origin=https%3A%2F%2Fapp.zebra.zebra"
        );
        let vm = VideoLink { url: "https://vimeo.com/76979871/a1b2c3".into(), kind: LinkKind::Vimeo };
        assert_eq!(embed_url(&vm, 5.0, None, true, None).unwrap(), "https://player.vimeo.com/video/76979871?h=a1b2c3&autoplay=1&dnt=1#t=5s");
        assert_eq!(site_name(&yt), "YouTube");
        assert_eq!(site_name(&vm), "Vimeo");
        assert_eq!(site_name(&VideoLink { url: "https://www.example.org/v.mp4".into(), kind: LinkKind::File }), "example.org");
        let file = VideoLink { url: "https://example.org/v.mp4".into(), kind: LinkKind::File };
        assert_eq!(embed_url(&file, 0.0, None, true, None), None);
    }

    #[test]
    fn percent() {
        assert_eq!(encode("https://a.b/c?d=e f"), "https%3A%2F%2Fa.b%2Fc%3Fd%3De%20f");
        assert_eq!(decode("a%20b+c%2"), "a b c%2");
    }
}
