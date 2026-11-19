//! Zebra's web player: zebra-core's presenting logic and zebra-render's
//! display list and CPU rasteriser, built for `wasm32-unknown-unknown` and
//! driven by the page script of an exported web page (`player.js`).
//!
//! The page owns the browser side (canvas, keys, pointer, `<video>` and
//! `<audio>` elements, the control bar); this crate owns everything the app
//! does while presenting: the path, builds and their fades, camera flights,
//! autoplay, free zoom and pan off the path, hit testing, and drawing.
//!
//! The wasm interface ([`ffi`]) is plain C ABI functions over linear memory,
//! so the page needs no generated glue and the build needs no tool beyond
//! the wasm32 target.
use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Value};
use zebra_core::model::{Asset, StepTarget};
use zebra_core::view::Blank;
use zebra_core::{Camera, Editor, ElementId, ElementKind, FrameStyle, Mode, Origin, ViewCommand};
use zebra_render::{Renderer, SceneOptions};

const H: Origin = Origin::Human;
/// Seconds a build takes to fade in (as in the app).
const FADE: f64 = 0.3;
/// Overview steps show later fade-ins at this opacity (as in the app).
const DIMMED: f32 = 0.25;

/// What a click on the show did.
#[derive(Debug, Clone, PartialEq)]
pub enum Click {
    /// Empty space: next step.
    Next,
    /// An object: the camera zooms to it (off the path).
    Zoom,
    /// A link in text: open it.
    Link(String),
    /// A video: play or pause it.
    Video(ElementId),
}

