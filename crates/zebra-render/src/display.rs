//! Renderer-neutral display list.
//!
//! Built per frame from the f64 world model. Every element's local→device
//! transform is composed from `element centre − camera centre` (computed in
//! f64 first), so items are already rebased to the camera and f32-safe at any
//! zoom. Paths are in device pixels; text and images carry a similarity
//! transform from their own local space to device pixels.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kurbo::{Affine, BezPath, Cap, Ellipse, Join, Point, Rect, RoundedRect, Shape, Stroke};
use zebra_core::{
    Camera, Editor, Element, ElementKind, FrameStyle, ShapeKind, Theme, Vec2,
};

use crate::text::{wrap_for, TextLayout};
use crate::{Renderer, SceneOptions};

/// Straight (non-premultiplied) sRGB RGBA.
pub type Rgba = [u8; 4];

/// Commands in device pixels (already rebased to the camera, f32-safe).
#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub width_px: u32,
    pub height_px: u32,
    pub background: [u8; 4],
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    /// Fill a device-space path (non-zero rule).
    Fill { path: BezPath, color: Rgba },
    /// Stroke a device-space path (width and dashes in device pixels).
    Stroke { path: BezPath, color: Rgba, stroke: Stroke },
    /// Text: glyph runs in layout space, mapped to device by `transform`
    /// (a similarity: rotation + uniform scale + translation).
    /// `run_colors[i]` colours `layout.runs[i]` (empty: all `color`).
    Text { layout: Arc<TextLayout>, transform: Affine, color: Rgba, run_colors: Arc<[Rgba]> },
    /// An image level drawn with `transform` mapping image pixels to device.
    Image { image: Arc<ImageLevel>, transform: Affine },
    /// Group opacity for the items up to the matching [`Item::PopOpacity`].
    PushOpacity(f32),
    PopOpacity,
}

/// One decoded mip level of an image asset (premultiplied RGBA8).
pub struct ImageLevel {
    pub asset: String,
    pub width: u32,
    pub height: u32,
    /// For vello_cpu.
    pub pixmap: Arc<vello_cpu::Pixmap>,
    /// For vello (GPU). The blob id is stable, so vello's atlas caches it.
    pub data: vello_cpu::peniko::ImageData,
}

impl std::fmt::Debug for ImageLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ImageLevel({} {}x{})", self.asset, self.width, self.height)
    }
}

impl DisplayList {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Decoded images, keyed by asset key. `None` = undecodable (placeholder).
/// Mip levels of one asset, largest first.
pub type ImageLevels = Arc<Vec<Arc<ImageLevel>>>;

#[derive(Default)]
pub(crate) struct ImageCache {
    map: Mutex<HashMap<String, Option<ImageLevels>>>,
}

/// Largest dimension kept for level 0.
const MAX_IMAGE_DIM: u32 = 8192;

impl ImageCache {
    pub(crate) fn levels(&self, editor: &Editor, key: &str) -> Option<ImageLevels> {
        if let Some(v) = self.map.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
            return v.clone();
        }
        let decoded = editor.doc.assets.get(key).and_then(|a| decode_levels(key, &a.bytes)).map(Arc::new);
        self.map.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_string(), decoded.clone());
        decoded
    }
}

fn decode_levels(key: &str, bytes: &[u8]) -> Option<Vec<Arc<ImageLevel>>> {
    let img = image::load_from_memory(bytes).ok()?;
    let mut rgba = img.to_rgba8();
    if rgba.width().max(rgba.height()) > MAX_IMAGE_DIM {
        let k = MAX_IMAGE_DIM as f64 / rgba.width().max(rgba.height()) as f64;
        let (w, h) = (((rgba.width() as f64 * k) as u32).max(1), ((rgba.height() as f64 * k) as u32).max(1));
        rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Triangle);
    }
    let mut levels = vec![];
    loop {
        levels.push(Arc::new(make_level(key, &rgba)));
        let (w, h) = (rgba.width(), rgba.height());
        if w.max(h) <= 32 {
            break;
        }
        rgba = image::imageops::resize(&rgba, (w / 2).max(1), (h / 2).max(1), image::imageops::FilterType::Triangle);
    }
    Some(levels)
}

