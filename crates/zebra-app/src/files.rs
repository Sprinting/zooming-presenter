//! Export, autosave sidecars and the debug screenshot hook.
use std::path::{Path, PathBuf};

use crate::app::ZebraApp;

const AUTOSAVE_SECS: f64 = 30.0;

pub fn short(s: &str, n: usize) -> String {
    zebra_core::model::truncate(s, n)
}

pub fn export_pdf(app: &mut ZebraApp) {
    let name = format!("{}.pdf", crate::app::sanitize(&app.editor.doc.title));
    let Some(p) = rfd::FileDialog::new().add_filter(crate::t!("app-file-filter-pdf"), &["pdf"]).set_file_name(name).save_file() else { return };
    export_in_background(app, p, |r, ed| r.export_pdf(ed));
}

pub fn export_png(app: &mut ZebraApp) {
    let name = format!("{}.png", crate::app::sanitize(&app.editor.doc.title));
    let Some(p) = rfd::FileDialog::new().add_filter(crate::t!("app-file-filter-png"), &["png"]).set_file_name(name).save_file() else { return };
    let vp = app.editor.view.viewport;
    let (w, h) = (((vp.x * 2.0) as u32).max(1), ((vp.y * 2.0) as u32).max(1));
    export_in_background(app, p, move |r, ed| r.capture(ed, &zebra_render::CaptureTarget::Viewport, w, h));
}

/// The exported page's control bar, in the person's language.
pub fn web_labels() -> zebra_web::export::Labels {
    zebra_web::export::Labels {
        previous: crate::t!("present-hud-previous-step"),
        next: crate::t!("present-hud-next-step"),
        full_screen: crate::t!("web-full-screen"),
        step_of: crate::t!("present-hud-step-of", step = "{step}", count = "{count}"),
        overview: crate::t!("present-hud-overview", count = "{count}"),
        play: crate::t!("web-play"),
        pause: crate::t!("web-pause"),
        unsupported: crate::t!("web-unsupported"),
    }
}

/// File > Export > Web Page: one .html that presents the deck in a
/// browser (a folder when its video and audio are big). Written on a
/// worker; a sheet then offers to open it in the default browser.
pub fn export_html(app: &mut ZebraApp) {
    let name = format!("{}.html", crate::app::sanitize(&app.editor.doc.title));
    let Some(p) = rfd::FileDialog::new().add_filter(crate::t!("app-file-filter-html"), &["html"]).set_file_name(name).save_file() else { return };
    let mut snap = zebra_core::Editor::new(app.editor.doc.clone());
    snap.view = app.editor.view.clone();
    snap.measure = app.renderer.measure();
    let renderer = app.renderer.clone();
    let labels = web_labels();
    let ctx = app.ctx.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("zebra-export".into()).spawn(move || {
        let _ = tx.send(zebra_web::export::export(&renderer, &snap, &labels, &p).map_err(|e| e.message));
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => {
            app.media.web_exports.push(rx);
            app.toast(crate::t!("export-toast-exporting"));
        }
        Err(e) => app.toast(crate::t!("export-toast-failed", error = e.to_string())),
    }
}

/// Open an exported page in the default web browser (never inside Zebra).
pub fn open_in_browser(page: &Path) {
    #[cfg(target_os = "macos")]
    crate::mac::open_in_browser(page);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = std::process::Command::new("xdg-open").arg(page).spawn();
    }
}

