//! Media in the app: playing videos in place and while presenting, the
//! soundtrack and per-step voice-overs, image crop mode, Replace Image,
//! pasting pictures and files from the system clipboard, and the inspector
//! sections for all of these.
//!
//! Playback state is session state, not document state: nothing here goes
//! through the command bus except edits (crop, poster, trim, options...).
//!
//! Memory: a video holds a platform player and one converted frame only
//! while it is on screen; a video that has been off screen for a second is
//! closed and shows its poster again. Frames are converted at the size the
//! video shows on screen, not at its full pixel size.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use base64::Engine;
use egui::{Color32, CursorIcon, Key, Pos2, Rect as ERect, Stroke, Ui};
use zebra_core::{Command, Crop, ElementId, ElementKind, ElementPatch, ImageMask, MediaKind, Mode, StepTarget, Vec2};
use zebra_media::{AudioPlayer, Backend, VideoPlayer};
use zebra_render::display::ImageLevel;

use crate::app::ZebraApp;

/// Close a video that has been off screen this long (seconds).
const OFFSCREEN_CLOSE: f64 = 1.0;
/// Soundtrack volume factor while a voice-over or an unmuted video plays.
const DUCK: f32 = 0.3;

struct Live {
    player: Box<dyn VideoPlayer>,
    frame: Option<Arc<ImageLevel>>,
    last_seen: f64,
    asset: String,
}

/// A file being copied out of the document for the player.
struct Opening {
    rx: Receiver<std::io::Result<PathBuf>>,
    play: bool,
}

pub struct Media {
    pub backend: Arc<dyn Backend>,
    videos: HashMap<ElementId, Live>,
    opening: HashMap<ElementId, Opening>,
    music: Option<(String, Box<dyn AudioPlayer>)>,
    music_opening: Option<(String, Receiver<std::io::Result<PathBuf>>)>,
    voice: Option<(usize, Box<dyn AudioPlayer>)>,
    voice_opening: Option<(usize, Receiver<std::io::Result<PathBuf>>)>,
    /// (step, build) last seen while presenting.
    last_step: Option<(Option<usize>, usize)>,
    /// Videos already auto-played on the current step.
    autoplayed: HashSet<ElementId>,
    /// Videos being inserted (poster made on a worker first).
    inserting: Vec<Receiver<Insert>>,
    pub crop: Option<CropEdit>,
    /// The text Zebra last put on the system clipboard when copying
    /// objects: pasting it again means "paste the copied objects".
    pub clipboard_marker: Option<String>,
    reported: HashSet<String>,
    /// Background work that ends with a message (exports).
    pub background: Vec<Receiver<String>>,
}

struct Insert {
    path: PathBuf,
    at: Vec2,
    width: f64,
    poster: Option<Vec<u8>>,
}

impl Default for Media {
    fn default() -> Self {
        Media {
            backend: zebra_media::platform(),
            videos: HashMap::new(),
            opening: HashMap::new(),
            music: None,
            music_opening: None,
            voice: None,
            voice_opening: None,
            last_step: None,
            autoplayed: HashSet::new(),
            inserting: vec![],
            crop: None,
            clipboard_marker: None,
            reported: HashSet::new(),
            background: vec![],
        }
    }
}

impl Media {
    /// Live frames and playing state for the renderer.
    pub fn scene(&self, opts: &mut zebra_render::SceneOptions) {
        for (id, l) in &self.videos {
            if let Some(f) = &l.frame {
                opts.video_frames.insert(id.clone(), f.clone());
            }
            if l.player.is_playing() {
                opts.playing.insert(id.clone());
            }
        }
        for id in self.opening.iter().filter(|(_, o)| o.play).map(|(id, _)| id) {
            opts.playing.insert(id.clone());
        }
        if let Some(c) = &self.crop {
            opts.ghost = Some(c.id.clone());
        }
    }

    pub fn is_playing(&self, id: &str) -> bool {
        self.videos.get(id).is_some_and(|l| l.player.is_playing()) || self.opening.get(id).is_some_and(|o| o.play)
    }

    /// Close every player (leaving present mode, opening another file).
    pub fn stop_all(&mut self) {
        self.videos.clear();
        self.opening.clear();
        self.music = None;
        self.music_opening = None;
        self.voice = None;
        self.voice_opening = None;
        self.last_step = None;
        self.autoplayed.clear();
        self.crop = None;
    }

    /// For the `perf` numbers: (open players, converted frame bytes).
    pub fn stats(&self) -> (usize, usize) {
        let bytes = self.videos.values().filter_map(|l| l.frame.as_ref()).map(|f| (f.width * f.height * 4) as usize).sum();
        (self.videos.len() + self.music.is_some() as usize + self.voice.is_some() as usize, bytes)
    }
}

/// Copy an asset to a plain file on a worker thread (players open files).
fn materialize_async(app: &ZebraApp, key: &str) -> Option<Receiver<std::io::Result<PathBuf>>> {
    let a = app.editor.doc.assets.get(key)?;
    let (key, ext, bytes) = (key.to_string(), a.ext.clone(), a.bytes.clone());
    let (tx, rx) = channel();
    let ctx = app.ctx.clone();
    std::thread::Builder::new()
        .name("zebra-media-copy".into())
        .spawn(move || {
            let _ = tx.send(zebra_core::media::materialize(&key, &ext, &bytes));
            ctx.request_repaint();
        })
        .ok()?;
    Some(rx)
}

fn report_once(app: &mut ZebraApp, key: &str, msg: String) {
    if app.media.reported.insert(key.to_string()) {
        app.toast(msg);
    }
}

// ----------------------------------------------------------------------
// Video playback

