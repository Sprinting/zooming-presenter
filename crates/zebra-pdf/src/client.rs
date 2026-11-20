//! The app's side: helpers started on demand, one request at a time each,
//! watched for memory and time, killed and replaced when they misbehave.
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use zebra_core::AssetBytes;

use crate::limits::{self, Limits};
use crate::sys;
use crate::wire::{self, Fail, Reply, Request};

/// Which helper serves a request. Each lane has at most one helper
/// process, so there are never more than two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// File › Import and the API's `import`: a hostile file here never
    /// takes the pages on screen with it, and Cancel kills only this one.
    Import = 0,
    /// Pages drawn for the canvas, the presenter, captures and exports.
    Draw = 1,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PdfError {
    /// The helper program isn't where it should be (a broken install).
    #[error("the PDF helper is missing ({0})")]
    NoHelper(String),
    /// The helper couldn't load PDFium (a broken install).
    #[error("the PDF reader is missing ({0})")]
    NoEngine(String),
    #[error("the file is protected with a password")]
    Encrypted,
    #[error("the file is damaged or isn't a PDF")]
    Damaged,
    /// This page can't be had (no such page, or PDFium can't draw it);
    /// nothing else is wrong.
    #[error("the page can't be drawn")]
    Page,
    /// The helper used too much memory or time on it and was stopped (or
    /// did so on this PDF before, this session).
    #[error("the PDF needs too much memory or time")]
    TooDemanding,
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Io(String),
}

/// A page's picture for import.
#[derive(Debug, Clone)]
pub struct Preview {
    /// The page's size, points.
    pub w_pt: f64,
    pub h_pt: f64,
    /// The picture's size and RGB rows.
    pub px: (u32, u32),
    pub rgb: Vec<u8>,
    /// The page's first line of text (at most [`limits::TEXT_CHARS`]).
    pub text: String,
}

/// What helpers have cost so far (tests and measurements).
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub spawns: usize,
    /// Helpers killed or lost while working.
    pub deaths: usize,
    /// Most memory a helper was seen using.
    pub peak: u64,
}

struct Proc {
    child: Child,
    sock: UnixStream,
    /// The PDF open in it: key and page count.
    open: Option<(String, u32)>,
    buf: Vec<u8>,
    filled: usize,
    last: Instant,
}

struct LaneState {
    proc: Option<Proc>,
    /// The helper's process id, readable while a request holds the lane.
    pid: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Helpers lost in a row without a good answer between.
    deaths: u32,
    /// No new helper before this.
    not_before: Instant,
}

#[derive(Default)]
struct Health {
    bad_pages: BTreeSet<u32>,
    dead: bool,
}

/// Why waiting for an answer ended without one.
enum Lost {
    Cancelled,
    /// Over the deadline, over the budget, gone, or said something wrong.
    Gone(&'static str),
}

/// The helpers of this process ([`Pool::global`]), or a separate set
/// (tests).
pub struct Pool {
    limits: Limits,
    helper: Option<PathBuf>,
    lanes: [Mutex<LaneState>; 2],
    /// Each lane's helper's process id (0: none), without its lock.
    pids: [std::sync::Arc<std::sync::atomic::AtomicU32>; 2],
    health: Mutex<HashMap<String, Health>>,
    /// PDFs that couldn't be drawn sharper: ever reported, and not yet
    /// taken by the app.
    reported: Mutex<(HashSet<String>, Vec<String>)>,
    spawns: AtomicUsize,
    deaths: AtomicUsize,
    peak: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where the helper program is: beside the app's program
/// (Zebra.app/Contents/MacOS); debug builds and tests also take
/// `ZEBRA_PDF_HELPER`, and look one folder up (test programs live in
/// target/debug/deps, the helper in target/debug).
pub fn helper_path() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("ZEBRA_PDF_HELPER").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = "zebra-pdf-helper";
    let here = dir.join(name);
    if here.is_file() {
        return Some(here);
    }
    if cfg!(debug_assertions) {
        let up = dir.parent()?.join(name);
        return up.is_file().then_some(up);
    }
    None
}

impl Pool {
    /// The helpers every PDF in this process goes through.
    pub fn global() -> &'static Pool {
        static POOL: OnceLock<Pool> = OnceLock::new();
        POOL.get_or_init(|| Pool::new(Limits::default(), None))
    }