pub struct Player {
    pub renderer: Renderer,
    pub editor: Editor,
    assets: BTreeMap<String, Asset>,
    /// Device pixels per CSS pixel.
    ppp: f64,
    /// Last frame, straight-alpha RGBA8.
    pub pixels: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    prev_hidden: HashSet<ElementId>,
    revealed: HashMap<ElementId, f64>,
    /// Videos the page is playing (their ▶ badge is not drawn).
    playing: HashSet<ElementId>,
    /// Something changed that the last frame doesn't show.
    dirty: bool,
    /// An animated GIF wants its next frame at this time.
    gif_due: Option<f64>,
    /// Camera of the last frame drawn.
    drawn: Option<Camera>,
    /// Frames drawn so far (the page copies pixels when it changes).
    pub frames: u32,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        let renderer = Renderer::new();
        let mut editor = Editor::default();
        editor.measure = renderer.measure();
        Player {
            renderer,
            editor,
            assets: BTreeMap::new(),
            ppp: 1.0,
            pixels: vec![],
            width_px: 0,
            height_px: 0,
            prev_hidden: HashSet::new(),
            revealed: HashMap::new(),
            playing: HashSet::new(),
            dirty: true,
            gif_due: None,
            drawn: None,
            frames: 0,
        }
    }

    pub fn add_font(&mut self, bytes: Vec<u8>) {
        self.renderer.add_font(bytes);
    }

    /// An asset the document refers to. Pictures need their bytes; video
    /// and audio play in the page, so theirs may be empty.
    pub fn add_asset(&mut self, key: &str, ext: &str, width: u32, height: u32, bytes: Vec<u8>) {
        self.assets.insert(key.to_string(), Asset { ext: ext.to_string(), bytes: bytes.into(), width, height });
    }

    /// Load `document.json` (assets first) and start presenting at the
    /// first step.
    pub fn load(&mut self, json: &str) -> Result<(), String> {
        let mut doc = zebra_core::io::from_json(json).map_err(|e| e.to_string())?;
        doc.assets = std::mem::take(&mut self.assets);
        self.editor.load(doc, None);
        self.editor.view(ViewCommand::Present { step: None }, H).map_err(|e| e.to_string())?;
        // The first step shows at once; flights start from there.
        self.editor.view.flight = None;
        self.editor.view.camera = self.target_camera();
        self.prev_hidden = self.editor.hidden_builds();
        self.dirty = true;
        Ok(())
    }

    fn target_camera(&self) -> Camera {
        let ed = &self.editor;
        match ed.view.present.step {
            Some(i) if i < ed.doc.path.len() => ed.camera_for_step(i),
            _ => ed.fit_all_camera(),
        }
    }

    /// The show's area: `w` x `h` CSS pixels at `ppp` device pixels each.
    pub fn resize(&mut self, w: f64, h: f64, ppp: f64) {
        let (w, h) = (w.max(1.0), h.max(1.0));
        self.ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
        let _ = self.editor.view(ViewCommand::SetViewport { width: w, height: h }, H);
        // As in the app: a step is fitted to the size of the screen, so a
        // new size fits it again (a flight under way flies to the new fit).
        if !self.editor.view.present.off_path {
            let cam = self.target_camera();
            match &mut self.editor.view.flight {
                Some(f) => f.to = cam,
                None => self.editor.view.camera = cam,
            }
        }
        self.dirty = true;
    }

    pub fn command(&mut self, c: ViewCommand) {
        if self.editor.view(c, H).is_ok() {
            self.dirty = true;
        }
    }

    pub fn next(&mut self) {
        self.command(ViewCommand::Next)
    }
    pub fn prev(&mut self) {
        self.command(ViewCommand::Prev)
    }
    pub fn goto(&mut self, step: usize) {
        self.command(ViewCommand::GotoStep { step })
    }
    pub fn overview(&mut self) {
        self.command(ViewCommand::Overview)
    }
    pub fn last(&mut self) {
        if let Some(s) = self.editor.doc.path.len().checked_sub(1) {
            self.goto(s)
        }
    }
    pub fn toggle_blank(&mut self, b: Blank) {
        let cur = self.editor.view.present.blank;
        let blank = if cur == Some(b) { None } else { Some(b) };
        self.command(ViewCommand::Blank { blank })
    }
    pub fn pause_autoplay(&mut self, paused: bool) {
        self.command(ViewCommand::PauseAutoplay { paused })
    }
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.command(ViewCommand::Pan { dx, dy })
    }
    /// Zoom by `factor` about the CSS pixel (`x`, `y`).
    pub fn zoom(&mut self, factor: f64, x: f64, y: f64) {
        self.command(ViewCommand::Zoom { factor, anchor: Some([x, y]) })
    }

    pub fn set_playing(&mut self, ids: HashSet<ElementId>) {
        if ids != self.playing {
            self.playing = ids;
            self.dirty = true;
        }
    }

    /// A click at CSS pixel (`x`, `y`), as in the app's present mode: a
    /// link opens, a video plays or pauses, an object zooms (off-path),
    /// empty space goes to the next step.
    pub fn click(&mut self, x: f64, y: f64) -> Click {
        let ed = &self.editor;
        let w = ed.view.camera.screen_to_world(zebra_core::Vec2::new(x, y), ed.view.viewport);
        let hidden = ed.hidden_builds();
        let link = ed
            .elements_at(w, 0.0)
            .into_iter()
            .rev()
            .filter(|e| !hidden.contains(&e.id) && e.bounds().contains(w))
            .find_map(|e| self.renderer.link_at(ed, e, w));
        if let Some(url) = link {
            return Click::Link(url);
        }
        let tol = 5.0 / ed.view.camera.zoom;
        let invisible = |e: &zebra_core::Element| matches!(&e.kind, ElementKind::Frame(f) if f.style == FrameStyle::Invisible);
        let hit = ed
            .elements_at(w, tol)
            .into_iter()
            .rev()
            .filter(|e| !hidden.contains(&e.id) && !invisible(e) && !e.placeholder)
            .find(|e| e.hit(w, tol, false))
            .map(|e| (e.id.clone(), matches!(e.kind, ElementKind::Video(_))));
        match hit {
            Some((id, true)) => Click::Video(id),
            Some((id, false)) => {
                self.command(ViewCommand::ZoomTo { id, animate: true });
                Click::Zoom
            }
            None => {
                self.next();
                Click::Next
            }
        }
    }

    /// Advance to `now` (seconds) and draw if anything changed. Returns
    /// seconds until the next frame is wanted: 0 while animating, None
    /// when idle until input.
    pub fn frame(&mut self, now: f64) -> Option<f64> {
        let flying = self.editor.view.tick(now);
        let autoplay = self.editor.autoplay_tick();
        if self.editor.view.flight.is_some() {
            self.dirty = true;
        }
        // Fade-ins: when a build is revealed, ~0.3 s.
        let hidden = self.editor.hidden_builds();
        for id in self.prev_hidden.difference(&hidden) {
            self.revealed.insert(id.clone(), now);
        }
        let fading = !self.revealed.is_empty();
        self.revealed.retain(|_, t| now - *t < FADE + 0.1);
        if hidden != self.prev_hidden || fading {
            self.dirty = true;
        }
        self.prev_hidden = hidden;
        if self.gif_due.is_some_and(|t| now >= t) {
            self.dirty = true;
        }
        if self.drawn != Some(self.editor.view.camera) {
            self.dirty = true;
        }
        if self.dirty {
            self.draw(now);
        }
        let mut wait: Option<f64> = None;
        let mut soonest = |t: f64| wait = Some(wait.map_or(t, |w: f64| w.min(t)));
        if flying || self.editor.view.flight.is_some() || !self.revealed.is_empty() {
            soonest(0.0);
        }
        if let Some(t) = autoplay {
            soonest(t.max(0.0));
        }
        if let Some(t) = self.gif_due {
            soonest((t - now).max(0.0));
        }
        wait
    }

    fn scene_options(&self, now: f64) -> SceneOptions {
        let ed = &self.editor;
        let mut opts = SceneOptions { presenting: true, ..Default::default() };
        let dimmed = ed.dimmed_builds();
        opts.hidden = ed.hidden_builds().into_iter().filter(|id| !dimmed.contains(id)).collect();
        opts.opacity = self
            .revealed
            .iter()
            .filter_map(|(id, t)| {
                let a = ((now - t) / FADE).clamp(0.0, 1.0) as f32;
                (a < 1.0).then(|| (id.clone(), a))
            })
            .collect();
        opts.opacity.extend(dimmed.into_iter().map(|id| (id, DIMMED)));
        opts.time = Some(now);
        opts.async_images = false;
        opts.playing = self.playing.clone();
        opts
    }

    /// Draw the current view into [`Player::pixels`].
    pub fn draw(&mut self, now: f64) {
        let opts = self.scene_options(now);
        let vp = self.editor.view.viewport;
        let cam = self.editor.view.camera;
        let list = self.renderer.build_display_list(&self.editor, &cam, [vp.x, vp.y], self.ppp, &opts);
        self.gif_due = list.next_frame_in.map(|t| now + t.max(0.01));
        self.width_px = list.width_px;
        self.height_px = list.height_px;
        self.pixels = zebra_render::cpu::render_rgba(&list);
        self.drawn = Some(cam);
        self.dirty = false;
        self.frames = self.frames.wrapping_add(1);
    }

    /// Videos on step `i`: the step's element, everything inside it, and
    /// for view steps everything fully in view (as in the app).
    fn videos_on_step(&self, i: usize) -> Vec<ElementId> {
        let ed = &self.editor;
        let Some(s) = ed.doc.path.get(i) else { return vec![] };
        let ids: Vec<ElementId> = match &s.target {
            StepTarget::Element { id } => {
                let mut v = ed.frame_contents(id);
                v.push(id.clone());
                v
            }
            StepTarget::View { .. } | StepTarget::Overview => {
                let b = ed.camera_for_step(i).visible_bounds(ed.view.viewport);
                ed.elements_in(b).into_iter().filter(|e| b.contains_rect(&e.bounds())).map(|e| e.id.clone()).collect()
            }
        };
        ids.into_iter().filter(|id| matches!(ed.doc.get(id).map(|e| &e.kind), Some(ElementKind::Video(_)))).collect()
    }

    /// Where the page puts each video, in CSS pixels: a `w` x `h` box
    /// mapped by the 2D matrix `m` (CSS `matrix(a, b, c, d, e, f)`, origin
    /// at the box's top-left corner).
    fn video_boxes(&self) -> Vec<Value> {
        let ed = &self.editor;
        let vp = ed.view.viewport;
        let cam = ed.view.camera;
        let bounds = cam.visible_bounds(vp);
        let hidden = ed.hidden_builds();
        let dimmed = ed.dimmed_builds();
        let mut out = vec![];
        for e in ed.elements_in(bounds) {
            let ElementKind::Video(_) = &e.kind else { continue };
            if hidden.contains(&e.id) && !dimmed.contains(&e.id) {
                continue;
            }
            let a = zebra_render::display::element_affine(e, &cam, [vp.x, vp.y], 1.0)
                * kurbo::Affine::translate((-e.size.w / 2.0, -e.size.h / 2.0));
            let c = a.as_coeffs();
            let opacity = if dimmed.contains(&e.id) {
                DIMMED as f64
            } else {
                self.revealed.get(&e.id).map(|t| ((ed.view.now - t) / FADE).clamp(0.0, 1.0)).unwrap_or(1.0)
            };
            out.push(json!({"id": e.id, "w": e.size.w, "h": e.size.h, "m": c, "opacity": opacity}));
        }
        out
    }

    /// Everything the page shows around the canvas, as JSON.
    pub fn state(&self) -> Value {
        let ed = &self.editor;
        let p = &ed.view.present;
        let step = p.step;
        let hidden = ed.hidden_builds();
        let autoplay_videos: Vec<ElementId> = match step {
            Some(i) if ed.view.flight.is_none() => self
                .videos_on_step(i)
                .into_iter()
                .filter(|id| !hidden.contains(id))
                .filter(|id| matches!(ed.doc.get(id).map(|e| &e.kind), Some(ElementKind::Video(v)) if v.autoplay))
                .collect(),
            _ => vec![],
        };
        json!({
            "step": step,
            "build": p.build,
            "count": ed.doc.path.len(),
            "off_path": p.off_path,
            "at_end": ed.at_end(),
            "arrived": ed.view.flight.is_none(),
            "blank": p.blank.map(|b| match b { Blank::Black => "black", Blank::White => "white" }),
            "autoplay": ed.autoplay_interval().is_some(),
            "autoplay_paused": p.autoplay_paused,
            "voiceover": step.and_then(|i| ed.doc.path.get(i)).and_then(|s| s.voiceover.clone()),
            "autoplay_videos": autoplay_videos,
            "videos": self.video_boxes(),
            "mode_present": ed.view.mode == Mode::Present,
        })
    }
}

#[cfg(target_arch = "wasm32")]
pub mod ffi;
#[cfg(not(target_arch = "wasm32"))]
pub mod export;
