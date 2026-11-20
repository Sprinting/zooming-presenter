//! Headless mode: the same server and dispatcher, no window.
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use zebra_core::Editor;

use crate::{ApiOptions, ApiServer, Dispatcher};

static SIGNALLED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    SIGNALLED.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    // SAFETY: the handler only stores to an atomic (async-signal-safe).
    unsafe {
        let h = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, h);
        libc::signal(libc::SIGTERM, h);
        libc::signal(libc::SIGHUP, h);
    }
}

pub(crate) fn run(opts: ApiOptions, open: Option<PathBuf>) -> anyhow::Result<()> {
    install_signal_handlers();
    let stop = AtomicBool::new(false);
    run_loop(opts, open, &stop, true, |server| {
        let ready = json!({
            "pid": std::process::id(),
            "socket": server.socket_path(),
            "token": server.token(),
            "version": env!("CARGO_PKG_VERSION"),
            "api_version": zebra_core::API_VERSION,
        });
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "zebra headless: API listening on {}", server.socket_path().display());
        let _ = writeln!(out, "ZEBRA_API_READY {ready}");
        let _ = out.flush();
    })
}

fn run_loop(
    opts: ApiOptions,
    open: Option<PathBuf>,
    stop: &AtomicBool,
    watch_signals: bool,
    ready: impl FnOnce(&ApiServer),
) -> anyhow::Result<()> {
    let signal = Arc::new((Mutex::new(false), Condvar::new()));
    let s2 = signal.clone();
    let waker: crate::Waker = Arc::new(move || {
        let (m, c) = &*s2;
        *m.lock().unwrap() = true;
        c.notify_all();
    });

    let renderer = Arc::new(zebra_render::Renderer::new());
    // Imported PDF pages keep only a small picture: captures and exports
    // draw them from the PDF, as in the app.
    renderer.set_page_raster(Some(Arc::new(|r: &zebra_render::pages::PageRequest| {
        zebra_import::pdf::raster(&r.pdf, &r.bytes, r.index, r.page_px, r.rect)
    })));
    let mut editor = Editor::default();
    editor.measure = renderer.measure();
    if let Some(p) = open {
        let doc = zebra_core::io::load(&p).map_err(|e| anyhow::anyhow!("cannot open {}: {e}", p.display()))?;
        editor.load(doc, Some(p));
    }
    let server = ApiServer::start_headless(opts, waker)?;
    let mut dispatcher = Dispatcher::new(renderer);
    dispatcher.headless = true;
    ready(&server);

    let t0 = Instant::now();
    loop {
        let animating = editor.view.tick(t0.elapsed().as_secs_f64());
        dispatcher.pump(&server, &mut editor);
        let _ = dispatcher.take_events();
        if dispatcher.quit_requested || stop.load(Ordering::SeqCst) || (watch_signals && SIGNALLED.load(Ordering::SeqCst)) {
            break;
        }
        let busy = animating || editor.view.flight.is_some() || dispatcher.has_pending();
        let timeout = if busy { Duration::from_millis(16) } else { Duration::from_millis(250) };
        let (m, c) = &*signal;
        let mut woken = m.lock().unwrap();
        if !*woken {
            woken = c.wait_timeout(woken, timeout).unwrap().0;
        }
        *woken = false;
    }
    // A save still being written finishes before the process ends.
    editor.wait_save();
    // Give the quit reply a moment to flush before the socket closes.
    std::thread::sleep(Duration::from_millis(50));
    drop(server);
    Ok(())
}

/// An in-process headless instance on its own thread (tests, embedding).
pub struct HeadlessHandle {
    pub socket: PathBuf,
    pub token: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<anyhow::Result<()>>>,
}

impl HeadlessHandle {
    pub fn spawn(opts: ApiOptions, open: Option<PathBuf>) -> anyhow::Result<HeadlessHandle> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let stop2 = stop.clone();
        let thread = std::thread::Builder::new().name("zebra-headless".into()).spawn(move || {
            let tx2 = tx.clone();
            let r = run_loop(opts, open, &stop2, false, move |s| {
                let _ = tx2.send(Ok((s.socket_path(), s.token().to_string())));
            });
            if let Err(e) = &r {
                let _ = tx.send(Err(anyhow::anyhow!("{e}")));
            }
            r
        })?;
        let (socket, token) = rx.recv()??;
        Ok(HeadlessHandle { socket, token, stop, thread: Some(thread) })
    }

    /// Stop the loop and wait for it (removes socket and discovery file).
    pub fn stop(mut self) -> anyhow::Result<()> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> anyhow::Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        // Poke the loop so it notices promptly.
        let _ = std::os::unix::net::UnixStream::connect(&self.socket);
        match self.thread.take() {
            Some(t) => t.join().map_err(|_| anyhow::anyhow!("headless thread panicked"))?,
            None => Ok(()),
        }
    }

    /// Finished on its own (e.g. a client called `quit`)?
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}

impl Drop for HeadlessHandle {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}