/// Render a snapshot of the document on a worker and write it to `path`,
/// so a big export never stalls the window. The result shows as a toast.
pub fn export_in_background(
    app: &mut ZebraApp,
    path: PathBuf,
    render: impl FnOnce(&zebra_render::Renderer, &zebra_core::Editor) -> Result<Vec<u8>, zebra_core::ZError> + Send + 'static,
) {
    let mut snap = zebra_core::Editor::new(app.editor.doc.clone());
    snap.view = app.editor.view.clone();
    snap.measure = app.renderer.measure();
    let renderer = app.renderer.clone();
    let ctx = app.ctx.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("zebra-export".into()).spawn(move || {
        let msg = match render(&renderer, &snap) {
            Ok(bytes) => match std::fs::write(&path, bytes) {
                Ok(()) => crate::t!("export-toast-exported", path = path.display().to_string()),
                Err(e) => crate::t!("export-toast-failed", error = e.to_string()),
            },
            Err(e) => crate::t!("export-toast-failed", error = e.message.as_str()),
        };
        let _ = tx.send(msg);
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => {
            app.media.background.push(rx);
            app.toast(crate::t!("export-toast-exporting"));
        }
        Err(e) => app.toast(crate::t!("export-toast-failed", error = e.to_string())),
    }
}

/// Sidecar next to the file: `talk.zebra` → `.talk.zebra.autosave`.
pub fn autosave_path(file: &Path) -> PathBuf {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    file.with_file_name(format!(".{name}.autosave"))
}

/// Where autosaves of untitled documents live: one file per window, so
/// several untitled windows never overwrite each other. In app_dir, so
/// tests can point it at a scratch folder (ZEBRA_DATA_DIR).
pub fn untitled_dir() -> PathBuf {
    zebra_api::discovery::app_dir().join("Autosave")
}

/// Spill new media to `Autosave/Media` (see `zebra_core::media::spill`),
/// after sweeping what a crashed run left there.
pub fn start_spilling() {
    zebra_core::media::set_spill_dir(Some(untitled_dir().join("Media")));
    zebra_core::media::sweep_spill(zebra_api::discovery::pid_alive);
}

/// A fresh name for a window's untitled autosave.
pub fn new_autosave_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("Untitled-{t:x}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

/// The process that owns an untitled autosave, from its name
/// (`Untitled-<time>-<pid>-<n>`). None for names from older builds.
fn autosave_owner(p: &Path) -> Option<u32> {
    let name = p.file_name()?.to_string_lossy().to_string();
    let mut parts = name.strip_suffix(".zebra.autosave")?.strip_prefix("Untitled-")?.split('-');
    let (_time, pid, _n) = (parts.next()?, parts.next()?, parts.next()?);
    pid.parse().ok()
}

/// Take over a leftover autosave: it gets a name of this process, so
/// another copy of Zebra started later doesn't open it as well. Returns
/// the new name (without the extension).
pub fn adopt_autosave(path: &Path) -> String {
    let name = new_autosave_name();
    let to = untitled_dir().join(format!("{name}.zebra.autosave"));
    if path.parent() == Some(untitled_dir().as_path()) && std::fs::rename(path, &to).is_ok() {
        return name;
    }
    let old = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    old.strip_suffix(".zebra.autosave").unwrap_or(&old).to_string()
}

fn untitled_autosave(app: &ZebraApp) -> PathBuf {
    untitled_dir().join(format!("{}.zebra.autosave", app.autosave_name))
}

pub fn remove_autosave(file: &Path) {
    let _ = std::fs::remove_file(autosave_path(file));
}

/// Forget this window's untitled autosave (saved, or the person chose
/// Don't Save).
pub fn remove_autosave_untitled(app: &ZebraApp) {
    app.autosave_job.cancel();
    let _ = std::fs::remove_file(untitled_autosave(app));
}

/// The person chose Don't Save: their unsaved changes are meant to go,
/// so the autosave holding them goes too.
pub fn discard_autosave(app: &ZebraApp) {
    app.autosave_job.cancel();
    match &app.editor.file_path {
        Some(f) => remove_autosave(f),
        None => remove_autosave_untitled(app),
    }
}

/// Untitled documents left from the last run (it crashed, or was killed
/// before the person chose Save or Don't Save), oldest first. An autosave
/// from before there was one per window is moved in with the rest.
pub fn leftover_untitled() -> Vec<PathBuf> {
    let dir = untitled_dir();
    let legacy = zebra_api::discovery::app_dir().join("Untitled.zebra.autosave");
    if legacy.is_file() {
        let _ = std::fs::create_dir_all(&dir);
        // Owner 0: no running Zebra has it open.
        let _ = std::fs::rename(&legacy, dir.join("Untitled-0-0-0.zebra.autosave"));
    }
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".zebra.autosave"))
        // Not the ones a running Zebra is still writing (this one, or
        // another copy): those are open windows, not leftovers.
        .filter(|p| !autosave_owner(p).is_some_and(zebra_api::discovery::pid_alive))
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    v.sort();
    v.into_iter().map(|(_, p)| p).collect()
}

