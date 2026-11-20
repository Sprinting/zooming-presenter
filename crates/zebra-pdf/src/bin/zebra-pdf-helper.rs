//! zebra-pdf-helper: reads and draws PDF pages for Zebra, in a process of
//! its own, so whatever PDFium does with a file (12 GB for a 3.5 KB one,
//! an abort, a hang) never takes the app with it.
//!
//! Zebra starts it with one end of a socket as its standard input and
//! talks to it in frames ([`zebra_pdf::wire`]): open a PDF (its file
//! arrives over the socket, so the helper opens no paths of its own),
//! page sizes, a page's preview and first line, a part of a page. One
//! request at a time.
//!
//! The app watches it and kills it over its memory budget or a deadline.
//! It also watches itself (a thread below) in case the app can't: past
//! the budget (plus a margin) or a deadline (plus a grace), or once the
//! app is gone, it ends at once. With nothing to do for a while it
//! leaves. Not meant to be run by hand.
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use pdfium_render::prelude::*;
use zebra_pdf::limits;
use zebra_pdf::sys;
use zebra_pdf::wire::{self, Fail, Reply, Request};

/// When the request under way started (ms since launch, plus one), or 0.
static BUSY_SINCE: AtomicU64 = AtomicU64::new(0);

struct Args {
    budget: u64,
    deadline: Duration,
    idle: Duration,
}

fn args() -> Args {
    let mut a = Args { budget: limits::BUDGET + limits::SELF_MARGIN, deadline: limits::OPEN_DEADLINE + limits::SELF_GRACE, idle: limits::SELF_IDLE };
    let v: Vec<String> = std::env::args().skip(1).collect();
    for w in v.windows(2) {
        let n = w[1].parse::<u64>().ok();
        match (w[0].as_str(), n) {
            ("--budget", Some(n)) => a.budget = n,
            ("--deadline-ms", Some(n)) => a.deadline = Duration::from_millis(n),
            ("--idle-ms", Some(n)) => a.idle = Duration::from_millis(n),
            _ => {}
        }
    }
    a
}

/// Where PDFium is: Zebra.app/Contents/Frameworks beside this program's
/// folder on the Mac, this program's folder elsewhere; debug builds also
/// take `ZEBRA_PDFIUM`.
fn library_path() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("ZEBRA_PDFIUM").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = Pdfium::pdfium_platform_library_name();
    let p = if cfg!(target_os = "macos") { dir.parent()?.join("Frameworks").join(name) } else { dir.join(name) };
    p.is_file().then_some(p)
}

/// A PDF as PDFium reads it: from the file the app passed (only
/// `start..start+len` of it), or bytes sent inside the request.
enum Source {
    File { file: std::fs::File, start: u64, len: u64, pos: u64 },
    Memory(std::io::Cursor<Vec<u8>>),
}

impl Read for Source {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Source::File { file, start, len, pos } => {
                let left = len.saturating_sub(*pos);
                let n = (buf.len() as u64).min(left) as usize;
                if n == 0 {
                    return Ok(0);
                }
                let got = file.read_at(&mut buf[..n], *start + *pos)?;
                *pos += got as u64;
                Ok(got)
            }
            Source::Memory(c) => c.read(buf),
        }
    }
}

impl Seek for Source {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        match self {
            Source::File { len, pos, .. } => {
                let p = match to {
                    SeekFrom::Start(n) => n as i128,
                    SeekFrom::End(d) => *len as i128 + d as i128,
                    SeekFrom::Current(d) => *pos as i128 + d as i128,
                };
                if p < 0 {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "before the start"));
                }
                *pos = p as u64;
                Ok(*pos)
            }
            Source::Memory(c) => c.seek(to),
        }
    }
}

fn main() {
    let a = args();
    // SAFETY: Zebra starts this program with its socket as standard input;
    // nothing else in this process uses descriptor 0.
    let sock = unsafe { UnixStream::from_raw_fd(0) };
    let t0 = Instant::now();
    watchdog(a.budget, a.deadline, t0);
    let _ = sock.set_read_timeout(Some(a.idle));
    let lib = library_path().ok_or_else(|| "libpdfium not found".to_string()).and_then(|p| Pdfium::bind_to_library(&p).map_err(|e| e.to_string()));
    let pdfium: &'static Pdfium = match lib {
        Ok(b) => Box::leak(Box::new(Pdfium::new(b))),
        Err(e) => {
            let _ = (&sock).write_all(&Reply::NoEngine(e).encode());
            return;
        }
    };
    if (&sock).write_all(&Reply::Hello { version: wire::VERSION }.encode()).is_err() {
        return;
    }
    let mut doc: Option<PdfDocument<'static>> = None;
    let mut misbehave: Option<String> = None;
    loop {
        let mut fds = vec![];
        // The app gone, or nothing asked for a while: leave.
        let Ok((tag, body)) = read_frame(&sock, &mut fds) else { return };
        let Ok(req) = Request::decode(tag, &body) else { return };
        drop(body);
        if let Request::Misbehave(m) = req {
            if cfg!(debug_assertions) {
                misbehave = Some(m);
            }
            continue;
        }
        BUSY_SINCE.store(t0.elapsed().as_millis() as u64 + 1, Ordering::SeqCst);
        if let Some(m) = misbehave.take() {
            if let Some(bad) = act_badly(&m, &req) {
                let _ = (&sock).write_all(&bad);
                BUSY_SINCE.store(0, Ordering::SeqCst);
                continue;
            }
        }
        let reply = handle(pdfium, &mut doc, req, fds);
        BUSY_SINCE.store(0, Ordering::SeqCst);
        if (&sock).write_all(&reply.encode()).is_err() {
            return;
        }
    }
}

