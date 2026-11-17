//! vello_cpu backend: headless captures and tests.
use kurbo::Affine;
use vello_cpu::peniko::{Color, Extend, ImageQuality, ImageSampler};
use vello_cpu::{Image, ImageSource, Pixmap, RenderContext, RenderSettings, Resources};
use zebra_core::{Camera, Editor, Mode, ZError};

use crate::display::{affine_scale, DisplayList, Item, Rgba};
use crate::{CaptureTarget, Renderer, SceneOptions};

/// Largest capture side in pixels.
pub const MAX_CAPTURE: u32 = 8192;

pub(crate) fn color(c: Rgba) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Rasterise a display list to a premultiplied pixmap.
pub fn render_pixmap(list: &DisplayList) -> Pixmap {
    let (w, h) = (list.width_px.clamp(1, u16::MAX as u32) as u16, list.height_px.clamp(1, u16::MAX as u32) as u16);
    let mut ctx = RenderContext::new_with(w, h, RenderSettings::default());
    let mut res = Resources::new();
    ctx.set_paint(color(list.background));
    ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w as f64, h as f64));
    for item in &list.items {
        ctx.reset_transform();
        match item {
            Item::Fill { path, color: c } => {
                ctx.set_paint(color(*c));
                ctx.fill_path(path);
            }
            Item::Stroke { path, color: c, stroke } => {
                ctx.set_paint(color(*c));
                ctx.set_stroke(stroke.clone());
                ctx.stroke_path(path);
            }
            Item::Text { layout, transform, color: c } => {
                ctx.set_paint(color(*c));
                let (s, rest) = split_scale(transform);
                ctx.set_transform(rest);
                for run in &layout.runs {
                    let glyphs = run.glyphs.iter().map(|g| vello_cpu::Glyph {
                        id: g.id,
                        x: g.x * s as f32,
                        y: g.y * s as f32,
                    });
                    let mut b = ctx
                        .glyph_run(&mut res, &run.font)
                        .font_size(run.font_size * s as f32)
                        .hint(false)
                        .normalized_coords(&run.coords);
                    if let Some(skew) = run.skew {
                        // Synthetic italic; vello_cpu glyph space is y-down.
                        b = b.glyph_transform(Affine::skew(-(skew as f64).to_radians().tan(), 0.0));
                    }
                    let _ = b.fill_glyphs(glyphs);
                }
            }
            Item::Image { image, transform } => {
                let img = Image {
                    image: ImageSource::Pixmap(image.pixmap()),
                    sampler: ImageSampler {
                        x_extend: Extend::Pad,
                        y_extend: Extend::Pad,
                        quality: ImageQuality::Medium,
                        alpha: 1.0,
                    },
                };
                ctx.set_transform(*transform);
                ctx.set_paint(img);
                ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, image.width as f64, image.height as f64));
            }
            Item::PushOpacity(a) => ctx.push_opacity_layer(*a),
            Item::PopOpacity | Item::PopClip => ctx.pop_layer(),
            Item::PushClip(path) => ctx.push_clip_layer(path),
        }
    }
    ctx.flush();
    let mut pixmap = Pixmap::new(w, h);
    ctx.render(&mut pixmap, &mut res);
    pixmap
}

/// Rasterise to straight-alpha RGBA8 rows (`width_px * height_px * 4` bytes).
pub fn render_rgba(list: &DisplayList) -> Vec<u8> {
    render_pixmap(list).take_rgba8(vello_cpu::peniko::ImageAlphaType::Alpha)
}

/// Split a similarity transform into (uniform scale, transform without it).
pub(crate) fn split_scale(a: &Affine) -> (f64, Affine) {
    let s = affine_scale(a);
    if s > 0.0 && s.is_finite() { (s, *a * Affine::scale(1.0 / s)) } else { (1.0, *a) }
}

/// What a capture shows: camera, logical viewport, device scale, scene options.
pub(crate) struct CaptureView {
    pub camera: Camera,
    pub viewport: [f64; 2],
    pub ppp: f64,
    pub opts: SceneOptions,
}