/// Play or pause a video in place.
pub fn toggle(app: &mut ZebraApp, id: &str) {
    if let Some(l) = app.media.videos.get_mut(id) {
        if l.player.is_playing() {
            l.player.pause();
        } else {
            start_live(app, id);
        }
        app.ctx.request_repaint();
        return;
    }
    if app.media.opening.remove(id).is_some() {
        return;
    }
    play(app, id);
}

pub fn play(app: &mut ZebraApp, id: &str) {
    if !app.media.backend.can_play() {
        report_once(app, "no-video", "Videos play in Zebra on macOS. Here you see the poster frame.".into());
        return;
    }
    if app.media.videos.contains_key(id) {
        start_live(app, id);
        return;
    }
    let Some(ElementKind::Video(v)) = app.editor.doc.get(id).map(|e| e.kind.clone()) else { return };
    if let Some(rx) = materialize_async(app, &v.asset) {
        app.media.opening.insert(id.to_string(), Opening { rx, play: true });
    }
    app.ctx.request_repaint();
}

pub fn pause(app: &mut ZebraApp, id: &str) {
    if let Some(l) = app.media.videos.get_mut(id) {
        l.player.pause();
    }
    app.media.opening.remove(id);
}

/// (Re)start a live player at its trim start when it is at the end or
/// before the start.
fn start_live(app: &mut ZebraApp, id: &str) {
    let Some(ElementKind::Video(v)) = app.editor.doc.get(id).map(|e| e.kind.clone()) else { return };
    let Some(l) = app.media.videos.get_mut(id) else { return };
    let pos = l.player.position();
    let end = v.end.or(l.player.duration()).unwrap_or(f64::INFINITY);
    if pos < v.start || pos >= end - 0.05 || l.player.ended() {
        l.player.seek(v.start);
    }
    l.player.set_muted(v.muted);
    l.player.play();
}

