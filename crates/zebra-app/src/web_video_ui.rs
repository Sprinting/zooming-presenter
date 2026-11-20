//! Web videos (spec 18e): Insert › Web Video…, the inspector's Link and
//! Download rows, the presenter view's "Loading video…", and video cards
//! (YouTube, Vimeo) opening in their play window.
//!
//! * The sheet checks a link on a worker (never longer than 10 s) with
//!   [`zebra_media::link::Checker`], then takes a poster: a file's frame
//!   at 1 s on that worker, a stream's first frame from a hidden muted
//!   player on the main thread, a card's thumbnail from the check. The
//!   video lands mid-view as one undo step.
//! * Download streams the clip to a temporary file on a worker (progress
//!   and Cancel in the inspector), then hands its path to
//!   `Command::DownloadVideo`, which keeps it in the deck like a local
//!   video; the temporary file is deleted either way.
//! * Opening a deck touches no network: players open only to play,
//!   preview, or because the step before was reached while presenting.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

use base64::Engine;
use egui::{Color32, Pos2, RichText, Vec2 as EVec2};
use zebra_core::weblink::{self, LinkError};
use zebra_core::{Command, ElementId, ElementKind, LinkFound, LinkKind, Vec2};
use zebra_media::link::{Checked, DownloadError};
use zebra_media::VideoPlayer;

use crate::app::ZebraApp;
use crate::style::{self, Palette};
use crate::widgets as w;

/// Longest the sheet waits for a check (the checker stops itself at 10 s;
/// this is in case the worker can't).
const CHECK_LIMIT: f64 = 11.0;
/// Clipboard text longer than this is not a link worth offering.
const CLIP_MAX: usize = 4096;
/// Most characters the Link field takes (a pasted novel is not a link).
const FIELD_MAX: usize = 8192;

/// The Insert Web Video sheet.
pub struct Sheet {
    pub text: String,
    /// The red line under the field.
    pub error: Option<LinkError>,
    at: Vec2,
    width: f64,
    phase: Phase,
    focus: bool,
}

enum Phase {
    Editing,
    /// The worker checks the link (and takes a file's poster).
    Checking { url: String, rx: Receiver<Result<(Checked, Option<Vec<u8>>), LinkError>>, started: f64 },
    /// A stream: a hidden, muted player gives its first frame.
    Grabbing { url: String, checked: Checked, player: Box<dyn VideoPlayer>, deadline: f64 },
}

impl Sheet {
    pub fn checking(&self) -> bool {
        !matches!(self.phase, Phase::Editing)
    }
}

/// The message for a link error, from the strings file.
pub fn error_text(e: LinkError) -> String {
    match e {
        LinkError::NotLink => crate::t!("web-video-error-not-link"),
        LinkError::Password => crate::t!("web-video-error-password"),
        LinkError::Http => crate::t!("web-video-error-http"),
        LinkError::NotVideo => crate::t!("web-video-error-not-video"),
        LinkError::SignIn => crate::t!("web-video-error-sign-in"),
        LinkError::NotFound => crate::t!("web-video-error-not-found"),
        LinkError::Unreachable => crate::t!("web-video-error-unreachable"),
    }
}

/// What is wrong with the text as typed, before anything is checked: a
/// password in it, or http. Anything else waits for Insert.
pub fn live_error(text: &str) -> Option<LinkError> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if weblink::has_password(t) {
        return Some(LinkError::Password);
    }
    match weblink::classify(t) {
        Err(LinkError::Http) => Some(LinkError::Http),
        _ => None,
    }
}

/// Insert is dimmed until the field holds an https link (and while it is
/// being checked).
pub fn can_insert(s: &Sheet) -> bool {
    !s.checking() && live_error(&s.text).is_none() && weblink::is_https_link(&s.text)
}

/// Insert › Web Video…: the sheet, with the clipboard's link in the field
/// when it holds one.
pub fn open(app: &mut ZebraApp) {
    let clip = clipboard_text();
    open_with(app, clip);
}

