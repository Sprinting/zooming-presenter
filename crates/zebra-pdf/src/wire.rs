//! What the app and the helper say to each other: length-prefixed binary
//! frames, written and read by hand (no serde), every length checked.
//!
//! A frame is a little-endian `u32` length (of what follows), a tag byte
//! and the fields. The app never trusts a frame from the helper: a PDFium
//! bug could be exploited from a crafted file, so lengths are capped,
//! sizes must be finite and pixels must be exactly as many as asked for
//! (the client checks those against its request).
use std::io::Read;

/// Most bytes after the length: one 4096² RGBA rect and change.
pub const MAX_FRAME: usize = 4096 * 4096 * 4 + 4096;
/// Longest asset key.
pub const MAX_KEY: usize = 256;
/// Longest page text sent back (bytes; the client cuts it to 120 chars).
pub const MAX_TEXT: usize = 1024;
/// Most pages a PDF may claim.
pub const MAX_PAGES: u32 = 1_000_000;
/// Most page sizes in one reply.
pub const MAX_SIZES: u32 = 100_000;
/// Bytes of a PDF sent inside an Open frame (bigger ones go as a file).
pub const MAX_INLINE: usize = 16 * 1024 * 1024;
/// Version of this protocol, in the helper's hello.
pub const VERSION: u32 = 1;

mod tag {
    pub const OPEN: u8 = 1;
    pub const SIZES: u8 = 2;
    pub const PREVIEW: u8 = 3;
    pub const RECT: u8 = 4;
    pub const MISBEHAVE: u8 = 9;
    pub const HELLO: u8 = 100;
    pub const NO_ENGINE: u8 = 101;
    pub const OPENED: u8 = 102;
    pub const FAILED: u8 = 103;
    pub const SIZES_R: u8 = 104;
    pub const PREVIEW_R: u8 = 105;
    pub const PIXELS: u8 = 106;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Open a PDF (its file comes with the frame, or `inline`), keyed by
    /// asset key; `start`/`len` is where it is in that file.
    Open { key: String, start: u64, len: u64, inline: Option<Vec<u8>> },
    /// Sizes of pages `first..=last` (0-based), in points.
    Sizes { first: u32, last: u32 },
    /// Page `index` drawn `long_px` on its long side, white behind, and
    /// its first line of text.
    Preview { index: u32, long_px: u32 },
    /// `rect` (x, y, w, h) of page `index` drawn `page_px` big.
    Rect { index: u32, page_px: (u32, u32), rect: (u32, u32, u32, u32) },
    /// Tests only (a debug helper obeys it, a release one ignores it):
    /// behave badly on the next request.
    Misbehave(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    Encrypted = 1,
    Damaged = 2,
    /// The page can't be had (out of range, PDFium said no); the helper
    /// is fine.
    Page = 3,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Hello { version: u32 },
    /// PDFium couldn't be bound: why.
    NoEngine(String),
    Opened { pages: u32 },
    Failed(Fail),
    Sizes(Vec<(f32, f32)>),
    Preview { w_pt: f32, h_pt: f32, px: (u32, u32), text: String, rgb: Vec<u8> },
    Pixels(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WireError {
    #[error("frame too long ({0} bytes)")]
    TooLong(usize),
    #[error("frame cut short")]
    Short,
    #[error("unknown frame {0}")]
    Unknown(u8),
    #[error("bad field: {0}")]
    Bad(&'static str),
}

struct W(Vec<u8>);

impl W {
    fn new(tag: u8) -> W {
        W(vec![0, 0, 0, 0, tag])
    }
    fn u32(mut self, v: u32) -> W {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(mut self, v: u64) -> W {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(mut self, v: f32) -> W {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(self, b: &[u8]) -> W {
        let mut w = self.u32(b.len() as u32);
        w.0.extend_from_slice(b);
        w
    }
    fn done(mut self) -> Vec<u8> {
        let n = (self.0.len() - 4) as u32;
        self.0[..4].copy_from_slice(&n.to_le_bytes());
        self.0
    }
}

struct R<'a>(&'a [u8]);

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if self.0.len() < n {
            return Err(WireError::Short);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| WireError::Short)?))
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| WireError::Short)?))
    }
    fn f32(&mut self) -> Result<f32, WireError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().map_err(|_| WireError::Short)?))
    }
    fn bytes(&mut self, max: usize) -> Result<&'a [u8], WireError> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(WireError::Bad("length"));
        }
        self.take(n)
    }
    fn text(&mut self, max: usize) -> Result<String, WireError> {
        String::from_utf8(self.bytes(max)?.to_vec()).map_err(|_| WireError::Bad("text"))
    }
    fn end(&self) -> Result<(), WireError> {
        if self.0.is_empty() { Ok(()) } else { Err(WireError::Bad("trailing bytes")) }
    }
}