fn make_level(key: &str, rgba: &image::RgbaImage) -> ImageLevel {
    use vello_cpu::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
    let (w, h) = (rgba.width(), rgba.height());
    let mut px = rgba.as_raw().clone();
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u16;
        if a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u16 * a + 127) / 255) as u8;
            }
        }
    }
    let pixmap = vello_cpu::Pixmap::from_parts(
        px.clone(),
        w as u16,
        h as u16,
        vello_cpu::PixelMetadata { alpha_type: vello_cpu::peniko::ImageAlphaType::AlphaPremultiplied, may_have_transparency: true },
    );
    let data = ImageData {
        data: Blob::from(px),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: w,
        height: h,
    };
    ImageLevel { asset: key.to_string(), width: w, height: h, pixmap: Arc::new(pixmap), data }
}

/// Elements whose on-screen size is below this (device px) are skipped.
pub const MIN_PIXELS: f64 = 0.5;

/// Local → device transform for an element.
pub fn element_affine(e: &Element, cam: &Camera, vp: [f64; 2], ppp: f64) -> Affine {
    let dx = e.transform.x - cam.cx;
    let dy = e.transform.y - cam.cy;
    Affine::translate((vp[0] * 0.5 * ppp, vp[1] * 0.5 * ppp))
        * Affine::scale(cam.zoom * ppp)
        * Affine::rotate(-cam.rotation.to_radians())
        * Affine::translate((dx, dy))
        * Affine::rotate(e.transform.rotation.to_radians())
        * Affine::scale(e.transform.scale)
}

/// Uniform scale factor of a similarity transform.
pub fn affine_scale(a: &Affine) -> f64 {
    a.determinant().abs().sqrt()
}

/// Extra world-space margin an element draws outside its box.
fn overhang(e: &Element) -> f64 {
    let local = match &e.kind {
        ElementKind::Ink(i) => i.width,
        ElementKind::Shape(s) => s.stroke_width,
        _ => 0.0,
    };
    local * e.transform.scale
}

pub(crate) fn build(r: &Renderer, editor: &Editor, cam: &Camera, vp: [f64; 2], ppp: f64, o: &SceneOptions) -> DisplayList {
    build_with(r, editor, cam, vp, ppp, o, false)
}

/// `full_res_images`: always choose image level 0 (PDF).
pub(crate) fn build_with(
    r: &Renderer,
    editor: &Editor,
    cam: &Camera,
    vp: [f64; 2],
    ppp: f64,
    o: &SceneOptions,
    full_res_images: bool,
) -> DisplayList {
    let theme = editor.theme();
    let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
    let width_px = (vp[0] * ppp).round().max(0.0) as u32;
    let height_px = (vp[1] * ppp).round().max(0.0) as u32;
    let mut list = DisplayList { width_px, height_px, background: theme.color(&theme.background), items: vec![] };
    if width_px == 0 || height_px == 0 || !(cam.zoom.is_finite() && cam.zoom > 0.0) {
        return list;
    }
    r.fonts.next_generation();
    let view = cam.visible_bounds(Vec2::new(vp[0], vp[1]));
    let px_per_world = cam.zoom * ppp;
    let ctx = Ctx { r, editor, theme: &theme, presenting: o.presenting, full_res_images, ppp };

    for e in editor.doc.ordered() {
        if o.skip.as_deref() == Some(e.id.as_str()) || (o.presenting && o.hidden.contains(&e.id)) {
            continue;
        }
        if o.presenting && matches!(&e.kind, ElementKind::Frame(f) if f.style == FrameStyle::Invisible) {
            continue;
        }
        let pad = overhang(e);
        let b = e.bounds().inflate(pad);
        if !b.intersects(&view) {
            continue;
        }
        let (ww, wh) = e.world_size();
        if (ww.max(wh) + 2.0 * pad) * px_per_world < MIN_PIXELS {
            continue;
        }
        let opacity = o.opacity.iter().rev().find(|(id, _)| *id == e.id).map(|(_, a)| a.clamp(0.0, 1.0)).unwrap_or(1.0);
        if opacity <= 0.0 {
            continue;
        }
        let a = element_affine(e, cam, vp, ppp);
        let start = list.items.len();
        if opacity < 1.0 {
            list.items.push(Item::PushOpacity(opacity));
        }
        ctx.element(e, &a, &mut list.items);
        if opacity < 1.0 {
            if list.items.len() == start + 1 {
                list.items.pop();
            } else {
                list.items.push(Item::PopOpacity);
            }
        }
    }
    crate::clip::clip_items(&mut list.items, width_px as f64, height_px as f64);
    list
}

struct Ctx<'a> {
    r: &'a Renderer,
    editor: &'a Editor,
    theme: &'a Theme,
    presenting: bool,
    full_res_images: bool,
    ppp: f64,
}

fn local_rect(e: &Element) -> Rect {
    let (hw, hh) = (e.size.w / 2.0, e.size.h / 2.0);
    Rect::new(-hw, -hh, hw, hh)
}

