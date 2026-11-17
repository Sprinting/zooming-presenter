//! Export, autosave sidecars and the debug screenshot hook.
use std::path::{Path, PathBuf};

use crate::app::ZebraApp;

const AUTOSAVE_SECS: f64 = 30.0;

pub fn short(s: &str, n: usize) -> String {
    zebra_core::model::truncate(s, n)
}

pub fn export_pdf(app: &mut ZebraApp) {
    let name = format!("{}.pdf", crate::app::sanitize(&app.editor.doc.title));
    let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name(name).save_file() else { return };
    match app.renderer.export_pdf(&app.editor) {
        Ok(bytes) => match std::fs::write(&p, bytes) {
            Ok(()) => app.toast(format!("Exported {}", p.display())),
            Err(e) => app.toast(format!("Export failed: {e}")),
        },
        Err(e) => app.toast(format!("Export failed: {}", e.message)),
    }
}

pub fn export_png(app: &mut ZebraApp) {
    let name = format!("{}.png", crate::app::sanitize(&app.editor.doc.title));
    let Some(p) = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(name).save_file() else { return };
    let vp = app.editor.view.viewport;
    let (w, h) = ((vp.x * 2.0) as u32, (vp.y * 2.0) as u32);
    match app.renderer.capture(&app.editor, &zebra_render::CaptureTarget::Viewport, w.max(1), h.max(1)) {
        Ok(bytes) => match std::fs::write(&p, bytes) {
            Ok(()) => app.toast(format!("Exported {}", p.display())),
            Err(e) => app.toast(format!("Export failed: {e}")),
        },
        Err(e) => app.toast(format!("Export failed: {}", e.message)),
    }
}

/// Sidecar next to the file: `talk.zebra` → `.talk.zebra.autosave`.
pub fn autosave_path(file: &Path) -> PathBuf {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    file.with_file_name(format!(".{name}.autosave"))
}

fn untitled_autosave() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("Zebra").join("Untitled.zebra.autosave"))
}

pub fn remove_autosave(file: &Path) {
    let _ = std::fs::remove_file(autosave_path(file));
}

pub fn remove_autosave_untitled() {
    if let Some(p) = untitled_autosave() {
        let _ = std::fs::remove_file(p);
    }
}

/// A sidecar newer than the file, if any (offer to restore it).
pub fn autosave_newer(file: &Path) -> Option<PathBuf> {
    let a = autosave_path(file);
    let am = std::fs::metadata(&a).ok()?.modified().ok()?;
    let fm = std::fs::metadata(file).ok()?.modified().ok()?;
    (am > fm).then_some(a)
}

pub fn autosave_tick(app: &mut ZebraApp) {
    let now = app.now();
    if !app.editor.dirty() || app.editor.revision == app.autosaved_rev || now - app.last_autosave < AUTOSAVE_SECS {
        return;
    }
    if app.editor.in_transaction() || app.editor.doc.elements.is_empty() {
        return;
    }
    app.last_autosave = now;
    app.autosaved_rev = app.editor.revision;
    let path = match &app.editor.file_path {
        Some(f) => Some(autosave_path(f)),
        None => untitled_autosave(),
    };
    if let Some(p) = path {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = zebra_core::io::write_file(&app.editor.doc, &p);
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
