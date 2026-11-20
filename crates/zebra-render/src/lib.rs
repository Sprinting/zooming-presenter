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
    /// An image in crop mode: its whole picture is drawn faintly behind the
    /// crop, so the person sees what they can uncrop.
    pub ghost: Option<ElementId>,
    /// Video cards (YouTube, Vimeo) with a resume point, 0..1 of the way
    /// through: a thin line along the card's bottom shows it. Session
    /// state, never saved.
    pub card_progress: std::collections::HashMap<ElementId, f32>,
}

pub mod display;
pub mod font_subset;
pub mod font_rights;
pub mod images;
pub mod pages;
pub mod svg;
mod clip;
pub use clip::{clip_polyline, frame_cost, MAX_FRAME_COST};
pub use display::DisplayList;

pub mod text;
pub mod cpu;
mod pdf;
pub mod shapes;
#[cfg(feature = "gpu")]
pub mod gpu;

/// See [`Renderer::web_text`].
#[derive(Debug, Default)]
pub struct WebText {
    /// Font files to carry, cut to the glyphs used.
    pub fonts: Vec<Vec<u8>>,
    /// Text elements to draw as pictures instead.
    pub rasterise: Vec<ElementId>,
    /// For the person: fonts with licence trouble.
    pub notes: Vec<String>,
    /// SF Pro, New York... families the deck uses (Apple licenses them for
    /// interface mock-ups only): worth a warning.
    pub apple_ui_families: Vec<String>,
}

/// See [`Renderer::rasterise_text`].
pub struct RasterText {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Local units added on each side of the element's box.
    pub pad: f64,
}