fn fill(items: &mut Vec<Item>, shape: &impl Shape, a: &Affine, color: Rgba) {
    if color[3] == 0 {
        return;
    }
    let mut path = shape.to_path(0.01);
    path.apply_affine(*a);
    items.push(Item::Fill { path, color });
}

/// Stroke with `width` in local units.
fn stroke(items: &mut Vec<Item>, shape: &impl Shape, a: &Affine, color: Rgba, mut st: Stroke) {
    if color[3] == 0 || st.width <= 0.0 {
        return;
    }
    let mut path = shape.to_path(0.01);
    path.apply_affine(*a);
    let k = affine_scale(a);
    st.width *= k;
    if !st.dash_pattern.is_empty() {
        for d in st.dash_pattern.iter_mut() {
            *d *= k;
        }
        st.dash_offset *= k;
    }
    items.push(Item::Stroke { path, color, stroke: st });
}

fn with_alpha(mut c: Rgba, k: f32) -> Rgba {
    c[3] = (c[3] as f32 * k).round().clamp(0.0, 255.0) as u8;
    c
}

impl Ctx<'_> {
    fn element(&self, e: &Element, a: &Affine, items: &mut Vec<Item>) {
        let th = self.theme;
        let lr = local_rect(e);
        let (w, h) = (e.size.w, e.size.h);
        match &e.kind {
            ElementKind::Frame(f) => {
                let accent = f.color.as_ref().map(|c| th.color(c));
                match f.style {
                    FrameStyle::Bracket => {
                        let color = accent.unwrap_or_else(|| th.color(&th.frame));
                        let t = (w.min(h) * 0.012).max(1e-9);
                        let arm = (w * 0.05).max(t * 2.0).min(w * 0.5);
                        let (hw, hh) = (w / 2.0 - t / 2.0, h / 2.0 - t / 2.0);
                        let st = Stroke::new(t).with_join(Join::Miter).with_caps(Cap::Butt);
                        for sx in [-1.0, 1.0] {
                            let mut p = BezPath::new();
                            p.move_to((sx * (hw - arm), -hh));
                            p.line_to((sx * hw, -hh));
                            p.line_to((sx * hw, hh));
                            p.line_to((sx * (hw - arm), hh));
                            stroke(items, &p, a, color, st.clone());
                        }
                    }
                    FrameStyle::Circle => {
                        let color = accent.unwrap_or_else(|| th.color(&th.frame_fill));
                        fill(items, &Ellipse::from_rect(lr), a, color);
                    }
                    FrameStyle::Rectangle => {
                        let color = accent.unwrap_or_else(|| th.color(&th.frame_fill));
                        fill(items, &RoundedRect::from_rect(lr, w.min(h) * 0.03), a, color);
                    }
                    FrameStyle::Invisible => {
                        if !self.presenting {
                            let color = with_alpha(accent.unwrap_or_else(|| th.color(&th.frame)), 0.6);
                            let s = affine_scale(a);
                            // 1 point wide, 6/4 point dashes, whatever the zoom.
                            let px = self.ppp / s;
                            let st = Stroke::new(px).with_dashes(0.0, [6.0 * px, 4.0 * px]);
                            stroke(items, &lr, a, color, st);
                        }
                    }
                }
            }
            ElementKind::Text(t) => {
                if t.text.is_empty() {
                    return;
                }
                let layout = self.r.fonts.layout(t, th, wrap_for(t, e.size.w));
                let style = th.text_style(t.style);
                let color = th.color(t.color.as_ref().unwrap_or(&style.color));
                let transform = *a * Affine::translate((-w / 2.0, -h / 2.0));
                // Per-format colours: explicit colour, else links in the accent.
                let fmt_colors: Vec<Rgba> = layout
                    .formats
                    .iter()
                    .map(|f| match (&f.color, &f.link) {
                        (Some(c), _) => th.color(c),
                        (None, Some(_)) => th.color(&"theme:accent".to_string()),
                        _ => color,
                    })
                    .collect();
                let run_colors: Arc<[Rgba]> = if layout.runs.iter().any(|r| r.format.is_some()) {
                    layout.runs.iter().map(|r| r.format.and_then(|i| fmt_colors.get(i as usize).copied()).unwrap_or(color)).collect()
                } else {
                    Arc::from([])
                };
                let decos: Vec<(kurbo::Rect, Rgba)> = layout
                    .decorations
                    .iter()
                    .map(|d| {
                        let r = kurbo::Rect::new(d.x0 as f64, (d.y - d.thickness / 2.0) as f64, d.x1 as f64, (d.y + d.thickness / 2.0) as f64);
                        (r, fmt_colors.get(d.format as usize).copied().unwrap_or(color))
                    })
                    .collect();
                items.push(Item::Text { layout, transform, color, run_colors });
                for (r, c) in decos {
                    fill(items, &r, &transform, c);
                }
            }
            ElementKind::Shape(s) => {
                let closed = !matches!(s.shape, ShapeKind::Line | ShapeKind::Arrow);
                let (fill_c, stroke_c) = match (&s.fill, &s.stroke) {
                    (None, None) if closed => (Some(th.color(&"theme:shape1".to_string())), None),
                    (None, None) => (None, Some(th.color(&"theme:shape1".to_string()))),
                    (f, st) => (f.as_ref().map(|c| th.color(c)), st.as_ref().map(|c| th.color(c))),
                };
                let sw = s.stroke_width.max(0.0);
                match s.shape {
                    ShapeKind::Rectangle | ShapeKind::Ellipse | ShapeKind::Triangle => {
                        let path = match s.shape {
                            ShapeKind::Rectangle => lr.to_path(0.01),
                            ShapeKind::Ellipse => Ellipse::from_rect(lr).to_path(0.01),
                            _ => {
                                let mut p = BezPath::new();
                                p.move_to((0.0, -h / 2.0));
                                p.line_to((w / 2.0, h / 2.0));
                                p.line_to((-w / 2.0, h / 2.0));
                                p.close_path();
                                p
                            }
                        };
                        if let Some(c) = fill_c {
                            fill(items, &path, a, c);
                        }
                        if let Some(c) = stroke_c {
                            stroke(items, &path, a, c, Stroke::new(sw).with_join(Join::Miter));
                        }
                    }
                    ShapeKind::Line | ShapeKind::Arrow => {
                        // Lines use the stroke colour, else the fill colour.
                        let Some(c) = stroke_c.or(fill_c) else { return };
                        let hw = w / 2.0;
                        let st = Stroke::new(sw).with_caps(Cap::Round);
                        if s.shape == ShapeKind::Line {
                            stroke(items, &kurbo::Line::new((-hw, 0.0), (hw, 0.0)), a, c, st);
                        } else {
                            let head = (sw * 4.0).max(w * 0.06).min(w * 0.45);
                            let half = head * 0.6;
                            let shaft_end = hw - head * 0.9;
                            stroke(items, &kurbo::Line::new((-hw, 0.0), (shaft_end, 0.0)), a, c, st);
                            let mut p = BezPath::new();
                            p.move_to((hw, 0.0));
                            p.line_to((hw - head, -half));
                            p.line_to((hw - head, half));
                            p.close_path();
                            fill(items, &p, a, c);
                        }
                    }
                }
            }
            ElementKind::Image(img) => {
                let levels = self.r.images.levels(self.editor, &img.asset);
                let Some(levels) = levels else {
                    // Placeholder: soft box with a cross.
                    fill(items, &lr, a, [200, 200, 200, 255]);
                    let mut p = BezPath::new();
                    p.move_to((lr.x0, lr.y0));
                    p.line_to((lr.x1, lr.y1));
                    p.move_to((lr.x1, lr.y0));
                    p.line_to((lr.x0, lr.y1));
                    stroke(items, &p, a, [150, 150, 150, 255], Stroke::new(w.min(h) * 0.01));
                    return;
                };
                let on_screen = w * affine_scale(a);
                let level = if self.full_res_images {
                    levels[0].clone()
                } else {
                    levels.iter().rev().find(|l| l.width as f64 >= on_screen).unwrap_or(&levels[0]).clone()
                };
                let transform = *a
                    * Affine::translate((-w / 2.0, -h / 2.0))
                    * Affine::scale_non_uniform(w / level.width as f64, h / level.height as f64);
                items.push(Item::Image { image: level, transform });
            }
            ElementKind::Ink(ink) => {
                if ink.points.is_empty() {
                    return;
                }
                let mut color = th.color(&ink.color);
                if ink.highlighter {
                    color = with_alpha(color, 0.35);
                }
                let mut p = BezPath::new();
                p.move_to(Point::new(ink.points[0].x, ink.points[0].y));
                if ink.points.len() == 1 {
                    // A dot.
                    p.line_to(Point::new(ink.points[0].x + 1e-3, ink.points[0].y));
                }
                for q in &ink.points[1..] {
                    p.line_to(Point::new(q.x, q.y));
                }
                let st = Stroke::new(ink.width).with_caps(Cap::Round).with_join(Join::Round);
                stroke(items, &p, a, color, st);
            }
        }
    }
}