/// One frame, with any descriptor sent along with it.
fn read_frame(sock: &UnixStream, fds: &mut Vec<OwnedFd>) -> std::io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        let n = sys::recv(sock.as_raw_fd(), &mut head[got..], fds)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        got += n;
    }
    let n = u32::from_le_bytes(head) as usize;
    if n == 0 || n > wire::MAX_FRAME {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut body = vec![0u8; n];
    let mut got = 0;
    while got < n {
        let k = sys::recv(sock.as_raw_fd(), &mut body[got..], fds)?;
        if k == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        got += k;
    }
    let tag = body.remove(0);
    Ok((tag, body))
}

fn handle(pdfium: &'static Pdfium, doc: &mut Option<PdfDocument<'static>>, req: Request, mut fds: Vec<OwnedFd>) -> Reply {
    match req {
        Request::Open { start, len, inline, .. } => {
            // The last PDF goes first (its memory and its file).
            *doc = None;
            let src = match inline {
                Some(b) => Source::Memory(std::io::Cursor::new(b)),
                None => match fds.pop() {
                    Some(fd) => Source::File { file: std::fs::File::from(fd), start, len, pos: 0 },
                    None => return Reply::Failed(Fail::Damaged),
                },
            };
            drop(fds);
            match pdfium.load_pdf_from_reader(src, None) {
                Ok(d) => {
                    let pages = (d.pages().len().max(0) as u32).min(wire::MAX_PAGES);
                    *doc = Some(d);
                    Reply::Opened { pages }
                }
                Err(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError | PdfiumInternalError::SecurityError)) => {
                    Reply::Failed(Fail::Encrypted)
                }
                Err(_) => Reply::Failed(Fail::Damaged),
            }
        }
        Request::Sizes { first, last } => {
            let Some(d) = doc.as_ref() else { return Reply::Failed(Fail::Damaged) };
            if last < first || last - first >= wire::MAX_SIZES {
                return Reply::Failed(Fail::Page);
            }
            let pages = d.pages();
            let v = (first..=last)
                .map(|i| {
                    let r = i32::try_from(i).ok().and_then(|i| pages.page_size(i).ok());
                    r.map(|r| (finite(r.width().value), finite(r.height().value))).unwrap_or((0.0, 0.0))
                })
                .collect();
            Reply::Sizes(v)
        }
        Request::Preview { index, long_px } => {
            let Some(d) = doc.as_ref() else { return Reply::Failed(Fail::Damaged) };
            if long_px == 0 || long_px > limits::MAX_RECT_PX {
                return Reply::Failed(Fail::Page);
            }
            let Some(page) = i32::try_from(index).ok().and_then(|i| d.pages().get(i).ok()) else { return Reply::Failed(Fail::Page) };
            let (w, h) = (finite(page.width().value), finite(page.height().value));
            if w <= 0.0 || h <= 0.0 {
                return Reply::Failed(Fail::Page);
            }
            let k = long_px as f64 / (w.max(h) as f64);
            let (pw, ph) = ((((w as f64) * k).round() as u32).clamp(1, long_px), (((h as f64) * k).round() as u32).clamp(1, long_px));
            let cfg = PdfRenderConfig::new().set_target_size(pw as i32, ph as i32).render_form_data(true).render_annotations(true);
            let Ok(bmp) = page.render_with_config(&cfg) else { return Reply::Failed(Fail::Page) };
            let (bw, bh) = (bmp.width().max(0) as u32, bmp.height().max(0) as u32);
            let rgba = bmp.as_rgba_bytes();
            if bw == 0 || bh == 0 || bw.max(bh) > long_px || rgba.len() != (bw * bh * 4) as usize {
                return Reply::Failed(Fail::Page);
            }
            let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            drop(rgba);
            Reply::Preview { w_pt: w, h_pt: h, px: (bw, bh), text: first_line(&page), rgb }
        }
        Request::Rect { index, page_px, rect } => {
            let Some(d) = doc.as_ref() else { return Reply::Failed(Fail::Damaged) };
            let (x, y, w, h) = rect;
            if w == 0 || h == 0 || w > limits::MAX_RECT_PX || h > limits::MAX_RECT_PX || page_px.0 > limits::MAX_PAGE_PX.0 || page_px.1 > limits::MAX_PAGE_PX.1 {
                return Reply::Failed(Fail::Page);
            }
            let Some(page) = i32::try_from(index).ok().and_then(|i| d.pages().get(i).ok()) else { return Reply::Failed(Fail::Page) };
            let Ok(mut bmp) = PdfBitmap::empty(w as i32, h as i32, PdfBitmapFormat::BGRA) else { return Reply::Failed(Fail::Page) };
            let (Ok(pw), Ok(ph), Ok(ox), Ok(oy)) = (i32::try_from(page_px.0), i32::try_from(page_px.1), i32::try_from(x), i32::try_from(y)) else {
                return Reply::Failed(Fail::Page);
            };
            let cfg = PdfRenderConfig::new().set_target_size(pw, ph).set_origin(-ox, -oy).render_form_data(true).render_annotations(true);
            if page.render_into_bitmap_with_config(&mut bmp, &cfg).is_err() {
                return Reply::Failed(Fail::Page);
            }
            let mut px = bmp.as_rgba_bytes();
            if px.len() != (w * h * 4) as usize {
                return Reply::Failed(Fail::Page);
            }
            // Opaque, like the page's picture.
            for p in px.chunks_exact_mut(4) {
                p[3] = 255;
            }
            Reply::Pixels(px)
        }
        Request::Misbehave(_) => Reply::Failed(Fail::Page),
    }
}

