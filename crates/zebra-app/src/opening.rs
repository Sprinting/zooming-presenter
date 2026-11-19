//! Opening a big file on a worker thread, with progress in the window.
//!
//! Reading, measuring and indexing a 200,000-element deck takes seconds.
//! A file whose document is big is prepared off the main thread (see
//! `Editor::prepare`); the window stays responsive and shows a small
//! progress panel until the document swaps in. Small files open at once.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

use egui::{Context, RichText};
use zebra_core::{Prepared, ZError, ZResult};

use crate::app::ZebraApp;

/// A document at least this big (`document.json`, uncompressed) opens on
/// a worker: about 5,000 elements, which take a moment to measure.
pub const BIG_DOCUMENT: u64 = 1 << 20;

/// A file being opened.
pub struct Opening {
    pub path: PathBuf,
    /// 0..=1000.
    progress: Arc<AtomicU32>,
    rx: Receiver<ZResult<Prepared>>,
}

impl Opening {
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }
}

impl Opening {
    /// An opening that stays at `progress` (the screen atlas).
    #[doc(hidden)]
    pub fn stalled(path: &Path, progress: f32) -> Opening {
        let (tx, rx) = std::sync::mpsc::channel();
        std::mem::forget(tx);
        Opening { path: path.to_path_buf(), progress: Arc::new(AtomicU32::new((progress * 1000.0) as u32)), rx }
    }
}

/// Should `path` open on a worker?
pub fn is_big(path: &Path) -> bool {
    zebra_core::io::document_size(path).is_some_and(|n| n >= BIG_DOCUMENT)
}

/// Start opening `path` on a worker thread.
pub fn start(app: &mut ZebraApp, path: &Path) {
    let progress = Arc::new(AtomicU32::new(0));
    let (tx, rx) = std::sync::mpsc::channel();
    let measure = app.renderer.measure();
    let (p, to) = (progress.clone(), path.to_path_buf());
    let ctx = app.ctx.clone();
    let viewport = app.viewport;
    let spawned = std::thread::Builder::new().name("zebra-open".into()).spawn(move || {
        let wake = || ctx.request_repaint_of(viewport);
        let set = |f: f32| {
            p.store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
            wake();
        };
        // Reading and parsing is about a third of the time; measuring the
        // text and indexing the rest.
        set(0.02);
        let r = zebra_core::io::load(&to).map(|doc| {
            set(0.35);
            zebra_core::Editor::prepare(doc, &*measure, &|f| set(0.35 + 0.65 * f))
        });
        let _ = tx.send(r);
        wake();
    });
    match spawned {
        Ok(_) => app.opening = Some(Opening { path: path.to_path_buf(), progress, rx }),
        // No thread: open it here.
        Err(_) => {
            let r = zebra_core::io::load(path).map(|doc| zebra_core::Editor::prepare(doc, &*app.editor.measure, &|_| {}));
            app.finish_open(path, r);
        }
    }
}

/// Swap the document in when it is ready.
pub fn poll(app: &mut ZebraApp) {
    let Some(o) = &app.opening else { return };
    let r = match o.rx.try_recv() {
        Err(TryRecvError::Empty) => return,
        Ok(r) => r,
        Err(TryRecvError::Disconnected) => Err(ZError::new("io", "opening stopped unexpectedly")),
    };
    let o = app.opening.take().expect("checked above");
    app.finish_open(&o.path, r);
}

/// The progress panel, over the window (it takes no input meanwhile).
pub fn ui(app: &mut ZebraApp, ctx: &Context) {
    let Some(o) = &app.opening else { return };
    let name = o.path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let progress = o.progress();
    egui::Modal::new(app.uid("opening")).show(ctx, |ui| {
        ui.set_width(280.0);
        ui.label(RichText::new(crate::t!("app-opening", name = name.as_str())).strong());
        ui.add_space(8.0);
        ui.add(egui::ProgressBar::new(progress).desired_height(6.0));
    });
}
