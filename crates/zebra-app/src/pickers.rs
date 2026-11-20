//! Open and save panels. On the Mac each one goes up as a sheet on its
//! document window right after the frame is drawn (see `sheets::run_picker`)
//! and its answer is polled: a modal panel run from inside the frame held
//! the window still until it came up, which showed as a lag after a click.
//! Elsewhere (and in tests) the panel runs where it is asked for.
use std::path::PathBuf;

use zebra_core::{Command, ElementId, Vec2};

use crate::app::ZebraApp;

pub const IMAGE_TYPES: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];
const STILL_TYPES: &[&str] = &["png", "jpg", "jpeg", "webp"];
const VIDEO_TYPES: &[&str] = &["mp4", "mov", "m4v"];
const AUDIO_TYPES: &[&str] = &["mp3", "m4a", "aac", "wav"];

/// What a panel is for, and so what happens with the file it gives.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    Open,
    Import,
    InsertImage(Vec2),
    ReplaceImage(ElementId),
    InsertVideo(Vec2),
    ReplaceVideo(ElementId),
    Soundtrack,
    Voiceover(usize),
    Poster(ElementId),
    ExportPdf,
    ExportPng,
    ExportHtml,
}

/// The panel a pick shows.
pub struct Panel {
    pub filter: String,
    pub types: &'static [&'static str],
    /// A save panel with this file name; None for an open panel.
    pub save_as: Option<String>,
}

impl Pick {
    pub fn panel(&self, app: &ZebraApp) -> Panel {
        let name = |ext: &str| Some(format!("{}.{ext}", crate::app::sanitize(&app.editor.doc.title)));
        let open = |filter: String, types| Panel { filter, types, save_as: None };
        match self {
            Pick::Open => open(crate::t!("app-file-filter-zebra"), &["zebra"]),
            Pick::Import => open(crate::t!("app-file-filter-import"), zebra_import::EXTENSIONS),
            Pick::InsertImage(_) | Pick::ReplaceImage(_) => open(crate::t!("app-file-filter-image"), IMAGE_TYPES),
            Pick::Poster(_) => open(crate::t!("app-file-filter-image"), STILL_TYPES),
            Pick::InsertVideo(_) | Pick::ReplaceVideo(_) => open(crate::t!("app-file-filter-video"), VIDEO_TYPES),
            Pick::Soundtrack | Pick::Voiceover(_) => open(crate::t!("app-file-filter-audio"), AUDIO_TYPES),
            Pick::ExportPdf => Panel { filter: crate::t!("app-file-filter-pdf"), types: &["pdf"], save_as: name("pdf") },
            Pick::ExportPng => Panel { filter: crate::t!("app-file-filter-png"), types: &["png"], save_as: name("png") },
            Pick::ExportHtml => Panel { filter: crate::t!("app-file-filter-html"), types: &["html"], save_as: name("html") },
        }
    }
}

/// Ask for a file for `pick`. On the Mac the panel goes up after this
/// frame; elsewhere it runs now.
pub fn ask(app: &mut ZebraApp, pick: Pick) {
    if app.native_dialogs && cfg!(target_os = "macos") {
        app.want_pick = Some(pick);
        app.want_pick_pass = app.ctx.cumulative_pass_nr();
        app.ctx.request_repaint();
        return;
    }
    let p = pick.panel(app);
    let d = rfd::FileDialog::new().add_filter(p.filter, p.types);
    let file = match p.save_as {
        Some(n) => d.set_file_name(n).save_file(),
        None => d.pick_file(),
    };
    if let Some(f) = file {
        finish(app, pick, f);
    }
}

/// Do what the pick was for with the file chosen.
pub fn finish(app: &mut ZebraApp, pick: Pick, p: PathBuf) {
    let path = || Some(p.to_string_lossy().to_string());
    match pick {
        // Through the action, so a file another window has open shows
        // that window, and a save in progress finishes first.
        Pick::Open => {
            app.queued.push(crate::actions::Action::OpenPath(p));
            app.ctx.request_repaint();
        }
        Pick::Import => crate::import_ui::start(app, p),
        Pick::InsertImage(at) => app.insert_image_file(&p, at),
        Pick::ReplaceImage(id) => {
            app.apply(Command::ReplaceImage { id, data_base64: String::new(), path: path() });
        }
        Pick::InsertVideo(at) => crate::media_ui::insert_video_file(app, &p, at),
        Pick::ReplaceVideo(id) => crate::media_ui::replace_video_file(app, &p, id),
        Pick::Soundtrack => crate::media_ui::set_soundtrack_file(app, &p),
        Pick::Voiceover(index) => {
            app.apply(Command::SetVoiceover { index, data_base64: String::new(), path: path(), asset: None });
        }
        Pick::Poster(id) => {
            app.apply(Command::SetPoster { id, data_base64: String::new(), path: path(), at: None });
        }
        Pick::ExportPdf => crate::files::export_pdf_to(app, with_ext(p, "pdf")),
        Pick::ExportPng => crate::files::export_png_to(app, with_ext(p, "png")),
        Pick::ExportHtml => crate::files::export_html_to(app, with_ext(p, "html")),
    }
}

fn with_ext(p: PathBuf, ext: &str) -> PathBuf {
    if p.extension().is_none() { p.with_extension(ext) } else { p }
}

/// For tests: the pick waiting to go up, if any.
pub fn waiting(app: &ZebraApp) -> Option<&Pick> {
    app.want_pick.as_ref()
}