    /// A separate pool (tests): its own helpers, limits and memory of
    /// bad pages. `helper`: the program, else [`helper_path`].
    pub fn new(limits: Limits, helper: Option<PathBuf>) -> Pool {
        let pids: [std::sync::Arc<std::sync::atomic::AtomicU32>; 2] = Default::default();
        let lane = |i: usize| Mutex::new(LaneState { proc: None, pid: pids[i].clone(), deaths: 0, not_before: Instant::now() });
        Pool {
            limits,
            helper,
            lanes: [lane(0), lane(1)],
            pids: pids.clone(),
            health: Default::default(),
            reported: Default::default(),
            spawns: AtomicUsize::new(0),
            deaths: AtomicUsize::new(0),
            peak: AtomicU64::new(0),
        }
    }

    pub fn stats(&self) -> Stats {
        Stats { spawns: self.spawns.load(Ordering::Relaxed), deaths: self.deaths.load(Ordering::Relaxed), peak: self.peak.load(Ordering::Relaxed) }
    }

    /// Can PDFs be read here? Starts a helper to find out (and keeps it).
    pub fn check(&self) -> Result<(), PdfError> {
        let mut st = lock(&self.lanes[Lane::Draw as usize]);
        self.ready(&mut st, &|| false)
    }

    /// Open the PDF `bytes` (asset key `key`): its page count.
    pub fn open(&self, lane: Lane, key: &str, bytes: &AssetBytes, cancel: &dyn Fn() -> bool) -> Result<u32, PdfError> {
        self.call(lane, key, bytes, None, cancel).map(|(_, pages)| pages)
    }

    /// Sizes of pages `first..=last` (0-based), in points; (0, 0) for a
    /// page whose size can't be read.
    pub fn sizes(&self, lane: Lane, key: &str, bytes: &AssetBytes, first: u32, last: u32, cancel: &dyn Fn() -> bool) -> Result<Vec<(f64, f64)>, PdfError> {
        if last < first || last - first >= wire::MAX_SIZES {
            return Err(PdfError::Page);
        }
        let n = (last - first + 1) as usize;
        let ok = |r: &Reply| matches!(r, Reply::Sizes(v) if v.len() == n);
        let (r, _) = self.call(lane, key, bytes, Some((Request::Sizes { first, last }, None, self.limits.page, &ok)), cancel)?;
        let Reply::Sizes(v) = r else { return Err(PdfError::Page) };
        Ok(v.into_iter().map(|(w, h)| (w as f64, h as f64)).collect())
    }

    /// Page `index` (0-based) drawn `long_px` on its long side, with its
    /// size and first line of text.
    pub fn preview(&self, lane: Lane, key: &str, bytes: &AssetBytes, index: u32, long_px: u32, cancel: &dyn Fn() -> bool) -> Result<Preview, PdfError> {
        if long_px == 0 || long_px > limits::MAX_RECT_PX {
            return Err(PdfError::Page);
        }
        let ok = |r: &Reply| match r {
            Reply::Preview { px, rgb, w_pt, h_pt, .. } => {
                px.0 >= 1 && px.1 >= 1 && px.0.max(px.1) <= long_px && rgb.len() as u64 == px.0 as u64 * px.1 as u64 * 3 && *w_pt > 0.0 && *h_pt > 0.0
            }
            _ => false,
        };
        let (r, _) = self.call(lane, key, bytes, Some((Request::Preview { index, long_px }, Some(index), self.limits.page, &ok)), cancel)?;
        let Reply::Preview { w_pt, h_pt, px, text, rgb } = r else { return Err(PdfError::Page) };
        let text: String = text.chars().filter(|c| !c.is_control()).take(limits::TEXT_CHARS).collect();
        Ok(Preview { w_pt: w_pt as f64, h_pt: h_pt as f64, px, rgb, text })
    }

