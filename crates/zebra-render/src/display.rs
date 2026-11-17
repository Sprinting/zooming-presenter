//! Renderer-neutral display list.
//!
//! Built per frame from the f64 world model. Every element's local→device
//! transform is composed from `element centre − camera centre` (computed in
//! f64 first), so items are already rebased to the camera and f32-safe at any
//! zoom. Paths are in device pixels; text and images carry a similarity
//! transform from their own local space to device pixels.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kurbo::{Affine, BezPath, Cap, Join, Point, Rect, RoundedRect, Shape, Stroke};
use zebra_core::{
    Camera, Dash, Editor, Element, ElementKind, FrameStyle, ShapeKind, Theme, Vec2,
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
    /// (images decoded, bytes of decoded pixels across all mip levels).
    pub(crate) fn stats(&self) -> (usize, u64) {
        let m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = m.values().flatten().flat_map(|l| l.iter()).map(|l| l.width as u64 * l.height as u64 * 4).sum();
        (m.len(), bytes)
    }

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

/// The smallest mip level at least `on_screen_px` wide.
fn pick_level(levels: &ImageLevels, on_screen_px: f64, full_res: bool) -> Arc<ImageLevel> {
    if full_res {
        return levels[0].clone();
    }
    levels.iter().rev().find(|l| l.width as f64 >= on_screen_px).unwrap_or(&levels[0]).clone()
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
        ElementKind::Icon(i) => i.stroke_width * e.size.w.min(e.size.h) / 24.0,
        // Arrowheads and the label reach past the route's box.
        ElementKind::Connector(c) => (c.width * 4.0).max(10.0) + if c.label.is_empty() { 0.0 } else { 200.0 },
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
    r.fonts.register_doc_fonts(&editor.doc);
    let view = cam.visible_bounds(Vec2::new(vp[0], vp[1]));
    let px_per_world = cam.zoom * ppp;
    let ctx = Ctx { r, editor, theme: &theme, presenting: o.presenting, full_res_images, ppp };
    ctx.backdrop(cam, vp, &mut list.items);

    for e in editor.doc.ordered() {
        if o.skip.as_deref() == Some(e.id.as_str()) || (o.presenting && o.hidden.contains(&e.id)) {
            continue;
        }
        if o.presenting && matches!(&e.kind, ElementKind::Frame(f) if f.style == FrameStyle::Invisible) {
            continue;
        }
        // Layout placeholders: faint while editing, gone when presenting.
        if o.presenting && e.placeholder {
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
        let mut opacity = o.opacity.iter().rev().find(|(id, _)| *id == e.id).map(|(_, a)| a.clamp(0.0, 1.0)).unwrap_or(1.0);
        if e.placeholder {
            opacity *= PLACEHOLDER_OPACITY;
        }
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

/// Darken (k < 1) a colour, keeping alpha.
fn shade(c: Rgba, k: f32) -> Rgba {
    let f = |v: u8| (v as f32 * k).round().clamp(0.0, 255.0) as u8;
    [f(c[0]), f(c[1]), f(c[2]), c[3]]
}

fn with_alpha(mut c: Rgba, k: f32) -> Rgba {
    c[3] = (c[3] as f32 * k).round().clamp(0.0, 255.0) as u8;
    c
}

/// How strongly layout placeholders show while editing.
pub const PLACEHOLDER_OPACITY: f32 = 0.45;

impl Ctx<'_> {
    /// The theme's background picture (fixed to the screen, scaled to
    /// cover it) and its 3D layers, which follow the camera only partly.
    fn backdrop(&self, cam: &Camera, vp: [f64; 2], items: &mut Vec<Item>) {
        let th = self.theme;
        let (wpx, hpx) = (vp[0] * self.ppp, vp[1] * self.ppp);
        if let Some(key) = &th.background_image {
            if let Some(levels) = self.r.images.levels(self.editor, key) {
                let l0 = &levels[0];
                let k = (wpx / l0.width as f64).max(hpx / l0.height as f64);
                let (iw, ih) = (l0.width as f64 * k, l0.height as f64 * k);
                let level = pick_level(&levels, iw, self.full_res_images);
                let t = Affine::translate(((wpx - iw) / 2.0, (hpx - ih) / 2.0))
                    * Affine::scale_non_uniform(iw / level.width as f64, ih / level.height as f64);
                items.push(Item::Image { image: level, transform: t });
            }
        }
        if th.layers.is_empty() {
            return;
        }
        let Some(b) = self.editor.doc.bounds() else { return };
        let home = Camera::fit_rect(&b, Vec2::new(vp[0], vp[1]), 0.06);
        if !(home.zoom.is_finite() && home.zoom > 0.0) {
            return;
        }
        for layer in &th.layers {
            let Some(levels) = self.r.images.levels(self.editor, &layer.asset) else { continue };
            let d = layer.depth.clamp(0.0, 1.0);
            // The layer's own camera: part of the way from the overview
            // camera to the real one (zoom interpolated geometrically).
            let zoom = home.zoom * (cam.zoom / home.zoom).powf(d);
            let (cx, cy) = (home.cx + (cam.cx - home.cx) * d, home.cy + (cam.cy - home.cy) * d);
            let rot = cam.rotation * d;
            // At the overview the layer covers the screen with a margin.
            let l0 = &levels[0];
            let (vw, vh) = (vp[0] / home.zoom * 1.3, vp[1] / home.zoom * 1.3);
            let k = (vw / l0.width as f64).max(vh / l0.height as f64);
            let (lw, lh) = (l0.width as f64 * k, l0.height as f64 * k);
            let level = pick_level(&levels, lw * zoom * self.ppp, self.full_res_images);
            let t = Affine::translate((wpx * 0.5, hpx * 0.5))
                * Affine::scale(zoom * self.ppp)
                * Affine::rotate(-rot.to_radians())
                * Affine::translate((home.cx - cx - lw / 2.0, home.cy - cy - lh / 2.0))
                * Affine::scale_non_uniform(lw / level.width as f64, lh / level.height as f64);
            let a = layer.opacity.clamp(0.0, 1.0) as f32;
            if a <= 0.0 {
                continue;
            }
            if a < 1.0 {
                items.push(Item::PushOpacity(a));
            }
            items.push(Item::Image { image: level, transform: t });
            if a < 1.0 {
                items.push(Item::PopOpacity);
            }
        }
    }

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
                        // Inscribed in the box (shorter side, centred).
                        let r = lr.width().min(lr.height()) / 2.0;
                        fill(items, &kurbo::Circle::new(lr.center(), r), a, color);
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
                let closed = !s.shape.is_line();
                let (fill_c, stroke_c) = match (&s.fill, &s.stroke) {
                    (None, None) if closed => (Some(th.color(&"theme:shape1".to_string())), None),
                    (None, None) => (None, Some(th.color(&"theme:shape1".to_string()))),
                    (f, st) => (f.as_ref().map(|c| th.color(c)), st.as_ref().map(|c| th.color(c))),
                };
                let sw = s.stroke_width.max(0.0);
                let dashed = |st: Stroke| match s.dash {
                    Dash::Solid => st,
                    Dash::Dashed => st.with_dashes(0.0, [sw * 3.0, sw * 2.0]),
                    Dash::Dotted => st.with_caps(Cap::Round).with_dashes(0.0, [0.0, sw * 2.0]),
                };
                if closed {
                    let g = crate::shapes::geometry(s.shape, w, h, s.corner_radius);
                    if let Some(c) = fill_c {
                        fill(items, &g.outline, a, c);
                    }
                    if let Some(c) = stroke_c {
                        stroke(items, &g.outline, a, c, dashed(Stroke::new(sw).with_join(Join::Miter)));
                    }
                    if let Some(d) = &g.detail {
                        // Rims and folds: the stroke colour, else a shade of the fill.
                        let c = stroke_c.or(fill_c.map(|c| shade(c, 0.78)));
                        if let Some(c) = c {
                            let dw = if stroke_c.is_some() { sw } else { (w.min(h) * 0.012).max(1e-9) };
                            stroke(items, d, a, c, Stroke::new(dw).with_join(Join::Round).with_caps(Cap::Round));
                        }
                    }
                } else {
                    // Lines use the stroke colour, else the fill colour.
                    let Some(c) = stroke_c.or(fill_c) else { return };
                    let hw = w / 2.0;
                    let st = dashed(Stroke::new(sw).with_caps(Cap::Round));
                    let head = (sw * 4.0).max(w * 0.06).min(w * 0.45);
                    let half = head * 0.6;
                    let (start_head, end_head) = match s.shape {
                        ShapeKind::Arrow => (false, true),
                        ShapeKind::DoubleArrow => (true, true),
                        _ => (false, false),
                    };
                    let x0 = if start_head { -hw + head * 0.9 } else { -hw };
                    let x1 = if end_head { hw - head * 0.9 } else { hw };
                    stroke(items, &kurbo::Line::new((x0, 0.0), (x1, 0.0)), a, c, st);
                    for (on, dir) in [(end_head, 1.0), (start_head, -1.0)] {
                        if on {
                            let mut p = BezPath::new();
                            p.move_to((hw * dir, 0.0));
                            p.line_to(((hw - head) * dir, -half));
                            p.line_to(((hw - head) * dir, half));
                            p.close_path();
                            fill(items, &p, a, c);
                        }
                    }
                }
            }
            ElementKind::Connector(c) => {
                let color = th.color(&c.color);
                let sw = c.width;
                let r = zebra_core::connector::local_route(c);
                let head = (sw * 4.0).max(10.0);
                // Pull the line back from filled arrowheads so its square end
                // doesn't poke through the tip.
                let mut path = BezPath::new();
                let pt = |v: Vec2| Point::new(v.x, v.y);
                let back = |h: zebra_core::Arrowhead| if h == zebra_core::Arrowhead::Arrow { head * 0.8 } else { 0.0 };
                let (t0, t1) = (r.tangent(false), r.tangent(true));
                match &r {
                    zebra_core::connector::RoutePath::Poly(p) => {
                        let mut p = p.clone();
                        let n = p.len();
                        if n >= 2 {
                            p[0] = p[0] - t0 * back(c.start);
                            p[n - 1] = p[n - 1] - t1 * back(c.end);
                        }
                        for (i, q) in p.iter().enumerate() {
                            if i == 0 {
                                path.move_to(pt(*q));
                            } else {
                                path.line_to(pt(*q));
                            }
                        }
                    }
                    zebra_core::connector::RoutePath::Cubic([a, b, cc, d]) => {
                        path.move_to(pt(*a - t0 * back(c.start)));
                        path.curve_to(pt(*b), pt(*cc), pt(*d - t1 * back(c.end)));
                    }
                }
                let st = Stroke::new(sw).with_caps(Cap::Round).with_join(Join::Round);
                let st = match c.dash {
                    Dash::Solid => st,
                    Dash::Dashed => st.with_caps(Cap::Butt).with_dashes(0.0, [sw * 3.0, sw * 2.0]),
                    Dash::Dotted => st.with_dashes(0.0, [0.0, sw * 2.0]),
                };
                stroke(items, &path, a, color, st);
                let samples = r.sample();
                let ends = [(c.start, samples.first().copied(), t0), (c.end, samples.last().copied(), t1)];
                for (kind, at, dir) in ends {
                    let Some(tip) = at else { continue };
                    let side = Vec2::new(-dir.y, dir.x);
                    let base = tip - dir * head;
                    match kind {
                        zebra_core::Arrowhead::None => {}
                        zebra_core::Arrowhead::Arrow => {
                            let p = BezPath::from_vec(vec![
                                kurbo::PathEl::MoveTo(pt(tip)),
                                kurbo::PathEl::LineTo(pt(base + side * (head * 0.55))),
                                kurbo::PathEl::LineTo(pt(base - side * (head * 0.55))),
                                kurbo::PathEl::ClosePath,
                            ]);
                            fill(items, &p, a, color);
                        }
                        zebra_core::Arrowhead::Open => {
                            let mut p = BezPath::new();
                            p.move_to(pt(base + side * (head * 0.55)));
                            p.line_to(pt(tip));
                            p.line_to(pt(base - side * (head * 0.55)));
                            stroke(items, &p, a, color, Stroke::new(sw).with_caps(Cap::Round).with_join(Join::Round));
                        }
                        zebra_core::Arrowhead::Dot => {
                            fill(items, &kurbo::Circle::new(pt(tip), (sw * 1.8).max(4.0)), a, color);
                        }
                    }
                }
                if !c.label.is_empty() {
                    let t = zebra_core::TextData { text: c.label.clone(), style: zebra_core::TextStyle::Body, ..Default::default() };
                    let layout = self.r.fonts.layout(&t, th, None);
                    let m = r.midpoint();
                    let (lw, lh) = (layout.width, layout.height);
                    let pad = lh * 0.25;
                    let bg = RoundedRect::new(m.x - lw / 2.0 - pad, m.y - lh / 2.0 - pad * 0.5, m.x + lw / 2.0 + pad, m.y + lh / 2.0 + pad * 0.5, lh * 0.3);
                    fill(items, &bg, a, th.color(&th.background));
                    let tc = th.color(&th.text_style(zebra_core::TextStyle::Body).color);
                    let transform = *a * Affine::translate((m.x - lw / 2.0, m.y - lh / 2.0));
                    items.push(Item::Text { layout, transform, color: tc, run_colors: Arc::from([]) });
                }
            }
            ElementKind::Icon(icon) => {
                let Some(path) = crate::shapes::icon_path(&icon.name) else { return };
                let ia = *a * crate::shapes::icon_affine(w, h);
                let st = Stroke::new(icon.stroke_width.max(0.0)).with_caps(Cap::Round).with_join(Join::Round);
                stroke(items, &*path, &ia, th.color(&icon.color), st);
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
                let level = pick_level(&levels, on_screen, self.full_res_images);
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