#[cfg(not(test))]
fn clipboard_text() -> Option<String> {
    #[cfg(not(target_os = "macos"))]
    if let Some(t) = TEST_CLIPBOARD.lock().ok().and_then(|g| g.clone()) {
        return Some(t);
    }
    arboard::Clipboard::new().ok()?.get_text().ok()
}

#[cfg(test)]
fn clipboard_text() -> Option<String> {
    None
}

/// Tests stand in a clipboard (there may be no system one).
#[cfg(not(target_os = "macos"))]
pub static TEST_CLIPBOARD: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The sheet with `clip` (the clipboard's text) in the field when it is a
/// link.
pub fn open_with(app: &mut ZebraApp, clip: Option<String>) {
    if app.web_sheet.is_some() {
        return;
    }
    let text = clip
        .map(|t| t.trim().to_string())
        .filter(|t| t.len() <= CLIP_MAX && weblink::parse(t).is_some_and(|u| u.scheme == "https" || u.scheme == "http"))
        .unwrap_or_default();
    let width = app.editor.view.viewport.x / app.editor.view.camera.zoom * 0.5;
    let error = live_error(&text);
    app.web_sheet = Some(Sheet { text, error, at: app.canvas_center_world(), width, phase: Phase::Editing, focus: true });
}

/// Insert: check the link on a worker.
fn start_check(app: &mut ZebraApp) {
    let Some(s) = app.web_sheet.as_mut() else { return };
    let url = s.text.trim().to_string();
    if let Err(e) = weblink::classify(&url) {
        s.error = Some(e);
        return;
    }
    s.error = None;
    let backend = app.media.backend.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = app.ctx.clone();
    let u = url.clone();
    let spawned = std::thread::Builder::new().name("zebra-web-check".into()).spawn(move || {
        let checker = zebra_media::link::Checker::from_env();
        let started = std::time::Instant::now();
        let r = checker.check(&u).map(|c| {
            // A file's frame at 1 s (its first when shorter), as a local
            // video's, in what is left of the check's time.
            let left = checker.timeout.saturating_sub(started.elapsed());
            let poster = match (&c.poster, c.kind) {
                (Some(p), _) => Some(p.clone()),
                (None, LinkKind::File) if !left.is_zero() => {
                    let at = if c.duration > 0.0 && c.duration < 1.0 { 0.0 } else { 1.0 };
                    backend.poster_png_url(&u, at, 1280, left).ok()
                }
                _ => None,
            };
            (c, poster)
        });
        let _ = tx.send(r);
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => s.phase = Phase::Checking { url, rx, started: app.editor.view.now },
        Err(_) => s.error = Some(LinkError::Unreachable),
    }
}

/// The check's answer, the hidden player's first frame.
fn poll(app: &mut ZebraApp) {
    let now = app.editor.view.now;
    let Some(mut s) = app.web_sheet.take() else { return };
    let phase = std::mem::replace(&mut s.phase, Phase::Editing);
    let busy = !matches!(phase, Phase::Editing);
    s.phase = match phase {
        Phase::Editing => Phase::Editing,
        Phase::Checking { url, rx, started } => match rx.try_recv() {
            Ok(Ok((checked, poster))) => {
                if poster.is_none() && matches!(checked.kind, LinkKind::Stream | LinkKind::Live) {
                    // A stream's first frame comes from a player.
                    match app.media.backend.open_video_url(&url) {
                        Ok(mut player) => {
                            player.set_muted(true);
                            player.play();
                            Phase::Grabbing { url, checked, player, deadline: started + CHECK_LIMIT - 1.0 }
                        }
                        Err(_) => {
                            insert(app, &s, &url, checked, None);
                            return;
                        }
                    }
                } else {
                    insert(app, &s, &url, checked, poster);
                    return;
                }
            }
            Ok(Err(e)) => {
                s.error = Some(e);
                Phase::Editing
            }
            Err(TryRecvError::Empty) if now - started < CHECK_LIMIT => {
                app.ctx.request_repaint_after(std::time::Duration::from_millis(100));
                Phase::Checking { url, rx, started }
            }
            Err(_) => {
                s.error = Some(LinkError::Unreachable);
                Phase::Editing
            }
        },
        Phase::Grabbing { url, checked, mut player, deadline } => {
            if let Some(f) = player.next_frame(1280) {
                drop(player);
                let png = zebra_media::encode_png(&f);
                insert(app, &s, &url, checked, Some(png));
                return;
            }
            match player.readiness() {
                zebra_media::Readiness::Failed(_) => {
                    s.error = Some(LinkError::NotVideo);
                    Phase::Editing
                }
                _ if now >= deadline => {
                    // It plays, but no frame came in time: the player shows
                    // one when it is played.
                    drop(player);
                    insert(app, &s, &url, checked, None);
                    return;
                }
                _ => {
                    app.ctx.request_repaint_after(std::time::Duration::from_millis(30));
                    Phase::Grabbing { url, checked, player, deadline }
                }
            }
        }
    };
    if busy && matches!(s.phase, Phase::Editing) {
        // The answer is a problem: back to the field to fix the link.
        s.focus = true;
    }
    app.web_sheet = Some(s);
}

/// The video lands mid-view at its own shape: one undo step.
fn insert(app: &mut ZebraApp, s: &Sheet, url: &str, c: Checked, poster: Option<Vec<u8>>) {
    let cmd = Command::AddWebVideo {
        url: url.to_string(),
        x: s.at.x,
        y: s.at.y,
        width: Some(s.width),
        alt: c.title.clone().unwrap_or_default(),
        poster_base64: poster.map(|p| base64::engine::general_purpose::STANDARD.encode(p)),
        autoplay: true,
        looping: false,
        muted: false,
        start: None,
        end: None,
        id: None,
        found: Some(LinkFound { kind: c.kind, width: c.width, height: c.height, duration: c.duration }),
    };
    if let Some(out) = app.apply(cmd) {
        app.select(out.created);
        app.view_cmd(zebra_core::ViewCommand::SetTool { tool: zebra_core::Tool::Select });
    }
}

/// The sheet, in the alert style of [`crate::sheets`], under the toolbar.
pub fn ui(app: &mut ZebraApp, ctx: &egui::Context) {
    poll(app);
    if app.web_sheet.is_none() {
        return;
    }
    let p = Palette::of(ctx);
    let content = ctx.content_rect();
    let top = content.top() + crate::mac::title_row(app) + crate::panels::TOOLBAR_ROW;
    let (block_id, sheet_id) = (app.uid("web-video-block"), app.uid("web-video-sheet"));
    let Some(s) = app.web_sheet.as_mut() else { return };
    #[derive(PartialEq)]
    enum Do {
        Nothing,
        Insert,
        Cancel,
    }
    let mut act = Do::Nothing;
    egui::Area::new(block_id).order(egui::Order::Foreground).fixed_pos(content.min).show(ctx, |ui| {
        let r = ui.allocate_rect(content, egui::Sense::click_and_drag());
        ui.painter().rect_filled(r.rect, 0.0, Color32::from_black_alpha(if p.dark { 40 } else { 14 }));
    });
    let width = 380.0;
    let enabled = can_insert(s);
    egui::Area::new(sheet_id)
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(content.center().x - width / 2.0 - style::SHEET_MARGIN, top + 2.0))
        .show(ctx, |ui| {
            style::sheet_frame(&p).show(ui, |ui| {
                ui.set_width(width);
                ui.add(egui::Label::new(RichText::new(crate::t!("web-video-sheet-title")).font(style::semibold(13.0)).color(p.text)));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::t!("web-video-link")).font(style::ui_font(13.0)).color(p.text));
                    let fw = ui.available_width();
                    let before = s.text.clone();
                    let r = ui.add_enabled_ui(!s.checking(), |ui| w::text_field(ui, &mut s.text, fw, &crate::t!("web-video-link-hint"))).inner;
                    if s.text.chars().count() > FIELD_MAX {
                        s.text = s.text.chars().take(FIELD_MAX).collect();
                    }
                    if s.focus {
                        r.request_focus();
                        s.focus = false;
                    }
                    if s.text != before {
                        // Typing clears the last answer; a password or http
                        // shows at once.
                        s.error = live_error(&s.text);
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    // Under the field, lined up with it.
                    let indent = ui.painter().layout_no_wrap(crate::t!("web-video-link"), style::ui_font(13.0), p.text).size().x + ui.spacing().item_spacing.x;
                    ui.add_space(indent);
                    match (&s.phase, s.error) {
                        (Phase::Checking { .. } | Phase::Grabbing { .. }, _) => {
                            ui.add(egui::Spinner::new().size(12.0).color(p.weak));
                            ui.label(RichText::new(crate::t!("web-video-checking")).font(style::ui_font(11.0)).color(p.weak));
                        }
                        (_, Some(e)) => {
                            ui.add(egui::Label::new(RichText::new(error_text(e)).font(style::ui_font(11.0)).color(style::RED)).wrap());
                        }
                        _ => {
                            ui.label(RichText::new(crate::t!("web-video-kinds")).font(style::ui_font(11.0)).color(p.weak));
                        }
                    }
                });
                ui.add_space(16.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::default_button_ext(ui, &crate::t!("web-video-insert"), enabled, Some(80.0)).clicked() {
                        act = Do::Insert;
                    }
                    if w::push_button_ext(ui, &crate::t!("sheet-cancel"), w::Size::Regular, true, Some(80.0)).clicked() {
                        act = Do::Cancel;
                    }
                });
            });
        });
    ctx.input(|i| {
        if i.key_pressed(egui::Key::Escape) {
            act = Do::Cancel;
        }
        if i.key_pressed(egui::Key::Enter) && act == Do::Nothing && enabled {
            act = Do::Insert;
        }
    });
    match act {
        Do::Nothing => {}
        // A check under way is dropped: its worker ends by itself within
        // its 10 s and its answer goes nowhere; a hidden player closes.
        Do::Cancel => app.web_sheet = None,
        Do::Insert => {
            if app.web_sheet.as_ref().is_some_and(can_insert) {
                start_check(app);
            }
        }
    }
}