    /// `rect` (x, y, w, h) of page `index` drawn `page_px` big: straight
    /// RGBA rows, opaque.
    #[allow(clippy::too_many_arguments)]
    pub fn rect(
        &self,
        lane: Lane,
        key: &str,
        bytes: &AssetBytes,
        index: u32,
        page_px: (u32, u32),
        rect: (u32, u32, u32, u32),
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, PdfError> {
        let (_, _, w, h) = rect;
        if w == 0 || h == 0 || w > limits::MAX_RECT_PX || h > limits::MAX_RECT_PX || page_px.0 == 0 || page_px.1 == 0 || page_px.0 > limits::MAX_PAGE_PX.0 || page_px.1 > limits::MAX_PAGE_PX.1 {
            return Err(PdfError::Page);
        }
        let want = w as usize * h as usize * 4;
        let ok = |r: &Reply| matches!(r, Reply::Pixels(p) if p.len() == want);
        let (r, _) = self.call(lane, key, bytes, Some((Request::Rect { index, page_px, rect }, Some(index), self.limits.rect, &ok)), cancel)?;
        let Reply::Pixels(px) = r else { return Err(PdfError::Page) };
        Ok(px)
    }

    /// Stop `lane`'s helper now (an import is over, or cancelled).
    pub fn release(&self, lane: Lane) {
        let mut st = lock(&self.lanes[lane as usize]);
        st.stop();
    }

    /// The PDFs (asset keys) that `mine` claims and whose pages couldn't
    /// be drawn sharper since last asked: each PDF once per session.
    pub fn take_draw_failures(&self, mine: impl Fn(&str) -> bool) -> Vec<String> {
        let mut r = lock(&self.reported);
        let (take, keep): (Vec<String>, Vec<String>) = std::mem::take(&mut r.1).into_iter().partition(|k| mine(k));
        r.1 = keep;
        take
    }

    /// Has this PDF (or this page of it) been given up on this session?
    pub fn given_up(&self, key: &str, page: Option<u32>) -> bool {
        lock(&self.health).get(key).is_some_and(|h| h.dead || page.is_some_and(|p| h.bad_pages.contains(&p)))
    }

    /// Tests: the helper behaves badly on its next request (`hang`,
    /// `grow`, `exit`, `garbage`, `oversize`, `lie`). Debug helpers only.
    #[doc(hidden)]
    pub fn misbehave(&self, lane: Lane, what: &str) -> Result<(), PdfError> {
        let mut st = lock(&self.lanes[lane as usize]);
        self.ready(&mut st, &|| false)?;
        let p = st.proc.as_mut().ok_or_else(|| PdfError::Io("no helper".into()))?;
        let f = Request::Misbehave(what.to_string()).encode();
        p.sock.set_nonblocking(false).map_err(io)?;
        let r = (&p.sock).write_all_blocking(&f);
        p.sock.set_nonblocking(true).map_err(io)?;
        r.map_err(io)
    }

    /// The process id of `lane`'s helper, if one is running (tests).
    #[doc(hidden)]
    pub fn pid(&self, lane: Lane) -> Option<u32> {
        Some(self.pids[lane as usize].load(Ordering::SeqCst)).filter(|p| *p != 0)
    }

    fn give_up(&self, key: &str, page: Option<u32>, lane: Lane) {
        let mut h = lock(&self.health);
        let e = h.entry(key.to_string()).or_default();
        match page {
            Some(p) => {
                e.bad_pages.insert(p);
                if e.bad_pages.len() >= limits::KILLS_PER_PDF {
                    e.dead = true;
                }
            }
            None => e.dead = true,
        }
        drop(h);
        if lane == Lane::Draw {
            self.report(key);
        }
    }

    fn report(&self, key: &str) {
        let mut r = lock(&self.reported);
        if r.0.insert(key.to_string()) {
            r.1.push(key.to_string());
        }
    }

    /// A running helper for this lane: the one there, or a new one (after
    /// the backoff, if helpers have been lost in a row).
    fn ready(&self, st: &mut LaneState, cancel: &dyn Fn() -> bool) -> Result<(), PdfError> {
        if let Some(p) = &mut st.proc {
            let gone = !matches!(p.child.try_wait(), Ok(None));
            if gone || p.last.elapsed() > self.limits.idle {
                st.stop();
            }
        }
        if st.proc.is_none() {
            while Instant::now() < st.not_before {
                if cancel() {
                    return Err(PdfError::Cancelled);
                }
                std::thread::sleep(limits::POLL.min(st.not_before.saturating_duration_since(Instant::now())));
            }
            match self.spawn(cancel) {
                Ok(p) => {
                    st.pid.store(p.child.id(), Ordering::SeqCst);
                    st.proc = Some(p);
                }
                Err(e) => {
                    if e != PdfError::Cancelled {
                        self.lost(st, "it didn't start");
                    }
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn lost(&self, st: &mut LaneState, why: &str) {
        eprintln!("zebra: PDF helper stopped ({why})");
        st.stop();
        st.deaths = st.deaths.saturating_add(1);
        st.not_before = Instant::now() + limits::backoff(st.deaths);
        self.deaths.fetch_add(1, Ordering::Relaxed);
    }

    fn spawn(&self, cancel: &dyn Fn() -> bool) -> Result<Proc, PdfError> {
        let path = self.helper.clone().or_else(helper_path).ok_or_else(|| PdfError::NoHelper("zebra-pdf-helper not found".into()))?;
        let (ours, theirs) = UnixStream::pair().map_err(io)?;
        let longest = self.limits.open.max(self.limits.page).max(self.limits.rect) + limits::SELF_GRACE;
        let mut cmd = Command::new(&path);
        cmd.env_clear()
            .arg("--budget")
            .arg((self.limits.budget + limits::SELF_MARGIN).to_string())
            .arg("--deadline-ms")
            .arg(longest.as_millis().to_string())
            .arg("--idle-ms")
            .arg(limits::SELF_IDLE.max(self.limits.idle + Duration::from_secs(5)).as_millis().to_string())
            .stdin(Stdio::from(OwnedFd::from(theirs)))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        #[cfg(debug_assertions)]
        if let Some(p) = std::env::var_os("ZEBRA_PDFIUM") {
            cmd.env("ZEBRA_PDFIUM", p);
        }
        let child = cmd.spawn().map_err(|e| PdfError::NoHelper(format!("{}: {e}", path.display())))?;
        drop(cmd);
        self.spawns.fetch_add(1, Ordering::Relaxed);
        ours.set_nonblocking(true).map_err(io)?;
        // Room for a whole tile in the socket, so it isn't passed over in
        // 200 KB pieces (the system may give less).
        sys::buffer(&ours, false, 4 << 20);
        #[cfg(target_os = "macos")]
        {
            let one: libc::c_int = 1;
            // SAFETY: a valid socket and an int option.
            unsafe { libc::setsockopt(ours.as_raw_fd(), libc::SOL_SOCKET, libc::SO_NOSIGPIPE, &one as *const _ as *const libc::c_void, 4) };
        }
        let mut p = Proc { child, sock: ours, open: None, buf: vec![], filled: 0, last: Instant::now() };
        match self.receive(&mut p, Instant::now(), self.limits.open, cancel) {
            Ok(Reply::Hello { version }) if version == wire::VERSION => Ok(p),
            Ok(Reply::NoEngine(why)) => {
                end(p);
                Err(PdfError::NoEngine(why))
            }
            Err(Lost::Cancelled) => {
                end(p);
                Err(PdfError::Cancelled)
            }
            _ => {
                end(p);
                Err(PdfError::NoEngine("the PDF helper didn't start".into()))
            }
        }
    }

    /// Send `req` (after opening the PDF if the helper has another one
    /// open) and wait for an answer `ok` accepts. A helper that runs out
    /// of time or memory, dies or answers wrongly is killed, and the page
    /// (or, for an open, the PDF) is given up on for the session.
    #[allow(clippy::type_complexity)]
    fn call(
        &self,
        lane: Lane,
        key: &str,
        bytes: &AssetBytes,
        req: Option<(Request, Option<u32>, Duration, &dyn Fn(&Reply) -> bool)>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<(Reply, u32), PdfError> {
        let page = req.as_ref().and_then(|r| r.1);
        if self.given_up(key, page) {
            if lane == Lane::Draw {
                self.report(key);
            }
            return Err(PdfError::TooDemanding);
        }
        if key.len() > wire::MAX_KEY {
            return Err(PdfError::Damaged);
        }
        let mut st = lock_or_cancel(&self.lanes[lane as usize], cancel)?;
        self.ready(&mut st, cancel)?;
        if proc(&mut st)?.open.as_ref().is_none_or(|(k, _)| k != key) {
            let (open, file) = open_request(key, bytes)?;
            let p = proc(&mut st)?;
            p.open = None;
            match self.exchange(p, &open, file, self.limits.open, cancel) {
                Ok(Reply::Opened { pages }) => p.open = Some((key.to_string(), pages)),
                Ok(Reply::Failed(f)) => {
                    st.deaths = 0;
                    return Err(if f == Fail::Encrypted { PdfError::Encrypted } else { PdfError::Damaged });
                }
                Ok(_) => {
                    self.lost(&mut st, "a wrong answer");
                    self.give_up(key, None, lane);
                    return Err(PdfError::TooDemanding);
                }
                Err(Lost::Gone(why)) => {
                    self.lost(&mut st, why);
                    self.give_up(key, None, lane);
                    return Err(PdfError::TooDemanding);
                }
                Err(Lost::Cancelled) => return Err(self.cancelled(&mut st)),
            }
        }
        let p = proc(&mut st)?;
        let pages = p.open.as_ref().map(|o| o.1).unwrap_or(0);
        p.last = Instant::now();
        let Some((req, page, deadline, ok)) = req else {
            st.deaths = 0;
            return Ok((Reply::Opened { pages }, pages));
        };
        if page.is_some_and(|i| i >= pages) {
            return Err(PdfError::Page);
        }
        let got = self.exchange(p, &req, None, deadline, cancel);
        p.last = Instant::now();
        match got {
            Ok(Reply::Failed(_)) => {
                st.deaths = 0;
                Err(PdfError::Page)
            }
            Ok(r) if ok(&r) => {
                st.deaths = 0;
                Ok((r, pages))
            }
            Ok(_) => {
                self.lost(&mut st, "a wrong answer");
                self.give_up(key, page, lane);
                Err(PdfError::TooDemanding)
            }
            Err(Lost::Gone(why)) => {
                self.lost(&mut st, why);
                self.give_up(key, page, lane);
                Err(PdfError::TooDemanding)
            }
            Err(Lost::Cancelled) => Err(self.cancelled(&mut st)),
        }
    }

    /// Cancel kills the helper at once (it may be deep in a page); that
    /// is no fault of the PDF's, so nothing is given up on.
    fn cancelled(&self, st: &mut LaneState) -> PdfError {
        st.stop();
        PdfError::Cancelled
    }

    fn exchange(&self, p: &mut Proc, req: &Request, file: Option<std::fs::File>, deadline: Duration, cancel: &dyn Fn() -> bool) -> Result<Reply, Lost> {
        let t0 = Instant::now();
        let frame = req.encode();
        let mut sent = 0;
        let mut file = file;
        while sent < frame.len() {
            match sys::send(p.sock.as_raw_fd(), &frame[sent..], file.as_ref().map(|f| f.as_raw_fd())) {
                Ok(0) => return Err(Lost::Gone("closed")),
                Ok(n) => {
                    sent += n;
                    // The descriptor went with the first bytes; ours closes.
                    file = None;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    sys::wait(&p.sock, true, limits::POLL.as_millis() as i32);
                    self.watch(p, t0, deadline, cancel)?;
                }
                Err(_) => return Err(Lost::Gone("closed")),
            }
        }
        self.receive(p, t0, deadline, cancel)
    }

    fn receive(&self, p: &mut Proc, t0: Instant, deadline: Duration, cancel: &dyn Fn() -> bool) -> Result<Reply, Lost> {
        // `p.buf[..p.filled]` is what has come of the frame so far. The
        // buffer is kept between frames (tiles are the same size), so it
        // is neither zeroed nor grown again for each one; past 16 MB it
        // is given back.
        p.filled = 0;
        loop {
            let need = if p.filled < 4 {
                4
            } else {
                let n = u32::from_le_bytes([p.buf[0], p.buf[1], p.buf[2], p.buf[3]]) as usize;
                if n == 0 || n > wire::MAX_FRAME {
                    return Err(Lost::Gone("bad frame"));
                }
                4 + n
            };
            if p.filled == need && need > 4 {
                let r = Reply::decode(p.buf[4], &p.buf[5..need]).map_err(|_| Lost::Gone("bad frame"));
                p.filled = 0;
                if p.buf.len() > 16 << 20 {
                    p.buf = vec![];
                }
                return r;
            }
            if p.buf.len() < need {
                p.buf.resize(need, 0);
            }
            match (&p.sock).read(&mut p.buf[p.filled..need]) {
                Ok(0) => return Err(Lost::Gone("closed")),
                Ok(n) => p.filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if !sys::wait(&p.sock, false, limits::POLL.as_millis() as i32) {
                        self.watch(p, t0, deadline, cancel)?;
                    }
                }
                Err(_) => return Err(Lost::Gone("closed")),
            }
        }
    }

    /// Between waits: still within its time and memory, still there, not
    /// cancelled?
    fn watch(&self, p: &mut Proc, t0: Instant, deadline: Duration, cancel: &dyn Fn() -> bool) -> Result<(), Lost> {
        if cancel() {
            return Err(Lost::Cancelled);
        }
        if t0.elapsed() > deadline {
            return Err(Lost::Gone("deadline"));
        }
        if let Some(m) = sys::footprint(p.child.id()) {
            self.peak.fetch_max(m, Ordering::Relaxed);
            if m > self.limits.budget {
                return Err(Lost::Gone("budget"));
            }
        }
        if !matches!(p.child.try_wait(), Ok(None)) {
            return Err(Lost::Gone("exited"));
        }
        Ok(())
    }
}

trait WriteAllBlocking {
    fn write_all_blocking(self, b: &[u8]) -> std::io::Result<()>;
}

impl WriteAllBlocking for &UnixStream {
    fn write_all_blocking(mut self, b: &[u8]) -> std::io::Result<()> {
        std::io::Write::write_all(&mut self, b)
    }
}

/// The lane, once the request ahead of this one is done; Cancel doesn't
/// wait for it.
fn lock_or_cancel<'a>(m: &'a Mutex<LaneState>, cancel: &dyn Fn() -> bool) -> Result<MutexGuard<'a, LaneState>, PdfError> {
    loop {
        match m.try_lock() {
            Ok(g) => return Ok(g),
            Err(std::sync::TryLockError::Poisoned(e)) => return Ok(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {
                if cancel() {
                    return Err(PdfError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

impl LaneState {
    /// Kill the helper, if any, and wait for it.
    fn stop(&mut self) {
        self.pid.store(0, Ordering::SeqCst);
        if let Some(p) = self.proc.take() {
            end(p);
        }
    }
}

fn proc(st: &mut LaneState) -> Result<&mut Proc, PdfError> {
    st.proc.as_mut().ok_or_else(|| PdfError::Io("no helper".into()))
}

/// Kill a helper and wait for it (no zombie left).
fn end(mut p: Proc) {
    drop(p.sock);
    let _ = p.child.kill();
    let _ = p.child.wait();
}

fn io(e: std::io::Error) -> PdfError {
    PdfError::Io(e.to_string())
}

/// The Open request for a PDF and the file it travels in: the file the
/// bytes are mapped from (no copy), the bytes inside the request when
/// small, or else a temporary file of them, already unlinked.
fn open_request(key: &str, bytes: &AssetBytes) -> Result<(Request, Option<std::fs::File>), PdfError> {
    let len = bytes.len() as u64;
    if let Some(Ok((f, start, l))) = bytes.file_slice() {
        if f.metadata().map(|m| m.len() >= start + l).unwrap_or(false) {
            return Ok((Request::Open { key: key.to_string(), start, len: l, inline: None }, Some(f)));
        }
    }
    if bytes.len() <= wire::MAX_INLINE {
        return Ok((Request::Open { key: key.to_string(), start: 0, len, inline: Some(bytes.to_vec()) }, None));
    }
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("zebra-pdf-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let f = (|| {
        let mut f = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path)?;
        std::io::Write::write_all(&mut f, bytes)?;
        Ok::<_, std::io::Error>(f)
    })();
    let _ = std::fs::remove_file(&path);
    let f = f.map_err(io)?;
    Ok((Request::Open { key: key.to_string(), start: 0, len, inline: None }, Some(f)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_helper_is_an_error() {
        let pool = Pool::new(Limits::default(), Some("/nonexistent/zebra-pdf-helper".into()));
        let b = AssetBytes::from(b"%PDF-1.4".to_vec());
        assert!(matches!(pool.open(Lane::Draw, "k", &b, &|| false), Err(PdfError::NoHelper(_))));
    }
}
