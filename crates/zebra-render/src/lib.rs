//! Rendering for Zebra.
//!
//! Pipeline: `Document + Camera` → [`build_display_list`] → [`DisplayList`]
//! (renderer-neutral) → backend: GPU (`gpu::VelloCanvas`, windowed app),
//! CPU (`render_png`, headless captures and tests) or PDF (`export_pdf`).
//!
//! CONTRACT (other crates code against these signatures; keep them stable):
//! * [`Renderer::new`] loads bundled fonts once; share one per process.
//! * [`Renderer::measure`] returns a `Box<dyn zebra_core::TextMeasure>` that the
//!   host installs into `Editor::measure` so layout, hit-testing and drawing agree.
//! * [`Renderer::capture`] renders any [`CaptureTarget`] off-screen to PNG
//!   bytes without touching the human's camera.
//! * [`Renderer::export_pdf`] one page per path step.
//! * `gpu::VelloCanvas` paints a display list into an egui `Ui` rect.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use zebra_core::{Camera, Editor, ElementId};

/// What to render.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureTarget {
    /// Exactly what the human's window shows now.
    Viewport,
    /// The agent's private camera.
    AgentView,
    /// A path step (0-based), with `build` fade-ins shown (default: all).
    Step { index: usize, #[serde(default)] build: Option<usize> },
    /// Fit one element.
    Element { id: ElementId },
    /// Fit everything.
    Overview,
    /// An explicit camera.
    Camera { camera: Camera },
}

/// Options for building a display list.
#[derive(Debug, Clone, Default)]
pub struct SceneOptions {
    /// Presenting: hide invisible frames, apply `hidden`.
    pub presenting: bool,
    /// Elements not drawn (pending fade-in builds).
    pub hidden: HashSet<ElementId>,
    /// Per-element opacity overrides 0..1 (fade-in animation).
    pub opacity: Vec<(ElementId, f32)>,
    /// Element being edited in a text box: drawn by the UI instead.
    pub skip: Option<ElementId>,
    /// Seconds on the animation clock: animated GIFs show the frame for this
    /// time. None shows their first frame (captures, PDF).
    pub time: Option<f64>,
    /// Decode pictures on worker threads, drawing a stand-in meanwhile
    /// (windowed app). False decodes before returning (captures, tests).
    pub async_images: bool,
    /// The current frame of each video that has one (playing or paused).
    pub video_frames: std::collections::HashMap<ElementId, std::sync::Arc<display::ImageLevel>>,
    /// Videos playing now (no ▶ badge).
    pub playing: HashSet<ElementId>,
}

pub mod display;
pub mod images;
mod clip;
pub use display::DisplayList;

pub mod text;
pub mod cpu;
mod pdf;
#[cfg(feature = "gpu")]
pub mod gpu;

pub struct Renderer {
    pub(crate) fonts: text::Fonts,
    pub(crate) images: images::ImageCache,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer { fonts: text::Fonts::new(), images: images::ImageCache::default() }
    }

    /// The decoded-image cache (budget, waker for async decodes, stats).
    pub fn images(&self) -> &images::ImageCache {
        &self.images
    }

    /// Text measurer to install in `Editor::measure`.
    pub fn measure(&self) -> Box<dyn zebra_core::TextMeasure> {
        self.fonts.measurer()
    }

    /// Build the display list for `editor.doc` seen through `camera` in a
    /// viewport of `viewport` logical points, at `pixels_per_point` (2 on Retina).
    pub fn build_display_list(
        &self,
        editor: &Editor,
        camera: &Camera,
        viewport: [f64; 2],
        pixels_per_point: f64,
        opts: &SceneOptions,
    ) -> DisplayList {
        display::build(self, editor, camera, viewport, pixels_per_point, opts)
    }

    /// Off-screen render to PNG. `width`x`height` in pixels.
    pub fn capture(&self, editor: &Editor, target: &CaptureTarget, width: u32, height: u32) -> Result<Vec<u8>, zebra_core::ZError> {
        cpu::capture(self, editor, target, width, height)
    }

    /// The display list a [`Renderer::capture`] of `target` at `width`x`height`
    /// pixels would rasterise (e.g. to render it on the GPU instead).
    pub fn capture_display_list(&self, editor: &Editor, target: &CaptureTarget, width: u32, height: u32) -> Result<DisplayList, zebra_core::ZError> {
        cpu::capture_list(self, editor, target, width, height)
    }

    /// Bundled font check: (family, faces found, files bundled).
    #[doc(hidden)]
    pub fn font_report(&self) -> Vec<(&'static str, usize, usize)> {
        self.fonts.bundled_report()
    }

    /// One page per path step (overview if the path is empty).
    pub fn export_pdf(&self, editor: &Editor) -> Result<Vec<u8>, zebra_core::ZError> {
        pdf::export(self, editor)
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}