/// Per frame: open finished copies, keep trims and loops, pull frames for
/// visible videos, close off-screen ones, run present-mode media, and
/// schedule repaints only while something plays.
pub fn tick(app: &mut ZebraApp) {
    let now = app.editor.view.now;
    let ppp = app.ctx.pixels_per_point() as f64;
    let presenting = app.editor.view.mode == Mode::Present;

    // Finished background work.
    let mut msgs = vec![];
    app.media.background.retain(|rx| match rx.try_recv() {
        Ok(m) => {
            msgs.push(m);
            false
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => true,
        Err(_) => false,
    });
    for m in msgs {
        app.toast(m);
    }

    // Finished inserts.
    let mut done = vec![];
    app.media.inserting.retain(|rx| match rx.try_recv() {
        Ok(i) => {
            done.push(i);
            false
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => true,
        Err(_) => false,
    });
    for i in done {
        finish_insert_video(app, i);
    }

    // Copies that finished: open the player.
    let ready: Vec<(ElementId, std::io::Result<PathBuf>, bool)> = app
        .media
        .opening
        .iter()
        .filter_map(|(id, o)| o.rx.try_recv().ok().map(|r| (id.clone(), r, o.play)))
        .collect();
    for (id, r, play) in ready {
        app.media.opening.remove(&id);
        let opened = r.map_err(|e| e.to_string()).and_then(|p| app.media.backend.open_video(&p));
        match opened {
            Ok(player) => {
                let asset = match app.editor.doc.get(&id).map(|e| &e.kind) {
                    Some(ElementKind::Video(v)) => v.asset.clone(),
                    _ => continue,
                };
                app.media.videos.insert(id.clone(), Live { player, frame: None, last_seen: now, asset });
                if play {
                    start_live(app, &id);
                }
            }
            Err(e) => report_once(app, &format!("open-{id}"), format!("Couldn't play the video: {e}")),
        }
    }

    // Live videos.
    let view = app.editor.view.camera.visible_bounds(app.editor.view.viewport);
    let zoom = app.editor.view.camera.zoom;
    let hidden = if presenting { app.editor.hidden_builds() } else { HashSet::new() };
    let mut close = vec![];
    let mut any_playing = false;
    let mut loud_video = false;
    for (id, l) in app.media.videos.iter_mut() {
        let Some(e) = app.editor.doc.get(id) else {
            close.push(id.clone());
            continue;
        };
        let ElementKind::Video(v) = &e.kind else {
            close.push(id.clone());
            continue;
        };
        if v.asset != l.asset {
            close.push(id.clone());
            continue;
        }
        let visible = e.bounds().intersects(&view) && !hidden.contains(id);
        if visible {
            l.last_seen = now;
        } else {
            if l.player.is_playing() {
                l.player.pause();
            }
            if now - l.last_seen > OFFSCREEN_CLOSE {
                close.push(id.clone());
            }
            continue;
        }
        // Trim end and loop.
        let end = v.end.or(l.player.duration());
        let at_end = l.player.ended() || end.is_some_and(|end| l.player.position() >= end);
        if at_end && l.player.is_playing() {
            if v.looping {
                l.player.seek(v.start);
                l.player.play();
            } else {
                l.player.pause();
            }
        } else if at_end && v.looping && l.player.ended() {
            l.player.seek(v.start);
            l.player.play();
        }
        l.player.set_muted(v.muted);
        // Convert at on-screen size (device pixels), never above the video.
        let on_screen = (e.world_size().0.max(e.world_size().1) * zoom * ppp).ceil().clamp(16.0, 4096.0) as u32;
        let max_dim = on_screen.next_power_of_two().min(4096);
        if let Some(f) = l.player.next_frame(max_dim) {
            l.frame = Some(Arc::new(ImageLevel::from_premultiplied(&v.asset, f.width, f.height, f.rgba)));
        }
        if l.player.is_playing() {
            any_playing = true;
            loud_video |= !v.muted;
        }
    }
    for id in close {
        app.media.videos.remove(&id);
    }
    if any_playing || !app.media.opening.is_empty() || !app.media.inserting.is_empty() {
        // Frames arrive at the video's rate; poll at display rate.
        app.ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }

    if presenting {
        present_tick(app, loud_video);
    } else if app.media.last_step.is_some() {
        // Left present mode: stop presentation media.
        app.media.music = None;
        app.media.music_opening = None;
        app.media.voice = None;
        app.media.voice_opening = None;
        app.media.last_step = None;
        app.media.autoplayed.clear();
        for l in app.media.videos.values_mut() {
            l.player.pause();
        }
    }
}

/// Videos "on" a step: the step's element, everything inside it, and for
/// view steps everything fully in view.
fn videos_on_step(app: &ZebraApp, step: usize) -> Vec<ElementId> {
    let d = &app.editor.doc;
    let Some(s) = d.path.get(step) else { return vec![] };
    let ids: Vec<ElementId> = match &s.target {
        StepTarget::Element { id } => {
            let mut v = d.frame_contents(id);
            v.push(id.clone());
            v
        }
        StepTarget::View { .. } | StepTarget::Overview => {
            let cam = app.editor.camera_for_step(step);
            let b = cam.visible_bounds(app.editor.view.viewport);
            d.ordered().filter(|e| b.contains_rect(&e.bounds())).map(|e| e.id.clone()).collect()
        }
    };
    ids.into_iter().filter(|id| matches!(d.get(id).map(|e| &e.kind), Some(ElementKind::Video(_)))).collect()
}

fn present_tick(app: &mut ZebraApp, loud_video: bool) {
    let st = (app.editor.view.present.step, app.editor.view.present.build);
    let arrived = app.editor.view.flight.is_none();

    // Soundtrack: opens once per presentation, keeps playing across steps.
    let want = app.editor.doc.soundtrack.clone();
    match (&want, &app.media.music) {
        (Some(t), Some((asset, _))) if *asset == t.asset => {}
        (Some(t), _) if app.media.backend.can_play() => {
            app.media.music = None;
            if app.media.music_opening.as_ref().is_none_or(|(a, _)| *a != t.asset) {
                app.media.music_opening = materialize_async(app, &t.asset).map(|rx| (t.asset.clone(), rx));
            }
        }
        (None, Some(_)) => app.media.music = None,
        _ => {}
    }
    if let Some((asset, rx)) = &app.media.music_opening {
        if let Ok(r) = rx.try_recv() {
            let asset = asset.clone();
            app.media.music_opening = None;
            match r.map_err(|e| e.to_string()).and_then(|p| app.media.backend.open_audio(&p)) {
                Ok(mut p) => {
                    p.play();
                    app.media.music = Some((asset, p));
                }
                Err(e) => report_once(app, "music", format!("Couldn't play the soundtrack: {e}")),
            }
        }
    }

    let step_changed = app.media.last_step.map(|s| s.0) != Some(st.0) || app.media.last_step.is_none();
    let build_changed = app.media.last_step != Some(st);
    if step_changed {
        // Leaving a step stops its media.
        for l in app.media.videos.values_mut() {
            l.player.pause();
        }
        app.media.opening.clear();
        app.media.autoplayed.clear();
        app.media.voice = None;
        app.media.voice_opening = None;
        if let Some(step) = st.0 {
            if let Some(key) = app.editor.doc.path.get(step).and_then(|s| s.voiceover.clone()) {
                if app.media.backend.can_play() {
                    app.media.voice_opening = materialize_async(app, &key).map(|rx| (step, rx));
                }
            }
        }
    }
    if let Some((step, rx)) = &app.media.voice_opening {
        if let Ok(r) = rx.try_recv() {
            let step = *step;
            app.media.voice_opening = None;
            match r.map_err(|e| e.to_string()).and_then(|p| app.media.backend.open_audio(&p)) {
                Ok(mut p) => {
                    p.play();
                    app.media.voice = Some((step, p));
                }
                Err(e) => report_once(app, &format!("voice-{step}"), format!("Couldn't play the voice-over: {e}")),
            }
        }
    }
    // Auto-play once the camera has arrived, and again as builds reveal
    // more videos.
    if (build_changed || step_changed || arrived) && arrived {
        if let Some(step) = st.0 {
            let hidden = app.editor.hidden_builds();
            for id in videos_on_step(app, step) {
                let auto = matches!(app.editor.doc.get(&id).map(|e| &e.kind), Some(ElementKind::Video(v)) if v.autoplay);
                if auto && !hidden.contains(&id) && !app.media.autoplayed.contains(&id) {
                    app.media.autoplayed.insert(id.clone());
                    play(app, &id);
                }
            }
        }
    }
    app.media.last_step = Some(st);

    // Duck the soundtrack under voice-overs and sound from videos.
    if let (Some(t), Some((_, m))) = (&want, app.media.music.as_mut()) {
        let voice = app.media.voice.as_ref().is_some_and(|(_, v)| v.is_playing());
        let k = if voice || loud_video { DUCK } else { 1.0 };
        m.set_volume(t.volume as f32 * k);
        m.set_looping(t.looping);
    }
    if app.media.music.is_some() || app.media.voice.is_some() || app.media.music_opening.is_some() || app.media.voice_opening.is_some() {
        // Slow heartbeat for the ducking logic; audio itself needs no repaints.
        app.ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

// ----------------------------------------------------------------------
// Inserting

pub fn insert_video_dialog(app: &mut ZebraApp) {
    if let Some(p) = rfd::FileDialog::new().add_filter("Video", &["mp4", "mov", "m4v"]).pick_file() {
        let c = app.canvas_center_world();
        insert_video_file(app, &p, c);
    }
}

/// Insert a video: the poster is made on a worker, then the video is added
/// through the command bus by path (copied into the media cache and
/// memory-mapped, never read into memory).
pub fn insert_video_file(app: &mut ZebraApp, p: &std::path::Path, at: Vec2) {
    let width = app.editor.view.viewport.x / app.editor.view.camera.zoom * 0.5;
    let backend = app.media.backend.clone();
    let path = p.to_path_buf();
    let (tx, rx) = channel();
    let ctx = app.ctx.clone();
    let spawned = std::thread::Builder::new().name("zebra-poster".into()).spawn(move || {
        let poster = backend.poster_png(&path, 1.0, 1280).ok();
        let _ = tx.send(Insert { path, at, width, poster });
        ctx.request_repaint();
    });
    if spawned.is_ok() {
        app.media.inserting.push(rx);
        app.ctx.request_repaint();
    }
}

fn finish_insert_video(app: &mut ZebraApp, i: Insert) {
    let alt = i.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let cmd = Command::AddVideo {
        data_base64: String::new(),
        path: Some(i.path.to_string_lossy().to_string()),
        x: i.at.x,
        y: i.at.y,
        width: Some(i.width),
        alt,
        poster_base64: i.poster.map(|p| base64::engine::general_purpose::STANDARD.encode(p)),
        autoplay: true,
        looping: false,
        muted: false,
        id: None,
    };
    if let Some(out) = app.apply(cmd) {
        app.select(out.created);
        app.view_cmd(zebra_core::ViewCommand::SetTool { tool: zebra_core::Tool::Select });
    }
}

pub fn soundtrack_dialog(app: &mut ZebraApp) {
    if let Some(p) = rfd::FileDialog::new().add_filter("Audio", &["mp3", "m4a", "aac", "wav"]).pick_file() {
        set_soundtrack_file(app, &p);
    }
}

pub fn set_soundtrack_file(app: &mut ZebraApp, p: &std::path::Path) {
    app.apply(Command::SetSoundtrack {
        data_base64: String::new(),
        path: Some(p.to_string_lossy().to_string()),
        looping: None,
        volume: None,
        name: None,
    });
}

pub fn voiceover_dialog(app: &mut ZebraApp, step: usize) {
    if let Some(p) = rfd::FileDialog::new().add_filter("Audio", &["mp3", "m4a", "aac", "wav"]).pick_file() {
        app.apply(Command::SetVoiceover { index: step, data_base64: String::new(), path: Some(p.to_string_lossy().to_string()) });
    }
}

pub fn replace_image_dialog(app: &mut ZebraApp, id: &str) {
    if let Some(p) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "webp", "gif"]).pick_file() {
        app.apply(Command::ReplaceImage { id: id.to_string(), data_base64: String::new(), path: Some(p.to_string_lossy().to_string()) });
    }
}

/// Insert any dropped or pasted file by what it is: pictures and videos go
/// on the canvas, audio becomes the soundtrack.
pub fn insert_file(app: &mut ZebraApp, p: &std::path::Path, at: Vec2) {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match MediaKind::from_ext(match ext.as_str() {
        "jpeg" => "jpg",
        e => e,
    }) {
        MediaKind::Video => insert_video_file(app, p, at),
        MediaKind::Audio => set_soundtrack_file(app, p),
        MediaKind::Image => app.insert_image_file(p, at),
    }
}

// ----------------------------------------------------------------------
// System clipboard

/// What Zebra puts on the system clipboard when objects are copied: their
/// text (handy in other apps) or a short description.
pub fn copied_to_system(app: &mut ZebraApp, ids: &[ElementId]) {
    let texts: Vec<String> = ids
        .iter()
        .filter_map(|id| match app.editor.doc.get(id).map(|e| &e.kind) {
            Some(ElementKind::Text(t)) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    let marker = if texts.is_empty() {
        format!("{} Zebra object{}", ids.len(), if ids.len() == 1 { "" } else { "s" })
    } else {
        texts.join("\n")
    };
    if let Ok(mut c) = arboard::Clipboard::new() {
        if c.set_text(marker.clone()).is_ok() {
            app.media.clipboard_marker = Some(marker);
            return;
        }
    }
    app.media.clipboard_marker = None;
}

/// Paste from the system clipboard if it holds something newer than
/// Zebra's own copy: files (Finder), a picture, or text. Returns false when
/// the caller should paste Zebra's copied objects instead.
pub fn paste_from_system(app: &mut ZebraApp) -> bool {
    let Ok(mut c) = arboard::Clipboard::new() else { return false };
    let at = app.canvas_center_world();
    if let Ok(files) = c.get().file_list() {
        let files: Vec<PathBuf> = files.into_iter().filter(|p| p.is_file()).collect();
        if !files.is_empty() {
            for (i, p) in files.iter().enumerate() {
                let off = 30.0 * i as f64 / app.editor.view.camera.zoom;
                insert_file(app, p, at + Vec2::new(off, off));
            }
            return true;
        }
    }
    if let Ok(text) = c.get_text() {
        if app.media.clipboard_marker.as_deref() == Some(text.as_str()) {
            return false;
        }
        if !text.trim().is_empty() {
            let cmd = Command::AddText { text, x: at.x, y: at.y, style: Default::default(), scale: Some(1.0 / app.editor.view.camera.zoom), place: None, id: None , markdown: false, on: None };
            if let Some(out) = app.apply(cmd) {
                app.select(out.created);
            }
            return true;
        }
    }
    if let Ok(img) = c.get_image() {
        paste_rgba(app, img.width as u32, img.height as u32, img.bytes.into_owned());
        return true;
    }
    false
}

/// Paste raw RGBA pixels as a PNG picture at the view centre.
pub fn paste_rgba(app: &mut ZebraApp, w: u32, h: u32, rgba: Vec<u8>) {
    let Some(img) = image::RgbaImage::from_raw(w, h, rgba) else { return };
    let mut png = vec![];
    if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_ok() {
        let at = app.canvas_center_world();
        app.insert_image_bytes(png, at, "Pasted image".into());
    }
}

// ----------------------------------------------------------------------
// Crop mode

/// An image being cropped. Edits go through one transaction, so the whole
/// crop is one undo step.
pub struct CropEdit {
    pub id: ElementId,
    /// Element and crop when the current drag started.
    start: Option<(zebra_core::Element, Crop, CropDrag)>,
    began: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum CropDrag {
    /// Move the crop window over the picture.
    Pan,
    /// Edges: (left, top, right, bottom) moved.
    Edges(bool, bool, bool, bool),
}

pub fn crop_active(app: &ZebraApp) -> bool {
    app.media.crop.is_some()
}

pub fn begin_crop(app: &mut ZebraApp, id: &str) {
    if !matches!(app.editor.doc.get(id).map(|e| &e.kind), Some(ElementKind::Image(_))) {
        return;
    }
    if app.editor.doc.get(id).is_some_and(|e| e.locked) {
        app.toast("Unlock the image to crop it.");
        return;
    }
    crate::canvas::commit_text_edit(app);
    app.select(vec![id.to_string()]);
    app.media.crop = Some(CropEdit { id: id.to_string(), start: None, began: false });
}

pub fn end_crop(app: &mut ZebraApp, keep: bool) {
    let Some(c) = app.media.crop.take() else { return };
    if c.began {
        if keep {
            app.editor.commit();
        } else {
            app.editor.abort();
        }
    }
}

/// Picture geometry for crop mode: the image's current crop, the full
/// picture's corners on screen, and the crop window's corners on screen.
struct CropGeom {
    crop: Crop,
    full: [Pos2; 4],
    window: [Pos2; 4],
}

fn crop_geom(app: &ZebraApp, e: &zebra_core::Element) -> Option<CropGeom> {
    let ElementKind::Image(img) = &e.kind else { return None };
    let c = img.crop.unwrap_or(Crop::FULL);
    let (w, h) = (e.size.w, e.size.h);
    let (fw, fh) = (w / c.w, h / c.h);
    let x0 = -w / 2.0 - c.x * fw;
    let y0 = -h / 2.0 - c.y * fh;
    let to_s = |lx: f64, ly: f64| app.canvas.w2s(&app.editor, e.transform.apply(Vec2::new(lx, ly)));
    let full = [to_s(x0, y0), to_s(x0 + fw, y0), to_s(x0 + fw, y0 + fh), to_s(x0, y0 + fh)];
    let window = [to_s(-w / 2.0, -h / 2.0), to_s(w / 2.0, -h / 2.0), to_s(w / 2.0, h / 2.0), to_s(-w / 2.0, h / 2.0)];
    Some(CropGeom { crop: c, full, window })
}

/// Source-fraction coordinates of a screen point, in the frame of `e0`
/// (the element as it was when the drag started).
fn frac_at(app: &ZebraApp, e0: &zebra_core::Element, c0: Crop, p: Pos2) -> (f64, f64) {
    let wpt = app.canvas.s2w(&app.editor, p);
    let l = e0.transform.inverse(wpt);
    let (fw, fh) = (e0.size.w / c0.w, e0.size.h / c0.h);
    (c0.x + (l.x + e0.size.w / 2.0) / fw, c0.y + (l.y + e0.size.h / 2.0) / fh)
}

fn handle_points(q: &[Pos2; 4]) -> [(Pos2, CropDrag); 8] {
    let mid = |a: Pos2, b: Pos2| Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    [
        (q[0], CropDrag::Edges(true, true, false, false)),
        (q[1], CropDrag::Edges(false, true, true, false)),
        (q[2], CropDrag::Edges(false, false, true, true)),
        (q[3], CropDrag::Edges(true, false, false, true)),
        (mid(q[0], q[1]), CropDrag::Edges(false, true, false, false)),
        (mid(q[1], q[2]), CropDrag::Edges(false, false, true, false)),
        (mid(q[2], q[3]), CropDrag::Edges(false, false, false, true)),
        (mid(q[3], q[0]), CropDrag::Edges(true, false, false, false)),
    ]
}

fn in_quad(p: Pos2, q: &[Pos2; 4]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let c = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
        if c != 0.0 {
            if sign != 0.0 && c.signum() != sign {
                return false;
            }
            sign = c.signum();
        }
    }
    true
}

/// Crop-mode input and overlay. Called by the canvas instead of its usual
/// interaction while crop mode is on.
pub fn crop_interact(app: &mut ZebraApp, ui: &mut Ui, resp: &egui::Response) {
    let Some(id) = app.media.crop.as_ref().map(|c| c.id.clone()) else { return };
    let Some(e) = app.editor.doc.get(&id).cloned() else {
        app.media.crop = None;
        return;
    };
    let ctx = ui.ctx().clone();
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        end_crop(app, false);
        return;
    }
    if ctx.input(|i| i.key_pressed(Key::Enter)) {
        end_crop(app, true);
        return;
    }
    let Some(g) = crop_geom(app, &e) else { return };
    let pointer = resp.hover_pos();
    let handles = handle_points(&g.window);
    let under = pointer.and_then(|p| handles.iter().find(|(h, _)| h.distance(p) < 9.0).map(|(_, d)| *d));
    let inside = pointer.is_some_and(|p| in_quad(p, &g.full));

    if resp.drag_started_by(egui::PointerButton::Primary) {
        let start = ctx.input(|i| i.pointer.press_origin()).unwrap_or_default();
        let drag = handles.iter().find(|(h, _)| h.distance(start) < 9.0).map(|(_, d)| *d).or(in_quad(start, &g.full).then_some(CropDrag::Pan));
        if let (Some(d), Some(c)) = (drag, app.media.crop.as_mut()) {
            c.start = Some((e.clone(), g.crop, d));
            if !c.began && !app.editor.in_transaction() {
                c.began = app.editor.begin("Crop", crate::app::HUMAN).is_ok();
            }
        }
    }
    if resp.dragged_by(egui::PointerButton::Primary) {
        if let (Some(p), Some((e0, c0, d))) = (resp.interact_pointer_pos(), app.media.crop.as_ref().and_then(|c| c.start.clone())) {
            let origin = ctx.input(|i| i.pointer.press_origin()).unwrap_or(p);
            let (u0, v0) = frac_at(app, &e0, c0, origin);
            let (u, v) = frac_at(app, &e0, c0, p);
            let (du, dv) = (u - u0, v - v0);
            let mut c = c0;
            match d {
                CropDrag::Pan => {
                    // Dragging the picture moves the window the other way.
                    c.x = (c0.x - du).clamp(0.0, 1.0 - c0.w);
                    c.y = (c0.y - dv).clamp(0.0, 1.0 - c0.h);
                }
                CropDrag::Edges(l, t, r, b) => {
                    let (mut x0, mut y0, mut x1, mut y1) = (c0.x, c0.y, c0.x + c0.w, c0.y + c0.h);
                    if l {
                        x0 = (x0 + du).clamp(0.0, x1 - 0.02);
                    }
                    if r {
                        x1 = (x1 + du).clamp(x0 + 0.02, 1.0);
                    }
                    if t {
                        y0 = (y0 + dv).clamp(0.0, y1 - 0.02);
                    }
                    if b {
                        y1 = (y1 + dv).clamp(y0 + 0.02, 1.0);
                    }
                    c = Crop { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
                }
            }
            let _ = app.editor.apply(Command::Update { id: id.clone(), patch: ElementPatch { crop: Some(c), ..Default::default() } }, crate::app::HUMAN);
        }
    }
    if resp.drag_stopped() {
        if let Some(c) = app.media.crop.as_mut() {
            c.start = None;
        }
    }
    // A click outside the picture finishes cropping.
    if resp.clicked() && !inside {
        end_crop(app, true);
        return;
    }

    ctx.set_cursor_icon(match under {
        Some(CropDrag::Edges(true, true, _, _) | CropDrag::Edges(_, _, true, true)) => CursorIcon::ResizeNwSe,
        Some(CropDrag::Edges(false, true, true, false) | CropDrag::Edges(true, false, false, true)) => CursorIcon::ResizeNeSw,
        Some(CropDrag::Edges(true, false, false, false) | CropDrag::Edges(false, false, true, false)) => CursorIcon::ResizeHorizontal,
        Some(CropDrag::Edges(..)) => CursorIcon::ResizeVertical,
        _ if inside => CursorIcon::Grab,
        _ => CursorIcon::Default,
    });

    // Overlay: the whole picture's outline, the crop window, handles.
    let painter = ui.painter_at(resp.rect);
    let g = app.editor.doc.get(&id).and_then(|e| crop_geom(app, e)).unwrap_or(g);
    let dash = Stroke::new(1.0, Color32::from_white_alpha(200));
    for i in 0..4 {
        painter.extend(egui::Shape::dashed_line(&[g.full[i], g.full[(i + 1) % 4]], dash, 4.0, 3.0));
    }
    let edge = Stroke::new(1.5, crate::style::ACCENT);
    painter.add(egui::Shape::closed_line(g.window.to_vec(), edge));
    for (h, _) in handle_points(&g.window) {
        painter.rect(ERect::from_center_size(h, egui::vec2(9.0, 9.0)), 1.5, Color32::WHITE, edge, egui::StrokeKind::Middle);
    }
    let top = g.full.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let cx = g.full.iter().map(|p| p.x).sum::<f32>() / 4.0;
    let bar = ERect::from_center_size(Pos2::new(cx, (top - 22.0).max(resp.rect.top() + 16.0)), egui::vec2(250.0, 26.0));
    painter.rect_filled(bar, 6.0, Color32::from_black_alpha(190));
    painter.text(bar.center(), egui::Align2::CENTER_CENTER, "Drag edges to crop · ↵ Done · Esc Cancel", egui::FontId::proportional(12.0), Color32::WHITE);
}

// ----------------------------------------------------------------------
// Inspector sections

/// Image section: crop, mask, replace. Returns a patch to apply.
pub fn image_section(app: &mut ZebraApp, ui: &mut Ui, id: &str, img: &zebra_core::ImageData) -> Option<ElementPatch> {
    let mut patch = None;
    ui.horizontal_wrapped(|ui| {
        let cropping = app.media.crop.as_ref().is_some_and(|c| c.id == id);
        if cropping {
            if ui.button("Done").on_hover_text("Finish cropping  ↵").clicked() {
                end_crop(app, true);
            }
        } else if ui.button("Crop…").on_hover_text("Crop the picture (or double-click it)").clicked() {
            begin_crop(app, id);
        }
        if img.crop.is_some() && ui.button("Uncrop").clicked() {
            patch = Some(ElementPatch { crop: Some(Crop::FULL), ..Default::default() });
        }
        if ui.button("Replace…").on_hover_text("Swap the picture, keeping its place and size").clicked() {
            replace_image_dialog(app, id);
        }
    });
    crate::widgets::row(ui, "Mask", |ui| {
        use crate::widgets::Seg;
        let mut m = img.mask;
        let items = [(ImageMask::Rectangle, Seg::Text("Square")), (ImageMask::Rounded, Seg::Text("Rounded")), (ImageMask::Circle, Seg::Text("Circle"))];
        if crate::widgets::segmented(ui, &mut m, &items, Some(ui.available_width())).changed() {
            patch = Some(ElementPatch { mask: Some(m), ..Default::default() });
        }
    });
    if let Some(a) = app.editor.doc.assets.get(&img.asset) {
        let note = if a.ext == "gif" { " · animated GIF plays on the canvas" } else { "" };
        ui.label(egui::RichText::new(format!("{} × {} px{note}", a.width, a.height)).size(11.0).color(ui.visuals().weak_text_color()));
    }
    patch
}

fn fmt_time(s: f64) -> String {
    let s = s.max(0.0);
    format!("{}:{:04.1}", (s / 60.0).floor() as u64, s % 60.0)
}

/// Video section: play, options, trim, poster. Returns a patch to apply and
/// whether a value is being dragged (one undo step per drag).
pub fn video_section(app: &mut ZebraApp, ui: &mut Ui, id: &str, v: &zebra_core::VideoData) -> (Option<ElementPatch>, bool) {
    let mut patch = None;
    let mut dragging = false;
    let playing = app.media.is_playing(id);
    ui.horizontal_wrapped(|ui| {
        let label = if playing { "Pause" } else { "Play" };
        if ui.button(label).on_hover_text("Play in place (or double-click the video)").clicked() {
            toggle(app, id);
        }
        let pos = app.media.videos.get(id).map(|l| l.player.position()).unwrap_or(v.start);
        let dur = if v.duration > 0.0 { v.duration } else { app.media.videos.get(id).and_then(|l| l.player.duration()).unwrap_or(0.0) };
        ui.label(egui::RichText::new(format!("{} / {}", fmt_time(pos), fmt_time(dur))).monospace().size(11.5));
    });
    if !app.media.backend.can_play() {
        ui.label(egui::RichText::new("Video plays on macOS; the poster shows here.").size(11.0).color(ui.visuals().weak_text_color()));
    }
    let mut auto = v.autoplay;
    if ui.checkbox(&mut auto, "Auto-play on its step").changed() {
        patch = Some(ElementPatch { autoplay: Some(auto), ..Default::default() });
    }
    ui.horizontal_wrapped(|ui| {
        let mut lp = v.looping;
        if ui.checkbox(&mut lp, "Loop").changed() {
            patch = Some(ElementPatch { looping: Some(lp), ..Default::default() });
        }
        let mut m = v.muted;
        if ui.checkbox(&mut m, "Mute").changed() {
            patch = Some(ElementPatch { muted: Some(m), ..Default::default() });
        }
    });
    let max = if v.duration > 0.0 { v.duration } else { 3600.0 };
    egui::Grid::new("trim").num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
        ui.label("Start");
        let mut s = v.start;
        let r = ui.add(egui::DragValue::new(&mut s).speed(0.05).range(0.0..=max).suffix(" s").max_decimals(1));
        dragging |= r.dragged() || r.has_focus();
        if r.changed() && v.end.is_none_or(|e| s < e) {
            patch = Some(ElementPatch { start: Some(s), ..Default::default() });
        }
        ui.end_row();
        ui.label("End");
        ui.horizontal_wrapped(|ui| {
            let mut e = v.end.unwrap_or(max);
            let r = ui.add(egui::DragValue::new(&mut e).speed(0.05).range(0.0..=max).suffix(" s").max_decimals(1));
            dragging |= r.dragged() || r.has_focus();
            if r.changed() && e > v.start {
                patch = Some(ElementPatch { end: Some(e), ..Default::default() });
            }
            if v.end.is_some() && ui.small_button("×").on_hover_text("Play to the end").clicked() {
                patch = Some(ElementPatch { end: Some(-1.0), ..Default::default() });
            }
        });
        ui.end_row();
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Poster");
        let frame = app.media.videos.get(id).and_then(|l| l.frame.clone());
        if let Some(f) = frame {
            if ui.button("Use This Frame").on_hover_text("Show this frame until the video plays").clicked() {
                let png = zebra_media::encode_png(&zebra_media::Frame { width: f.width, height: f.height, rgba: f.data.data.data().to_vec() });
                app.apply(Command::SetPoster { id: id.to_string(), data_base64: base64::engine::general_purpose::STANDARD.encode(png), path: None });
            }
        }
        if ui.button("Choose…").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "webp"]).pick_file() {
                app.apply(Command::SetPoster { id: id.to_string(), data_base64: String::new(), path: Some(p.to_string_lossy().to_string()) });
            }
        }
        if v.poster.is_some() && ui.small_button("×").on_hover_text("Remove the poster").clicked() {
            app.apply(Command::SetPoster { id: id.to_string(), data_base64: String::new(), path: None });
        }
    });
    (patch, dragging)
}

