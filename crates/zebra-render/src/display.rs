//! Renderer-neutral display list.
//!
//! Built per frame from the f64 world model. Every element's local→device
//! transform is composed from `element centre − camera centre` (computed in
//! f64 first), so items are already rebased to the camera and f32-safe at any
//! zoom. Paths are in device pixels; text and images carry a similarity
//! transform from their own local space to device pixels.

use std::sync::Arc;

use kurbo::{Affine, BezPath, Cap, Ellipse, Join, Point, Rect, RoundedRect, Shape, Stroke};
use zebra_core::{
    Camera, Crop, Dash, Editor, Element, ElementKind, FrameStyle, ImageMask, PageRef, ShapeKind, Theme, Vec2,
};

use crate::images::{level_for, Decoded};

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
    /// Seconds until a visible animation (GIF) shows its next frame. The
    /// host repaints then; None when nothing on screen animates.
    pub next_frame_in: Option<f64>,
    /// Some pictures are still decoding (a lower-resolution stand-in or a
    /// placeholder was drawn); the image cache's waker fires when they land.
    pub images_pending: bool,
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
    /// Clip the items up to the matching [`Item::PopClip`] to a device-space
    /// path (image crops and masks).
    PushClip(BezPath),
    PopClip,
}

pub use crate::images::ImageLevel;