// ----------------------------------------------------------------------
// Download

/// A download under way (Download in the inspector).
pub struct Download {
    /// The link it came from: a link changed meanwhile (Undo) drops it.
    url: String,
    rx: Receiver<Result<PathBuf, DownloadError>>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
    /// u64::MAX while the size isn't known.
    total: Arc<AtomicU64>,
}

impl Download {
    /// 0..1 when the size is known.
    pub fn fraction(&self) -> Option<f64> {
        let t = self.total.load(Ordering::Relaxed);
        (t != u64::MAX && t > 0).then(|| (self.done.load(Ordering::Relaxed) as f64 / t as f64).clamp(0.0, 1.0))
    }
    pub fn bytes(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Start downloading a web video file's clip.
pub fn download(app: &mut ZebraApp, id: &str) {
    let Some(ElementKind::Video(v)) = app.editor.doc.get(id).map(|e| &e.kind) else { return };
    let Some(link) = v.link.clone().filter(|l| l.kind == LinkKind::File) else { return };
    if app.media.downloads.contains_key(id) {
        return;
    }
    let dir = zebra_core::media::cache_dir().join("downloads");
    let ext = weblink::parse(&link.url)
        .and_then(|u| u.path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
        .filter(|e| matches!(e.as_str(), "mp4" | "mov" | "m4v"))
        .unwrap_or_else(|| "mp4".into());
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let file = dir.join(format!("{}-{nanos}.{ext}", std::process::id()));
    let (cancel, done, total) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(u64::MAX)));
    let (c2, d2, t2) = (cancel.clone(), done.clone(), total.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = app.ctx.clone();
    let url = link.url.clone();
    let spawned = std::thread::Builder::new().name("zebra-web-download".into()).spawn(move || {
        let r = std::fs::create_dir_all(&dir).map_err(|_| DownloadError::Disk).and_then(|_| {
            let checker = zebra_media::link::Checker::from_env();
            let ctx2 = ctx.clone();
            let last = std::sync::Mutex::new(std::time::Instant::now());
            checker.download(&url, &file, &c2, &move |n, len| {
                d2.store(n, Ordering::Relaxed);
                t2.store(len.unwrap_or(u64::MAX), Ordering::Relaxed);
                // A few repaints a second for the progress line.
                if let Ok(mut l) = last.lock() {
                    if l.elapsed().as_millis() > 200 {
                        *l = std::time::Instant::now();
                        ctx2.request_repaint();
                    }
                }
            })
        });
        let _ = tx.send(r.map(|_| file));
        ctx.request_repaint();
    });
    if spawned.is_ok() {
        app.media.downloads.insert(id.to_string(), Download { url: link.url, rx, cancel, done, total });
    }
}

pub fn cancel_download(app: &mut ZebraApp, id: &str) {
    if let Some(d) = app.media.downloads.remove(id) {
        // The worker stops at its next read and deletes its part file.
        d.cancel();
    }
}

/// Finished downloads: the clip goes into the deck (one undo step).
pub fn poll_downloads(app: &mut ZebraApp) {
    let mut finished = vec![];
    app.media.downloads.retain(|id, d| match d.rx.try_recv() {
        Ok(r) => {
            finished.push((id.clone(), d.url.clone(), r));
            false
        }
        Err(TryRecvError::Empty) => true,
        Err(TryRecvError::Disconnected) => {
            finished.push((id.clone(), d.url.clone(), Err(DownloadError::Disk)));
            false
        }
    });
    if !app.media.downloads.is_empty() {
        app.ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    for (id, url, r) in finished {
        match r {
            Ok(file) => {
                // Still the same web video (not deleted, replaced or undone)?
                let same = matches!(app.editor.doc.get(&id).map(|e| &e.kind), Some(ElementKind::Video(v)) if v.link.as_ref().is_some_and(|l| l.url == url));
                if same {
                    let ok = app.apply(Command::DownloadVideo { id: id.clone(), path: Some(file.to_string_lossy().to_string()) }).is_some();
                    if ok && app.editor.view.mode != zebra_core::Mode::Present {
                        app.toast(crate::t!("web-video-download-done"));
                    }
                }
                // The deck has its own copy now (or didn't want it).
                let _ = std::fs::remove_file(&file);
            }
            Err(DownloadError::Cancelled) => {}
            Err(e) => {
                let reason = match e {
                    DownloadError::Link(l) => error_text(l),
                    DownloadError::NoSpace => crate::t!("web-video-download-no-space"),
                    _ => crate::t!("web-video-download-disk"),
                };
                if app.editor.view.mode != zebra_core::Mode::Present {
                    app.toast(crate::t!("web-video-download-failed", reason = reason));
                }
            }
        }
    }
}

/// "12.5 MB".
pub fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{mb:.1} MB")
    }
}

// ----------------------------------------------------------------------
// Inspector

/// The rows a web video adds to the Video section: its site with Copy
/// Link, and Download (a file link only) with its progress and Cancel.
pub fn inspector_rows(app: &mut ZebraApp, ui: &mut egui::Ui, id: &str, v: &zebra_core::VideoData) {
    let Some(link) = v.link.clone() else { return };
    let weak = ui.visuals().weak_text_color();
    let site = weblink::site_name(&link);
    w::row(ui, &crate::t!("web-video-link"), |ui| {
        // A long host name is cut short; the whole link shows on hover.
        ui.add(egui::Label::new(RichText::new(&site).font(style::ui_font(13.0))).truncate()).on_hover_text(&link.url);
    });
    let busy = app.media.downloads.contains_key(id);
    w::row(ui, "", |ui| {
        if w::push_button_ext(ui, &crate::t!("web-video-copy-link"), w::Size::Regular, true, None).on_hover_text(crate::t!("web-video-copy-link-tip")).clicked() {
            ui.ctx().copy_text(link.url.clone());
            app.toast(crate::t!("web-video-link-copied"));
        }
    });
    // Its own row: the inspector is too narrow for both side by side.
    if !link.kind.is_card() && !busy {
        let file = link.kind == LinkKind::File;
        let tip = if file { crate::t!("web-video-download-tip") } else { crate::t!("web-video-download-stream-tip") };
        w::row(ui, "", |ui| {
            if w::push_button_ext(ui, &crate::t!("web-video-download"), w::Size::Regular, file, None).on_hover_text(tip).clicked() {
                download(app, id);
            }
        });
    }
    if let Some(d) = app.media.downloads.get(id) {
        let text = match d.fraction() {
            Some(f) => crate::t!("web-video-downloading", percent = ((f * 100.0).floor() as i64)),
            None => crate::t!("web-video-downloading-size", size = size_text(d.bytes())),
        };
        // Cancel where Download was, the progress under it.
        w::row(ui, "", |ui| {
            if w::push_button_ext(ui, &crate::t!("sheet-cancel"), w::Size::Regular, true, None).clicked() {
                cancel_download(app, id);
            }
        });
        w::row(ui, "", |ui| {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(RichText::new(text).font(style::ui_font(11.0)).color(weak));
        });
    }
    let note = if link.kind.is_card() {
        Some(crate::t!("web-video-card-note", site = site))
    } else if link.kind == LinkKind::Live {
        Some(crate::t!("web-video-live"))
    } else {
        None
    };
    if let Some(note) = note {
        ui.add(egui::Label::new(RichText::new(note).size(11.0).color(weak)).wrap());
    }
}

// ----------------------------------------------------------------------
// Presenting

/// What the presenter view says about the current step's web videos (the
/// audience screen shows only the poster until one can play).
pub fn presenter_status(app: &ZebraApp) -> Option<String> {
    if let Some(site) = crate::card_window::playing_site(app) {
        return Some(crate::t!("web-video-card-playing", site = site));
    }
    let step = app.editor.view.present.step?;
    let mut loading = false;
    for id in crate::media_ui::videos_on_step(app, step) {
        let Some(ElementKind::Video(v)) = app.editor.doc.get(&id).map(|e| &e.kind) else { continue };
        if v.link.as_ref().is_none_or(|l| l.kind.is_card()) {
            continue;
        }
        if app.media.web_failed(&id) {
            return Some(crate::t!("web-video-failed"));
        }
        loading |= app.media.web_loading(&id);
    }
    loading.then(|| crate::t!("web-video-loading"))
}

/// The ids of the web videos (not cards) among `ids`.
pub fn web_ids(app: &ZebraApp, ids: &[ElementId]) -> Vec<ElementId> {
    ids.iter()
        .filter(|id| matches!(app.editor.doc.get(id).map(|e| &e.kind), Some(ElementKind::Video(v)) if v.link.as_ref().is_some_and(|l| !l.kind.is_card())))
        .cloned()
        .collect()
}

/// Where a web video's chain-link mark goes: a small square in its
/// bottom-left corner, on screen (edit mode only; never when presenting or
/// in exports, which don't draw overlays).
pub fn paint_link_marks(app: &ZebraApp, painter: &egui::Painter) {
    for e in app.editor.doc.ordered() {
        let ElementKind::Video(v) = &e.kind else { continue };
        if v.link.as_ref().is_none_or(|l| l.kind.is_card()) {
            continue;
        }
        let q = crate::canvas::quad_screen(app, e.corners());
        // Corners: top-left, top-right, bottom-right, bottom-left.
        let bl = q[3];
        let side = ((q[2] - q[3]).length().min((q[3] - q[0]).length()) * 0.16).clamp(0.0, 22.0);
        if side < 10.0 || !painter.clip_rect().contains(bl) {
            continue;
        }
        // Inset along the video's own edges, so it turns with it.
        let along = (q[2] - q[3]).normalized();
        let up = (q[0] - q[3]).normalized();
        let c = bl + along * (side * 0.5 + 6.0) + up * (side * 0.5 + 6.0);
        let r = egui::Rect::from_center_size(c, EVec2::splat(side));
        painter.rect_filled(r, side * 0.25, Color32::from_black_alpha(150));
        crate::icons::paint_icon(painter, r.shrink(side * 0.12), crate::icons::Icon::Link, Color32::WHITE);
    }
}