fn finite(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 && v < 1e7 { v } else { 0.0 }
}

/// The page's first line of text, for the picture's description.
fn first_line(page: &PdfPage) -> String {
    let all = page.text().map(|t| t.all()).unwrap_or_default();
    let line = all.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut s: String = line.chars().take(limits::TEXT_CHARS).collect();
    if s.len() < line.len() {
        s.push('…');
    }
    s
}

/// Tests: misbehave instead of answering (debug builds only). Some
/// never return; the others give the bytes to send.
fn act_badly(what: &str, req: &Request) -> Option<Vec<u8>> {
    if !cfg!(debug_assertions) {
        return None;
    }
    match what {
        "hang" => loop {
            std::thread::sleep(Duration::from_secs(1));
        },
        "spin" => {
            let mut x = 0u64;
            loop {
                x = std::hint::black_box(x.wrapping_add(1));
            }
        }
        "grow" => {
            let mut keep: Vec<Vec<u8>> = vec![];
            loop {
                keep.push(vec![1u8; 64 << 20]);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        "exit" => std::process::exit(3),
        "abort" => std::process::abort(),
        "garbage" => Some(vec![9, 0, 0, 0, 250, 1, 2, 3, 4, 5, 6, 7, 8]),
        "oversize" => Some(vec![0xff, 0xff, 0xff, 0x7f, 106]),
        "lie" => Some(match req {
            Request::Rect { rect, .. } => Reply::Pixels(vec![0; (rect.2 * rect.3 * 4) as usize / 2]).encode(),
            _ => Reply::Opened { pages: 7 }.encode(),
        }),
        _ => None,
    }
}

/// The helper's own watch, every 25 ms: over the budget, over the longest
/// deadline, or the app gone (a new parent, or the socket closed while
/// a request is under way): end now.
fn watchdog(budget: u64, deadline: Duration, t0: Instant) {
    let parent = sys::parent();
    let me = std::process::id();
    let _ = std::thread::Builder::new().name("watchdog".into()).spawn(move || loop {
        std::thread::sleep(limits::POLL);
        if sys::parent() != parent {
            sys::exit_now(72);
        }
        if sys::footprint(me).is_some_and(|m| m > budget) {
            sys::exit_now(70);
        }
        let busy = BUSY_SINCE.load(Ordering::SeqCst);
        if busy != 0 {
            let now = t0.elapsed().as_millis() as u64 + 1;
            if now.saturating_sub(busy) > deadline.as_millis() as u64 {
                sys::exit_now(71);
            }
            if app_gone() {
                sys::exit_now(73);
            }
        }
    });
}

/// While a request is under way the app sends nothing: the socket being
/// readable then can only mean it was closed.
fn app_gone() -> bool {
    let mut p = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd; descriptor 0 is the socket.
    if unsafe { libc::poll(&mut p, 1, 0) } <= 0 {
        return false;
    }
    let mut b = [0u8; 1];
    // SAFETY: peeking one byte into a one-byte buffer.
    let n = unsafe { libc::recv(0, b.as_mut_ptr() as *mut libc::c_void, 1, libc::MSG_PEEK | libc::MSG_DONTWAIT) };
    n == 0
}