/// Logical viewport with the capture's aspect ratio that contains `base`.
fn fit_viewport(base: [f64; 2], w: u32, h: u32) -> ([f64; 2], f64) {
    let base = [base[0].max(1.0), base[1].max(1.0)];
    let ppp = (w as f64 / base[0]).min(h as f64 / base[1]);
    ([w as f64 / ppp, h as f64 / ppp], ppp)
}

fn fit_element(e: &zebra_core::Element, vp: [f64; 2]) -> Camera {
    let (w, h) = e.world_size();
    let margin = if e.is_frame() { 0.04 } else { 0.12 };
    Camera::fit(
        &zebra_core::ViewRect { cx: e.transform.x, cy: e.transform.y, width: w, height: h, rotation: e.transform.rotation },
        zebra_core::Vec2::new(vp[0], vp[1]),
        margin,
    )
}

fn fit_all(editor: &Editor, vp: [f64; 2]) -> Camera {
    match editor.doc.bounds() {
        Some(b) => Camera::fit_rect(&b, zebra_core::Vec2::new(vp[0], vp[1]), 0.06),
        None => Camera::default(),
    }
}

/// Same framing rules as `Editor::camera_for_target`, for any viewport.
pub(crate) fn camera_for_step(editor: &Editor, i: usize, vp: [f64; 2]) -> Camera {
    use zebra_core::StepTarget;
    match editor.doc.path.get(i).map(|s| &s.target) {
        Some(StepTarget::Element { id }) => match editor.doc.get(id) {
            Some(e) => fit_element(e, vp),
            None => fit_all(editor, vp),
        },
        Some(StepTarget::View { view }) => Camera::fit(view, zebra_core::Vec2::new(vp[0], vp[1]), 0.0),
        _ => fit_all(editor, vp),
    }
}

pub(crate) fn capture_view(editor: &Editor, target: &CaptureTarget, w: u32, h: u32) -> Result<CaptureView, ZError> {
    let base = [editor.view.viewport.x, editor.view.viewport.y];
    let (vp, ppp) = fit_viewport(base, w, h);
    let mut opts = SceneOptions::default();
    let camera = match target {
        CaptureTarget::Viewport => {
            if editor.view.mode == Mode::Present {
                opts.presenting = true;
                opts.hidden = editor.hidden_builds();
            }
            editor.view.camera
        }
        CaptureTarget::AgentView => editor.view.agent_camera,
        CaptureTarget::Camera { camera } => *camera,
        CaptureTarget::Step { index, build } => {
            if *index >= editor.doc.path.len() {
                return Err(ZError::new(
                    "not_found",
                    format!("step {index} does not exist (the path has {} steps)", editor.doc.path.len()),
                ));
            }
            opts.presenting = true;
            opts.hidden = editor.hidden_builds_at(*index, build.unwrap_or(usize::MAX));
            camera_for_step(editor, *index, vp)
        }
        CaptureTarget::Element { id } => {
            let e = editor.doc.get(id).ok_or_else(|| ZError::new("not_found", format!("no element with id {id:?}")))?;
            fit_element(e, vp)
        }
        CaptureTarget::Overview => fit_all(editor, vp),
    };
    Ok(CaptureView { camera, viewport: vp, ppp, opts })
}

pub(crate) fn capture(r: &Renderer, editor: &Editor, target: &CaptureTarget, w: u32, h: u32) -> Result<Vec<u8>, ZError> {
    let list = capture_list(r, editor, target, w, h)?;
    let pixmap = render_pixmap(&list);
    pixmap.into_png().map_err(|e| ZError::new("render_failed", format!("PNG encoding failed: {e}")))
}

pub(crate) fn capture_list(r: &Renderer, editor: &Editor, target: &CaptureTarget, w: u32, h: u32) -> Result<DisplayList, ZError> {
    if w == 0 || h == 0 || w > MAX_CAPTURE || h > MAX_CAPTURE {
        return Err(ZError::new("invalid", format!("capture size must be 1..={MAX_CAPTURE} pixels per side, got {w}x{h}")));
    }
    let cv = capture_view(editor, target, w, h)?;
    let mut list = r.build_display_list(editor, &cv.camera, cv.viewport, cv.ppp, &cv.opts);
    // Rounding can be off by one; the PNG must be exactly w x h.
    list.width_px = w;
    list.height_px = h;
    Ok(list)
}