impl DisplayList {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
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

/// Extra world-space margin an element draws outside its box. Lives in
/// core: the spatial index culls with it too.
fn overhang(e: &Element) -> f64 {
    zebra_core::spatial::draw_overhang(e)
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
    let mut list = DisplayList { width_px, height_px, background: theme.color(&theme.background), ..Default::default() };
    if width_px == 0 || height_px == 0 || !(cam.zoom.is_finite() && cam.zoom > 0.0) {
        return list;
    }
    r.fonts.next_generation();
    r.fonts.register_doc_fonts(&editor.doc);
    r.images.next_generation();
    let anim = std::cell::Cell::new(None::<f64>);
    let pending = std::cell::Cell::new(false);
    let view = cam.visible_bounds(Vec2::new(vp[0], vp[1]));
    let px_per_world = cam.zoom * ppp;
    let ctx = Ctx { r, editor, theme: &theme, presenting: o.presenting, full_res_images, ppp, o, anim: &anim, pending: &pending, screen: (width_px, height_px) };
    ctx.backdrop(cam, vp, &mut list.items);

    // Only what may reach into view (from the spatial index), back to front.
    for e in editor.elements_in(view) {
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
            opacity *= if on_fill(editor, e) { PLACEHOLDER_ON_FILL_OPACITY } else { PLACEHOLDER_OPACITY };
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
    list.next_frame_in = anim.get();
    list.images_pending = pending.get();
    list
}

struct Ctx<'a> {
    r: &'a Renderer,
    editor: &'a Editor,
    theme: &'a Theme,
    presenting: bool,
    full_res_images: bool,
    ppp: f64,
    o: &'a SceneOptions,
    anim: &'a std::cell::Cell<Option<f64>>,
    pending: &'a std::cell::Cell<bool>,
    /// Device pixels drawn.
    screen: (u32, u32),
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
/// Placeholder text on a filled shape (diagram boxes): its colour was
/// picked to read on the fill, and fading it as much loses it.
pub const PLACEHOLDER_ON_FILL_OPACITY: f32 = 0.8;

/// Text centred on a filled shape of its group (the text inside a box).
fn on_fill(editor: &Editor, t: &Element) -> bool {
    let (ElementKind::Text(_), Some(g)) = (&t.kind, &t.group) else { return false };
    let c = t.transform.center();
    editor.doc.elements.values().any(|s| match &s.kind {
        ElementKind::Shape(sd) => {
            s.group.as_ref() == Some(g)
                && !sd.shape.is_line()
                && sd.fill.as_deref().is_some_and(|f| !f.is_empty())
                && (s.transform.center() - c).len() <= (s.size.w.min(s.size.h) * s.transform.scale * 0.01).max(0.5)
        }
        _ => false,
    })
}

impl Ctx<'_> {
    /// The theme's background picture (fixed to the screen, scaled to
    /// cover it) and its 3D layers, which follow the camera only partly.
    fn backdrop(&self, cam: &Camera, vp: [f64; 2], items: &mut Vec<Item>) {
        let th = self.theme;
        let (wpx, hpx) = (vp[0] * self.ppp, vp[1] * self.ppp);
        if let Some(key) = &th.background_image {
            if let Some((sw, sh)) = self.asset_size(key) {
                let k = (wpx / sw as f64).max(hpx / sh as f64);
                let (iw, ih) = (sw as f64 * k, sh as f64 * k);
                if let Some(level) = self.still(key, iw) {
                    let t = Affine::translate(((wpx - iw) / 2.0, (hpx - ih) / 2.0))
                        * Affine::scale_non_uniform(iw / level.width as f64, ih / level.height as f64);
                    items.push(Item::Image { image: level, transform: t });
                }
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
            let Some((sw, sh)) = self.asset_size(&layer.asset) else { continue };
            let d = layer.depth.clamp(0.0, 1.0);
            // The layer's own camera: part of the way from the overview
            // camera to the real one (zoom interpolated geometrically).
            let zoom = home.zoom * (cam.zoom / home.zoom).powf(d);
            let (cx, cy) = (home.cx + (cam.cx - home.cx) * d, home.cy + (cam.cy - home.cy) * d);
            let rot = cam.rotation * d;
            // At the overview the layer covers the screen with a margin.
            let (vw, vh) = (vp[0] / home.zoom * 1.3, vp[1] / home.zoom * 1.3);
            let k = (vw / sw as f64).max(vh / sh as f64);
            let (lw, lh) = (sw as f64 * k, sh as f64 * k);
            let Some(level) = self.still(&layer.asset, lw * zoom * self.ppp) else { continue };
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

    /// Pixel size of an image asset.
    fn asset_size(&self, key: &str) -> Option<(u32, u32)> {
        self.editor.doc.assets.get(key).filter(|a| a.width > 0 && a.height > 0).map(|a| (a.width, a.height))
    }

    /// A decoded still of `key` at least `on_screen_px` wide (through the
    /// bounded image cache; first frame of an animation).
    fn still(&self, key: &str, on_screen_px: f64) -> Option<Arc<ImageLevel>> {
        let src = self.editor.doc.assets.get(key)?;
        let level = if self.full_res_images { 0 } else { level_for(src.width, src.height, on_screen_px) };
        let sync = !self.o.async_images || self.full_res_images;
        match self.r.images.get(key, &src.ext, &src.bytes, (src.width, src.height), level, sync) {
            Some(Decoded::Still(l)) => Some(l),
            Some(Decoded::Animated(a)) => Some(a.frames[0].clone()),
            None => {
                self.pending.set(true);
                None
            }
        }
    }

    /// Draw a picture asset filling `e`'s box (through `crop`, clipped to
    /// `mask`). False when it can't be drawn yet.
    #[allow(clippy::too_many_arguments)]
    fn picture(&self, asset: &str, crop: Option<Crop>, mask: ImageMask, e: &Element, a: &Affine, items: &mut Vec<Item>, svg: Option<crate::svg::SvgStyle>, page: Option<&PageRef>) -> bool {
        let Some(src) = self.editor.doc.assets.get(asset) else { return false };
        let (w, h) = (e.size.w, e.size.h);
        let c = crop.unwrap_or(Crop::FULL);
        // The whole picture's box in local units.
        let (fw, fh) = (w / c.w, h / c.h);
        let on_screen = fw * affine_scale(a);
        let level = if self.full_res_images { 0 } else { level_for(src.width, src.height, on_screen) };
        let sync = !self.o.async_images || self.full_res_images;
        let got = if src.ext == "svg" {
            let style = svg.unwrap_or_default();
            let key = if style.is_plain() { asset.to_string() } else { format!("{asset}#{}", style.key()) };
            self.r.images.get_svg(&key, &src.bytes, (src.width, src.height), level, sync, style)
        } else {
            self.r.images.get(asset, &src.ext, &src.bytes, (src.width, src.height), level, sync)
        };
        let Some(decoded) = got else {
            self.pending.set(true);
            return false;
        };
        if decoded.width() < crate::images::level_size(src.width, src.height, level).0 {
            self.pending.set(true);
        }
        let image = match decoded {
            Decoded::Still(l) => l,
            Decoded::Animated(anim) => match self.o.time {
                Some(t) => {
                    let (f, next) = anim.frame_at(t);
                    self.anim.set(Some(self.anim.get().map_or(next, |n: f64| n.min(next))));
                    f.clone()
                }
                None => anim.frames[0].clone(),
            },
        };
        let lr = local_rect(e);
        // The whole picture's top-left corner, in local units.
        let whole = *a * Affine::translate((-w / 2.0 - c.x * fw, -h / 2.0 - c.y * fh));
        let transform = whole * Affine::scale_non_uniform(fw / image.width as f64, fh / image.height as f64);
        let tiles = match page {
            Some(p) if !self.full_res_images => self.page_tiles(p, &whole, fw, fh, c, src.width),
            _ => vec![],
        };
        if self.o.ghost.as_deref() == Some(e.id.as_str()) {
            items.push(Item::PushOpacity(0.35));
            items.push(Item::Image { image: image.clone(), transform });
            items.push(Item::PopOpacity);
        }
        let clip = crop.is_some() || mask != ImageMask::Rectangle;
        if clip {
            let mut p = match mask {
                ImageMask::Rectangle => lr.to_path(0.01),
                ImageMask::Rounded => RoundedRect::from_rect(lr, w.min(h) * 0.12).to_path(0.01),
                ImageMask::Circle => Ellipse::from_rect(lr).to_path(0.01),
            };
            p.apply_affine(*a);
            items.push(Item::PushClip(p));
        }
        items.push(Item::Image { image, transform });
        items.extend(tiles);
        if clip {
            items.push(Item::PopClip);
        }
        true
    }

    /// A PDF page seen larger than its picture: the visible part drawn
    /// again from the PDF at this zoom, in tiles (see [`crate::pages`]).
    /// `whole` maps the whole page box (`fw` x `fh` local units, top-left
    /// at 0,0) to device pixels; `pic_w` is the page picture's width.
    /// Tiles still being made leave a gap: the half-zoom tile under it is
    /// drawn if cached, else the picture already drawn shows through.
    fn page_tiles(&self, page: &PageRef, whole: &Affine, fw: f64, fh: f64, c: Crop, pic_w: u32) -> Vec<Item> {
        use crate::images::TILE_PREFIX;
        use crate::pages::{PageRequest, MAX_PAGE_PX, TILE};
        let mut out = vec![];
        let Some(raster) = self.r.page_raster() else { return out };
        let Some(pdf) = self.editor.doc.assets.get(&page.pdf) else { return out };
        if !(fw > 0.0 && fh > 0.0) {
            return out;
        }
        let on_screen = fw * affine_scale(whole);
        if !on_screen.is_finite() || on_screen <= pic_w as f64 * 1.05 {
            return out;
        }
        let mut pw = TILE;
        while (pw as f64) < on_screen && pw < MAX_PAGE_PX {
            pw *= 2;
        }
        let height_for = |pw: u32| ((pw as f64) * fh / fw).round().clamp(1.0, (MAX_PAGE_PX * 4) as f64) as u32;
        let ph = height_for(pw);
        let to_device = |pw: u32, ph: u32| *whole * Affine::scale_non_uniform(fw / pw as f64, fh / ph as f64);
        let t = to_device(pw, ph);
        // The screen, in page pixels.
        let inv = t.inverse();
        let (sw, sh) = (self.screen.0 as f64, self.screen.1 as f64);
        let pts = [Point::new(0.0, 0.0), Point::new(sw, 0.0), Point::new(0.0, sh), Point::new(sw, sh)].map(|p| inv * p);
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in pts {
            (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
        }
        // Only the shown (cropped) part of the page.
        let (cx0, cy0) = (c.x * pw as f64, c.y * ph as f64);
        let (cx1, cy1) = (cx0 + c.w * pw as f64, cy0 + c.h * ph as f64);
        let (x0, y0) = (x0.max(cx0).max(0.0), y0.max(cy0).max(0.0));
        let (x1, y1) = (x1.min(cx1).min(pw as f64), y1.min(cy1).min(ph as f64));
        if !(x1 > x0 && y1 > y0) {
            return out;
        }
        let tile = TILE as f64;
        let (tx0, ty0) = ((x0 / tile).floor() as u32, (y0 / tile).floor() as u32);
        let (tx1, ty1) = ((x1 / tile).ceil() as u32, (y1 / tile).ceil() as u32);
        // A turned page on a big screen: more tiles than are worth making.
        if (tx1 - tx0) as u64 * (ty1 - ty0) as u64 > 96 {
            return out;
        }
        let key = |pw: u32, tx: u32, ty: u32| format!("{TILE_PREFIX}{}#{}@{pw}:{tx},{ty}", page.pdf, page.index);
        let sync = !self.o.async_images;
        let mut have = vec![];
        let mut under = std::collections::BTreeSet::new();
        for ty in ty0..ty1 {
            for tx in tx0..tx1 {
                let (x, y) = (tx * TILE, ty * TILE);
                let rect = (x, y, TILE.min(pw - x), TILE.min(ph - y));
                let k = key(pw, tx, ty);
                let req = PageRequest { pdf: page.pdf.clone(), bytes: pdf.bytes.clone(), index: page.index, page_px: (pw, ph), rect };
                let raster = raster.clone();
                let name = k.clone();
                let make: crate::images::Make = Arc::new(move || {
                    let px = raster(&req)?;
                    let (w, h) = (req.rect.2, req.rect.3);
                    (px.len() == (w * h * 4) as usize).then(|| ImageLevel::from_rgba(&name, w, h, px))
                });
                match self.r.images.get_made(&k, sync, make) {
                    Some(Decoded::Still(l)) => have.push(Item::Image { image: l, transform: t * Affine::translate((x as f64, y as f64)) }),
                    _ => {
                        self.pending.set(true);
                        if pw / 2 > pic_w {
                            under.insert((tx / 2, ty / 2));
                        }
                    }
                }
            }
        }
        let (hw, hh) = (pw / 2, height_for(pw / 2));
        let th = to_device(hw, hh);
        for (tx, ty) in under {
            if let Some(Decoded::Still(l)) = self.r.images.peek(&key(hw, tx, ty)) {
                out.push(Item::Image { image: l, transform: th * Affine::translate(((tx * TILE) as f64, (ty * TILE) as f64)) });
            }
        }
        out.extend(have);
        out
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
                let svg = img.symbol.as_ref().map(|l| crate::svg::SvgStyle {
                    color: l.color.as_ref().map(|c| th.color(c)),
                    color2: l.color2.as_ref().map(|c| th.color(c)),
                    sticker: l.sticker,
                });
                if !self.picture(&img.asset, img.crop, img.mask, e, a, items, svg, img.page.as_ref()) {
                    placeholder(items, &lr, a, w.min(h), !self.r.images.is_failed(&img.asset));
                }
            }
            ElementKind::Video(v) => {
                let playing = self.o.playing.contains(&e.id);
                if let Some(frame) = self.o.video_frames.get(&e.id) {
                    let transform = *a
                        * Affine::translate((-w / 2.0, -h / 2.0))
                        * Affine::scale_non_uniform(w / frame.width as f64, h / frame.height as f64);
                    items.push(Item::Image { image: frame.clone(), transform });
                } else if !v.poster.as_ref().is_some_and(|p| self.picture(p, None, ImageMask::Rectangle, e, a, items, None, None)) {
                    fill(items, &RoundedRect::from_rect(lr, w.min(h) * 0.02), a, [29, 29, 31, 255]);
                }
                if !playing {
                    play_badge(items, &lr, a);
                }
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

/// Stand-in for a picture: a soft box while it decodes, plus a cross if it
/// can't be decoded at all.
fn placeholder(items: &mut Vec<Item>, lr: &Rect, a: &Affine, min_side: f64, loading: bool) {
    fill(items, lr, a, [220, 220, 220, 255]);
    if !loading {
        let mut p = BezPath::new();
        p.move_to((lr.x0, lr.y0));
        p.line_to((lr.x1, lr.y1));
        p.move_to((lr.x1, lr.y0));
        p.line_to((lr.x0, lr.y1));
        stroke(items, &p, a, [150, 150, 150, 255], Stroke::new(min_side * 0.01));
    }
}

/// The round ▶ badge drawn on a video that isn't playing.
fn play_badge(items: &mut Vec<Item>, lr: &Rect, a: &Affine) {
    let d = (lr.width().min(lr.height()) * 0.22).max(1e-9);
    let c = lr.center();
    fill(items, &kurbo::Circle::new(c, d / 2.0), a, [0, 0, 0, 140]);
    stroke(items, &kurbo::Circle::new(c, d / 2.0), a, [255, 255, 255, 230], Stroke::new(d * 0.04));
    let t = d * 0.2;
    let mut p = BezPath::new();
    p.move_to((c.x - t * 0.8, c.y - t));
    p.line_to((c.x + t * 1.2, c.y));
    p.line_to((c.x - t * 0.8, c.y + t));
    p.close_path();
    fill(items, &p, a, [255, 255, 255, 240]);
}