/// Document section (nothing selected): the soundtrack.
pub fn document_section(app: &mut ZebraApp, ui: &mut Ui) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new("SOUNDTRACK").size(10.5).color(ui.visuals().weak_text_color()).strong());
    match app.editor.doc.soundtrack.clone() {
        None => {
            if ui.button("Add Music…").on_hover_text("Background music that plays while presenting").clicked() {
                soundtrack_dialog(app);
            }
        }
        Some(t) => {
            ui.horizontal(|ui| {
                ui.label(format!("Music: {}", if t.name.is_empty() { "Music" } else { &t.name }));
                if ui.small_button("Replace…").clicked() {
                    soundtrack_dialog(app);
                }
                if ui.small_button("Remove").clicked() {
                    app.apply(Command::RemoveSoundtrack);
                }
            });
            let mut vol = t.volume;
            let r = ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).text("Volume").show_value(false));
            if r.drag_started() && !app.editor.in_transaction() {
                let _ = app.editor.begin("Change soundtrack", crate::app::HUMAN);
            }
            if r.changed() {
                let _ = app.editor.apply(Command::SetSoundtrack { data_base64: String::new(), path: None, looping: None, volume: Some(vol), name: None }, crate::app::HUMAN);
            }
            if r.drag_stopped() && app.editor.in_transaction() {
                app.editor.commit();
            }
            let mut lp = t.looping;
            if ui.checkbox(&mut lp, "Loop").changed() {
                app.apply(Command::SetSoundtrack { data_base64: String::new(), path: None, looping: Some(lp), volume: None, name: None });
            }
        }
    }
}