/// A size in points from the helper: finite and not negative (0: not
/// known), else refused.
fn points(v: f32) -> Result<f32, WireError> {
    if (0.0..1e7).contains(&v) { Ok(v) } else { Err(WireError::Bad("size")) }
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Request::Open { key, start, len, inline } => {
                let w = W::new(tag::OPEN).bytes(key.as_bytes()).u64(*start).u64(*len);
                match inline {
                    Some(b) => w.u32(1).bytes(b),
                    None => w.u32(0),
                }
                .done()
            }
            Request::Sizes { first, last } => W::new(tag::SIZES).u32(*first).u32(*last).done(),
            Request::Preview { index, long_px } => W::new(tag::PREVIEW).u32(*index).u32(*long_px).done(),
            Request::Rect { index, page_px, rect } => {
                W::new(tag::RECT).u32(*index).u32(page_px.0).u32(page_px.1).u32(rect.0).u32(rect.1).u32(rect.2).u32(rect.3).done()
            }
            Request::Misbehave(s) => W::new(tag::MISBEHAVE).bytes(s.as_bytes()).done(),
        }
    }

    pub fn decode(tag: u8, body: &[u8]) -> Result<Request, WireError> {
        let mut r = R(body);
        let q = match tag {
            tag::OPEN => {
                let key = r.text(MAX_KEY)?;
                let (start, len) = (r.u64()?, r.u64()?);
                let inline = match r.u32()? {
                    0 => None,
                    1 => Some(r.bytes(MAX_INLINE)?.to_vec()),
                    _ => return Err(WireError::Bad("inline")),
                };
                Request::Open { key, start, len, inline }
            }
            tag::SIZES => Request::Sizes { first: r.u32()?, last: r.u32()? },
            tag::PREVIEW => Request::Preview { index: r.u32()?, long_px: r.u32()? },
            tag::RECT => Request::Rect { index: r.u32()?, page_px: (r.u32()?, r.u32()?), rect: (r.u32()?, r.u32()?, r.u32()?, r.u32()?) },
            tag::MISBEHAVE => Request::Misbehave(r.text(64)?),
            t => return Err(WireError::Unknown(t)),
        };
        r.end()?;
        Ok(q)
    }
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Reply::Hello { version } => W::new(tag::HELLO).u32(*version).done(),
            Reply::NoEngine(s) => W::new(tag::NO_ENGINE).bytes(&s.as_bytes()[..s.len().min(MAX_TEXT)]).done(),
            Reply::Opened { pages } => W::new(tag::OPENED).u32(*pages).done(),
            Reply::Failed(f) => W::new(tag::FAILED).u32(*f as u32).done(),
            Reply::Sizes(v) => {
                let mut w = W::new(tag::SIZES_R).u32(v.len() as u32);
                for (a, b) in v {
                    w = w.f32(*a).f32(*b);
                }
                w.done()
            }
            Reply::Preview { w_pt, h_pt, px, text, rgb } => {
                W::new(tag::PREVIEW_R).f32(*w_pt).f32(*h_pt).u32(px.0).u32(px.1).bytes(text.as_bytes()).bytes(rgb).done()
            }
            Reply::Pixels(p) => W::new(tag::PIXELS).bytes(p).done(),
        }
    }

    pub fn decode(tag: u8, body: &[u8]) -> Result<Reply, WireError> {
        let mut r = R(body);
        let a = match tag {
            tag::HELLO => Reply::Hello { version: r.u32()? },
            tag::NO_ENGINE => Reply::NoEngine(String::from_utf8_lossy(r.bytes(MAX_TEXT)?).into_owned()),
            tag::OPENED => {
                let pages = r.u32()?;
                if pages > MAX_PAGES {
                    return Err(WireError::Bad("pages"));
                }
                Reply::Opened { pages }
            }
            tag::FAILED => Reply::Failed(match r.u32()? {
                1 => Fail::Encrypted,
                2 => Fail::Damaged,
                3 => Fail::Page,
                _ => return Err(WireError::Bad("failure")),
            }),
            tag::SIZES_R => {
                let n = r.u32()?;
                if n > MAX_SIZES || n as usize * 8 != r.0.len() {
                    return Err(WireError::Bad("sizes"));
                }
                let mut v = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    v.push((points(r.f32()?)?, points(r.f32()?)?));
                }
                Reply::Sizes(v)
            }
            tag::PREVIEW_R => {
                let (w_pt, h_pt) = (points(r.f32()?)?, points(r.f32()?)?);
                let px = (r.u32()?, r.u32()?);
                let text = r.text(MAX_TEXT)?;
                let rgb = r.bytes(MAX_FRAME)?.to_vec();
                Reply::Preview { w_pt, h_pt, px, text, rgb }
            }
            tag::PIXELS => Reply::Pixels(r.bytes(MAX_FRAME)?.to_vec()),
            t => return Err(WireError::Unknown(t)),
        };
        r.end()?;
        Ok(a)
    }
}