pub struct Renderer {
    pub(crate) fonts: text::Fonts,
    pub(crate) images: images::ImageCache,
    /// Draws imported PDF pages sharp when zoomed in (see [`pages`]).
    page_raster: std::sync::RwLock<Option<pages::PageRaster>>,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer { fonts: text::Fonts::new(), images: images::ImageCache::default(), page_raster: Default::default() }
    }

    /// Give the renderer a way to draw PDF pages (the app has PDFium; the
    /// web player has none and draws each page's picture).
    pub fn set_page_raster(&self, f: Option<pages::PageRaster>) {
        *self.page_raster.write().unwrap_or_else(|e| e.into_inner()) = f;
    }

    pub(crate) fn page_raster(&self) -> Option<pages::PageRaster> {
        self.page_raster.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The decoded-image cache (budget, waker for async decodes, stats).
    pub fn images(&self) -> &images::ImageCache {
        &self.images
    }

    /// The link URL under world point `p` on text element `e`, if any.
    pub fn link_at(&self, editor: &Editor, e: &zebra_core::Element, p: zebra_core::Vec2) -> Option<String> {
        let zebra_core::ElementKind::Text(t) = &e.kind else { return None };
        if t.spans.iter().all(|s| s.format.link.is_none()) {
            return None;
        }
        let tr = &e.transform;
        let local = (p - zebra_core::Vec2::new(tr.x, tr.y)).rotate_deg(-tr.rotation) * (1.0 / tr.scale);
        let layout = self.fonts.layout(t, &editor.theme(), text::wrap_for(t, e.size.w));
        let (x, y) = (local.x + e.size.w / 2.0, local.y + e.size.h / 2.0);
        layout.link_at(x as f32, y as f32).map(str::to_string)
    }

    /// Register a font file for text layout (the web player's fonts).
    pub fn add_font(&self, bytes: Vec<u8>) {
        self.fonts.add_font(bytes)
    }

    /// The font files the document's text uses, each once (bundled,
    /// embedded in a theme, or a system fallback for other scripts).
    /// The fonts a page showing `editor`'s deck may carry, each cut down to
    /// the glyphs the deck shows: see [`Renderer::web_text`].
    pub fn font_files_used(&self, editor: &Editor) -> Vec<Vec<u8>> {
        self.web_text(editor).fonts
    }

    /// How a web page shows `editor`'s text. Fonts it may carry (see
    /// [`font_rights`]) ship cut to the glyphs used ([`font_subset`]); text
    /// set in any other font (Apple's, or one whose licence has no web
    /// rights) is listed in `rasterise`, to be drawn into pictures here on
    /// the person's Mac ([`Renderer::rasterise_text`]): the font file never
    /// leaves the machine.
    pub fn web_text(&self, editor: &Editor) -> WebText {
        use font_rights::{web_use, Reason, WebUse};
        let by_element = self.fonts.faces_by_element(editor);
        let mut rights: Vec<((u64, u32), WebUse)> = vec![];
        let mut use_of = |f: &text::FontData| -> WebUse {
            let key = (f.data.id(), f.index);
            if let Some((_, u)) = rights.iter().find(|(k, _)| *k == key) {
                return u.clone();
            }
            let u = if text::Fonts::is_bundled(f) { WebUse::Ship } else { web_use(f.data.as_ref(), f.index) };
            rights.push((key, u.clone()));
            u
        };
        let mut out = WebText::default();
        let mut ship: Vec<(text::FontData, std::collections::BTreeSet<u16>)> = vec![];
        for (id, faces) in by_element {
            let blocked: Vec<Reason> = faces.iter().filter_map(|(f, _)| match use_of(f) {
                WebUse::Rasterise(r) => Some(r),
                WebUse::Ship => None,
            }).collect();
            for r in &blocked {
                if let Reason::AppleUiFont(family) = r {
                    if !out.apple_ui_families.contains(family) {
                        out.apple_ui_families.push(family.clone());
                    }
                    let note = format!("{family} is licensed by Apple only for interface mock-ups; its text was drawn as pictures. Consider Inter.");
                    if !out.notes.contains(&note) {
                        out.notes.push(note);
                    }
                }
            }
            let is_text = matches!(editor.doc.get(&id).map(|e| &e.kind), Some(zebra_core::ElementKind::Text(_)));
            if !blocked.is_empty() && is_text {
                out.rasterise.push(id);
                continue;
            }
            if !blocked.is_empty() {
                out.notes.push(format!("The label of {id} uses a font a web page can't carry; the page shows it in another font."));
            }
            for (f, glyphs) in faces {
                if use_of(&f) != WebUse::Ship {
                    continue;
                }
                match ship.iter_mut().find(|(g, _)| g.data.id() == f.data.id() && g.index == f.index) {
                    Some((_, g)) => g.extend(glyphs),
                    None => ship.push((f, glyphs)),
                }
            }
        }
        // A font that can't be cut is left out (its text falls back to
        // another font on the page), unless it is one of Zebra's own.
        out.fonts = ship
            .iter()
            .filter_map(|(f, glyphs)| {
                font_subset::subset(f.data.as_ref(), f.index, glyphs).or_else(|| text::Fonts::is_bundled(f).then(|| f.data.as_ref().to_vec()))
            })
            .collect();
        out
    }

    /// The camera path step `i` shows (as a PDF page or capture would).
    pub fn step_camera(&self, editor: &Editor, i: usize) -> Option<Camera> {
        (i < editor.doc.path.len()).then(|| cpu::camera_for_step(editor, i, [editor.view.viewport.x.max(1.0), editor.view.viewport.y.max(1.0)]))
    }

    /// Text element `id` drawn alone into a transparent PNG about `px`
    /// pixels wide, for a page that may not carry its font. The picture
    /// covers the element's box plus `pad` (local units) on every side, so
    /// overhanging letters aren't cut; place it with the element's own
    /// position, scale and rotation.
    pub fn rasterise_text(&self, editor: &Editor, id: &str, px: u32) -> Option<RasterText> {
        let e = editor.doc.get(id)?.clone();
        let pad = (e.size.h * 0.2).max(1.0);
        let (bw, bh) = ((e.size.w + 2.0 * pad) * e.transform.scale, (e.size.h + 2.0 * pad) * e.transform.scale);
        if !(bw > 0.0 && bh > 0.0 && bw.is_finite() && bh.is_finite()) {
            return None;
        }
        let w = px.clamp(16, cpu::MAX_CAPTURE);
        let zoom = w as f64 / bw;
        let h = ((bh * zoom).ceil() as u32).clamp(1, cpu::MAX_CAPTURE);
        let mut ed = Editor::default();
        ed.measure = self.measure();
        ed.doc.theme = editor.doc.theme.clone();
        ed.doc.themes = editor.doc.themes.clone();
        // Theme fonts embedded in the document.
        for t in &editor.doc.themes {
            for k in t.fonts.values().flatten() {
                if let Some(a) = editor.doc.assets.get(k) {
                    ed.doc.assets.insert(k.clone(), a.clone());
                }
            }
        }
        let mut e = e;
        e.transform.x = 0.0;
        e.transform.y = 0.0;
        e.transform.rotation = 0.0;
        e.placeholder = false;
        ed.doc.z_order = vec![e.id.clone()];
        ed.doc.elements.insert(e.id.clone(), e);
        ed.view.viewport = zebra_core::Vec2::new(w as f64, h as f64);
        let camera = Camera { cx: 0.0, cy: 0.0, zoom, rotation: 0.0 };
        let opts = SceneOptions { presenting: true, ..Default::default() };
        let mut list = self.build_display_list(&ed, &camera, [w as f64, h as f64], 1.0, &opts);
        list.width_px = w;
        list.height_px = h;
        list.background = [0, 0, 0, 0];
        let rgba = cpu::render_rgba(&list);
        let img = image::RgbaImage::from_raw(w, h, rgba)?;
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(RasterText { png, width: w, height: h, pad })
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

    /// Cache sizes for the `perf` query: name -> bytes or count.
    pub fn cache_stats(&self) -> Vec<(&'static str, u64)> {
        let st = self.images.stats();
        vec![
            ("image_kb", st.bytes as u64 / 1024),
            ("image_budget_kb", st.budget as u64 / 1024),
            ("images", st.entries as u64),
            ("text_layouts", self.fonts.cached_layouts() as u64),
        ]
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