/// Path-step section: the step's voice-over.
pub fn step_section(app: &mut ZebraApp, ui: &mut Ui, step: usize) {
    let Some(s) = app.editor.doc.path.get(step).cloned() else { return };
    ui.add_space(6.0);
    ui.label(egui::RichText::new(format!("STEP {} VOICE-OVER", step + 1)).size(10.5).color(ui.visuals().weak_text_color()).strong());
    ui.horizontal(|ui| match s.voiceover {
        None => {
            if ui.button("Add Recording…").on_hover_text("Audio that plays when this step is reached").clicked() {
                voiceover_dialog(app, step);
            }
        }
        Some(_) => {
            ui.label("Recorded");
            if ui.small_button("Replace…").clicked() {
                voiceover_dialog(app, step);
            }
            if ui.small_button("Remove").clicked() {
                app.apply(Command::SetVoiceover { index: step, data_base64: String::new(), path: None });
            }
        }
    });
}

// ----------------------------------------------------------------------
// Demo deck (screenshots and manual testing)

fn demo_picture(w: u32, h: u32, hue: f32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        // Sky to sea, with a sun.
        let sun = ((fx - 0.7).powi(2) + (fy - 0.3).powi(2)).sqrt() < 0.12;
        if sun {
            return image::Rgba([255, 214, 120, 255]);
        }
        let horizon = fy > 0.55 + 0.04 * (fx * 12.0).sin();
        let (r, g, b) = if horizon { (20.0 + 40.0 * fy, 70.0 + 60.0 * fy, 120.0 + 60.0 * fy) } else { (250.0 - 120.0 * fy, 150.0 + 60.0 * fy * hue, 120.0 + 120.0 * fy) };
        image::Rgba([r as u8, g as u8, b as u8, 255])
    });
    let mut v = vec![];
    let _ = img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png);
    v
}