/// A sidecar newer than the file, if any (offer to restore it).
pub fn autosave_newer(file: &Path) -> Option<PathBuf> {
    let a = autosave_path(file);
    let am = std::fs::metadata(&a).ok()?.modified().ok()?;
    let fm = std::fs::metadata(file).ok()?.modified().ok()?;
    (am > fm).then_some(a)
}

/// Seconds until an autosave is due, if one is pending; the frame loop
/// schedules a single wake for it so an idle window still autosaves.
pub fn autosave_due_in(app: &ZebraApp) -> Option<f64> {
    if !app.editor.dirty() || app.editor.revision == app.autosaved_rev || app.editor.doc.elements.is_empty() {
        return None;
    }
    Some((AUTOSAVE_SECS - (app.now() - app.last_autosave)).max(0.5))
}

pub fn autosave_tick(app: &mut ZebraApp) {
    let now = app.now();
    if !app.editor.dirty() || app.editor.revision == app.autosaved_rev || now - app.last_autosave < AUTOSAVE_SECS {
        return;
    }
    if app.editor.in_transaction() || app.editor.doc.elements.is_empty() {
        return;
    }
    // Not while the document itself is being saved: that save is about
    // to make the autosave unneeded.
    if app.editor.saving().is_some() {
        return;
    }
    // One at a time: a deck with large video can take longer to write
    // than the time between two autosaves.
    use std::sync::atomic::Ordering;
    if app.autosave_job.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    app.last_autosave = now;
    app.autosaved_rev = app.editor.revision;
    let p = match &app.editor.file_path {
        Some(f) => autosave_path(f),
        None => untitled_autosave(app),
    };
    // Written on a worker from a copy (pictures and video are shared, not
    // copied): writing every asset here stalled the window on big decks.
    let doc = app.editor.doc.clone();
    let job = app.autosave_job.clone();
    let epoch = job.epoch.load(Ordering::SeqCst);
    let write = move || {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = zebra_core::io::write_file(&doc, &p);
        // Saved or discarded while this was being written: it is stale.
        if job.epoch.load(Ordering::SeqCst) != epoch {
            let _ = std::fs::remove_file(&p);
        }
        job.busy.store(false, Ordering::SeqCst);
    };
    if std::thread::Builder::new().name("zebra-autosave".into()).spawn(write).is_err() {
        app.autosave_job.busy.store(false, Ordering::SeqCst);
        app.autosaved_rev = 0;
    }
}

/// The autosave being written in the background, if any.
#[derive(Default)]
pub struct AutosaveJob {
    busy: std::sync::atomic::AtomicBool,
    /// Bumped when the document is saved or its changes discarded: an
    /// autosave still being written then removes its own file.
    epoch: std::sync::atomic::AtomicU64,
}

impl AutosaveJob {
    pub fn cancel(&self) {
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn busy(&self) -> bool {
        self.busy.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// `--screenshot-after <secs> <png>`: capture egui's frame, save, quit.
pub fn debug_screenshot(app: &mut ZebraApp, ctx: &egui::Context) {
    let Some((secs, path)) = app.screenshot.clone() else { return };
    if app.screenshot_requested {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        return;
    }
    if app.now() < secs {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        return;
    }
    app.screenshot_requested = true;
    app.force_quit = true;
    let c = ctx.clone();
    ctx.request_screenshot(move |img| {
        let [w, h] = img.size;
        let mut buf = Vec::with_capacity(w * h * 4);
        for px in &img.pixels {
            buf.extend_from_slice(&px.to_srgba_unmultiplied());
        }
        if let Some(rgba) = image::RgbaImage::from_raw(w as u32, h as u32, buf) {
            if let Err(e) = rgba.save(&path) {
                eprintln!("screenshot failed: {e}");
            } else {
                eprintln!("screenshot saved to {}", path.display());
            }
        }
        c.send_viewport_cmd(egui::ViewportCommand::Close);
    });
}