/// A whole frame: its tag, its body and the bytes it took.
pub type Frame<'a> = (u8, &'a [u8], usize);

/// Split one whole frame off the front of `buf`: (tag, body, bytes
/// used), or None while it is still arriving. A length over
/// [`MAX_FRAME`] is an error at once, before any of it is waited for.
pub fn split(buf: &[u8]) -> Result<Option<Frame<'_>>, WireError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let n = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if n > MAX_FRAME {
        return Err(WireError::TooLong(n));
    }
    if n == 0 {
        return Err(WireError::Short);
    }
    if buf.len() < 4 + n {
        return Ok(None);
    }
    Ok(Some((buf[4], &buf[5..4 + n], 4 + n)))
}

/// Read one frame from a blocking stream (the helper's side).
pub fn read_frame(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_le_bytes(len) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad frame length"));
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    let tag = body[0];
    body.remove(0);
    Ok((tag, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let qs = [
            Request::Open { key: "abc".into(), start: 5, len: 9, inline: None },
            Request::Open { key: "k".into(), start: 0, len: 3, inline: Some(vec![1, 2, 3]) },
            Request::Sizes { first: 0, last: 9 },
            Request::Preview { index: 3, long_px: 512 },
            Request::Rect { index: 1, page_px: (1024, 768), rect: (0, 512, 512, 256) },
            Request::Misbehave("hang".into()),
        ];
        for q in qs {
            let f = q.encode();
            let (t, b, used) = split(&f).unwrap().unwrap();
            assert_eq!(used, f.len());
            assert_eq!(Request::decode(t, b).unwrap(), q);
        }
        let rs = [
            Reply::Hello { version: VERSION },
            Reply::NoEngine("x".into()),
            Reply::Opened { pages: 3 },
            Reply::Failed(Fail::Encrypted),
            Reply::Sizes(vec![(1.0, 2.0), (0.0, 0.0)]),
            Reply::Preview { w_pt: 1.0, h_pt: 2.0, px: (1, 1), text: "é".into(), rgb: vec![1, 2, 3] },
            Reply::Pixels(vec![9; 16]),
        ];
        for a in rs {
            let f = a.encode();
            let (t, b, _) = split(&f).unwrap().unwrap();
            assert_eq!(Reply::decode(t, b).unwrap(), a);
        }
    }

    /// Whatever the helper sends, reading it never panics: cut short,
    /// lying lengths, wrong tags, bad numbers.
    #[test]
    fn hostile_frames_are_errors() {
        assert_eq!(split(&[0xff, 0xff, 0xff, 0xff, 1]), Err(WireError::TooLong(u32::MAX as usize)));
        assert_eq!(split(&[0, 0, 0, 0]), Err(WireError::Short));
        assert_eq!(split(&[9, 0, 0]), Ok(None));
        let good = Reply::Preview { w_pt: 1.0, h_pt: 2.0, px: (1, 1), text: "t".into(), rgb: vec![1, 2, 3] }.encode();
        for cut in 5..good.len() {
            let body = &good[5..cut];
            assert!(Reply::decode(good[4], body).is_err(), "cut at {cut}");
        }
        assert!(matches!(Reply::decode(200, &[]), Err(WireError::Unknown(200))));
        let nan = W::new(tag::SIZES_R).u32(1).f32(f32::NAN).f32(1.0).done();
        assert!(Reply::decode(nan[4], &nan[5..]).is_err());
        let lie = W::new(tag::SIZES_R).u32(1000).done();
        assert!(Reply::decode(lie[4], &lie[5..]).is_err());
        let pages = W::new(tag::OPENED).u32(u32::MAX).done();
        assert!(Reply::decode(pages[4], &pages[5..]).is_err());
        let text = W::new(tag::PREVIEW_R).f32(1.0).f32(1.0).u32(1).u32(1).bytes(&[0xff, 0xfe]).bytes(&[]).done();
        assert!(Reply::decode(text[4], &text[5..]).is_err());
        let extra = { let mut f = Reply::Opened { pages: 1 }.encode(); f.push(0); f[0] += 1; f };
        assert!(Reply::decode(extra[4], &extra[5..]).is_err());
        // Random bytes.
        let mut seed = 1u64;
        for _ in 0..2000 {
            let mut b = vec![0u8; (seed % 40) as usize];
            for x in b.iter_mut() {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                *x = (seed >> 33) as u8;
            }
            let _ = Reply::decode(b.first().copied().unwrap_or(0).wrapping_add(100), &b);
            let _ = Request::decode(b.first().copied().unwrap_or(0) % 10, &b);
            let _ = split(&b);
        }
    }
}