pub fn demo(app: &mut ZebraApp, which: &str) {
    use zebra_core::Origin;
    let h = Origin::Human;
    app.editor.load(zebra_core::Document::default(), None);
    app.after_load();
    let ed = &mut app.editor;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let cmd = |j: serde_json::Value| -> Command { serde_json::from_value(j).expect("demo command") };
    let _ = ed.apply(cmd(serde_json::json!({"op":"add_frame","x":0,"y":0,"w":2400,"h":1300,"style":"bracket","name":"Media","add_to_path":true})), h.clone());
    let _ = ed.apply(cmd(serde_json::json!({"op":"add_text","text":"Pictures, video and sound","x":0,"y":-520,"style":"title","scale":1.6})), h.clone());
    let pic = demo_picture(1200, 800, 1.0);
    let a = ed.apply(cmd(serde_json::json!({"op":"add_image","data_base64":b64(&pic),"x":-760,"y":80,"width":640,"alt":"Sunset"})), h.clone()).ok().and_then(|o| o.created.first().cloned());
    let c = ed.apply(cmd(serde_json::json!({"op":"add_image","data_base64":b64(&demo_picture(800, 800, 0.4)),"x":0,"y":80,"width":520,"alt":"Sun"})), h.clone()).ok().and_then(|o| o.created.first().cloned());
    if let Some(c) = &c {
        let _ = ed.apply(cmd(serde_json::json!({"op":"update","id":c,"patch":{"crop":{"x":0.45,"y":0.1,"w":0.5,"h":0.5},"mask":"circle"}})), h.clone());
        let _ = ed.apply(cmd(serde_json::json!({"op":"update","id":c,"patch":{"x":0,"y":80}})), h.clone());
    }
    // A video file (header only; the fake backend plays it).
    let dir = zebra_core::media::cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("demo-clip.mp4");
    let _ = std::fs::write(&path, zebra_core::media::fixtures::tiny_mp4(1280, 720, 20, true));
    let poster = app.media.backend.poster_png(&path, 2.0, 640).ok();
    let v = app
        .editor
        .apply(
            Command::AddVideo {
                data_base64: String::new(),
                path: Some(path.to_string_lossy().to_string()),
                x: 760.0,
                y: 80.0,
                width: Some(640.0),
                alt: "Product demo".into(),
                poster_base64: poster.map(|p| b64(&p)),
                autoplay: true,
                looping: true,
                muted: false,
                id: None,
            },
            h.clone(),
        )
        .ok()
        .and_then(|o| o.created.first().cloned());
    let _ = app.editor.view(zebra_core::ViewCommand::FitAll { animate: false }, h.clone());
    match which {
        "mediacrop" => {
            if let Some(a) = a {
                let _ = app.editor.apply(cmd(serde_json::json!({"op":"update","id":a,"patch":{"crop":{"x":0.2,"y":0.1,"w":0.6,"h":0.7}}})), h.clone());
                begin_crop(app, &a);
            }
        }
        "mediaplay" => {
            if let Some(v) = v {
                app.select(vec![v.clone()]);
                play(app, &v);
            }
        }
        _ => {}
    }
}
