//! GPU canvas: vello renders the display list into a wgpu texture on eframe's
//! device (Metal on macOS) and egui shows it as an image.
use egui_wgpu::wgpu;
use kurbo::{Affine, Rect, Shape};
use vello::peniko::{BlendMode, Color, Fill, Mix};
use vello::{AaConfig, AaSupport, RenderParams, RendererOptions, Scene};

use crate::cpu::split_scale;
use crate::display::{DisplayList, Item, Rgba};

fn color(c: Rgba) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Encode a display list into a vello scene (identity transform: items are
/// already in device pixels).
pub fn encode_scene(scene: &mut Scene, list: &DisplayList) {
    let full = Rect::new(0.0, 0.0, list.width_px as f64, list.height_px as f64);
    scene.fill(Fill::NonZero, Affine::IDENTITY, color(list.background), None, &full);
    let layer_rects = layer_bounds(&list.items, full);
    let mut skip_pop = Vec::new();
    let mut alpha = 1.0f32;
    for (i, item) in list.items.iter().enumerate() {
        // A group holding one fill or stroke: fold the opacity into its
        // colour instead of opening a layer (same pixels, much cheaper).
        if let Item::PushOpacity(a) = item {
            if matches!(list.items.get(i + 2), Some(Item::PopOpacity))
                && matches!(list.items.get(i + 1), Some(Item::Fill { .. } | Item::Stroke { .. }))
            {
                alpha = *a;
                skip_pop.push(i + 2);
                continue;
            }
        }
        if skip_pop.last() == Some(&i) {
            skip_pop.pop();
            alpha = 1.0;
            continue;
        }
        let color = |c: Rgba| color(c).multiply_alpha(alpha);
        match item {
            Item::Fill { path, color: c } => scene.fill(Fill::NonZero, Affine::IDENTITY, color(*c), None, path),
            Item::Stroke { path, color: c, stroke } => scene.stroke(stroke, Affine::IDENTITY, color(*c), None, path),
            Item::Text { layout, transform, color: c, run_colors } => {
                let (s, rest) = split_scale(transform);
                for (ri, run) in layout.runs.iter().enumerate() {
                    let brush = color(run_colors.get(ri).copied().unwrap_or(*c));
                    // vello GPU glyph space is y-up, vello_cpu y-down: the signs differ.
                    let skew = run.skew.map(|a| Affine::skew((a as f64).to_radians().tan(), 0.0));
                    scene
                        .draw_glyphs(&run.font)
                        .transform(rest)
                        .font_size(run.font_size * s as f32)
                        .hint(false)
                        .normalized_coords(&run.coords)
                        .glyph_transform(skew)
                        .brush(brush)
                        .draw(
                            Fill::NonZero,
                            run.glyphs.iter().map(|g| vello::Glyph { id: g.id, x: g.x * s as f32, y: g.y * s as f32 }),
                        );
                }
            }
            Item::Image { image, transform } => scene.draw_image(&image.data, *transform),
            // A layer costs work in every tile its clip covers, so bound it
            // to what it holds: dozens of full-screen layers (faded tiny
            // text at fit-all) overflowed vello's fixed buffers and the
            // whole frame came out black.
            Item::PushOpacity(a) => {
                scene.push_layer(Fill::NonZero, BlendMode::from(Mix::Normal), *a, Affine::IDENTITY, &layer_rects[i])
            }
            Item::PopOpacity | Item::PopClip => scene.pop_layer(),
            Item::PushClip(path) => scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, path),
        }
    }
}

/// For each `PushOpacity`, the device rect its group can touch (clamped to
/// the frame); other entries are unused.
fn layer_bounds(items: &[Item], full: Rect) -> Vec<Rect> {
    let mut out = vec![full; items.len()];
    // Open groups: (index, bounds so far, any unbounded item).
    let mut open: Vec<(usize, Option<Rect>, bool)> = Vec::new();
    let mut clips: Vec<Option<Rect>> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let r = match item {
            Item::Fill { path, .. } => Some(path.bounding_box()),
            Item::Stroke { path, stroke, .. } => Some(path.bounding_box().inflate(stroke.width * 2.0 + 2.0, stroke.width * 2.0 + 2.0)),
            Item::Text { layout, transform, .. } => {
                // Glyphs can overhang the layout box (italics, accents): pad.
                let pad = layout.runs.iter().map(|r| r.font_size as f64).fold(0.0, f64::max);
                let b = Rect::new(-pad, -pad, layout.width + pad, layout.height + pad);
                Some(transform.transform_rect_bbox(b))
            }
            Item::Image { image, transform } => {
                Some(transform.transform_rect_bbox(Rect::new(0.0, 0.0, image.width as f64, image.height as f64)))
            }
            Item::PushOpacity(_) => {
                open.push((i, None, false));
                continue;
            }
            Item::PushClip(path) => {
                clips.push(Some(path.bounding_box()));
                continue;
            }
            Item::PopClip => {
                clips.pop();
                continue;
            }
            Item::PopOpacity => {
                let Some((at, b, unbounded)) = open.pop() else { continue };
                let b = if unbounded { full } else { b.map_or(Rect::ZERO, |b| b.inflate(2.0, 2.0).intersect(full)) };
                out[at] = b;
                // The finished group counts as one item of its parent.
                if let Some(parent) = open.last_mut() {
                    parent.1 = Some(parent.1.map_or(b, |p| p.union(b)));
                    parent.2 |= unbounded;
                }
                continue;
            }
        };
        // A clip bounds what's inside it.
        let r = match (r, clips.iter().flatten().next_back()) {
            (Some(r), Some(c)) => Some(r.intersect(*c)),
            (r, _) => r,
        };
        if let Some(g) = open.last_mut() {
            match r {
                Some(r) if r.is_finite() => g.1 = Some(g.1.map_or(r, |p| p.union(r))),
                _ => g.2 = true,
            }
        }
    }
    out
}

/// vello renderer bound to one wgpu device. Usable headless (tests,
/// GPU screenshots) as well as under [`VelloCanvas`].
pub struct GpuRenderer {
    renderer: vello::Renderer,
    scene: Scene,
    /// Milliseconds of the last render: (scene encode, vello submit, error
    /// scope wait). For the `perf` query.
    pub last_ms: [f32; 3],
    /// Pictures drawn in the last frame.
    last_images: std::collections::HashSet<Shown>,
    /// Frames still to draw so pictures that didn't fit show (see
    /// [`GpuRenderer::needs_another_frame`]).
    settle: u8,
}

/// Frames heavier than this (see [`crate::frame_cost`]) don't leave their
/// encoding buffers behind.
const SHRINK_AFTER_COST: usize = 200_000;

/// Pictures in a frame above this many pixels might not fit next to the
/// last frames' in vello's atlas (8192² = 67M).
const SETTLE_ABOVE_PX: u64 = 8_000_000;

/// A picture as [`GpuRenderer::needs_another_frame`] tells them apart: a
/// still by its pixels (blob id), a moving one (GIF, video) by what it is
/// and its size, so its next frame is not a new picture.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Shown {
    Still(u64),
    Moving(String, u32, u32),
}

/// The pictures in `list`, and their pixels.
fn shown_images(list: &DisplayList) -> (std::collections::HashSet<Shown>, u64) {
    let mut shown = std::collections::HashSet::new();
    let mut px = 0u64;
    for it in &list.items {
        if let Item::Image { image, .. } = it {
            let key = if image.moving { Shown::Moving(image.asset.clone(), image.width, image.height) } else { Shown::Still(image.data.data.id()) };
            if shown.insert(key) {
                px += image.width as u64 * image.height as u64;
            }
        }
    }
    (shown, px)
}

/// Frames still to draw after one showing `now`: two more when a new
/// picture came in and the set is big enough to fill the atlas (small sets
/// can't; don't spend frames on them). A big GIF's next frame is not new,
/// so it no longer keeps the window redrawing (20 Nov audit, W9).
fn settle_after(settle: u8, last: &std::collections::HashSet<Shown>, now: &std::collections::HashSet<Shown>, px: u64) -> u8 {
    let new = now.iter().any(|s| !last.contains(s));
    if new && px > SETTLE_ABOVE_PX { 2 } else { settle.saturating_sub(1) }
}

fn renderer_options() -> RendererOptions {
    RendererOptions { antialiasing_support: AaSupport::area_only(), ..Default::default() }
}

impl GpuRenderer {
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
        let renderer = vello::Renderer::new(device, renderer_options())
        .map_err(|e| format!("vello init failed: {e}"))?;
        Ok(GpuRenderer { renderer, scene: Scene::new(), last_ms: [0.0; 3], last_images: Default::default(), settle: 0 })
    }

    /// Render `list` into `view` (an Rgba8Unorm texture with STORAGE_BINDING,
    /// at least `list.width_px` x `list.height_px`).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        list: &DisplayList,
        view: &wgpu::TextureView,
    ) -> Result<(), String> {
        self.note_images(list);
        let t0 = std::time::Instant::now();
        self.scene.reset();
        encode_scene(&mut self.scene, list);
        let t1 = std::time::Instant::now();
        let params = RenderParams {
            base_color: color(list.background),
            width: list.width_px,
            height: list.height_px,
            antialiasing_method: AaConfig::Area,
        };
        // Catch validation errors (e.g. a buffer over the device's binding
        // limit) instead of letting wgpu's default handler panic, and rebuild
        // vello afterwards: a failed render can leave its resources (image
        // atlas included) in a bad state, which showed as a black canvas and
        // missing images until restart.
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let res = self.renderer.render_to_texture(device, queue, &self.scene, view, &params);
        let t2 = std::time::Instant::now();
        let gpu_err = pollster::block_on(scope.pop());
        // A heavy frame grows the scene's encoding buffers, and reset keeps
        // their capacity: after one, start a fresh scene so the memory goes
        // back instead of staying at the high-water mark.
        if crate::clip::frame_cost(&list.items) > SHRINK_AFTER_COST {
            self.scene = Scene::new();
        }
        let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f32() * 1000.0;
        self.last_ms = [ms(t0, t1), ms(t1, t2), ms(t2, std::time::Instant::now())];
        if let Some(e) = gpu_err {
            if let Ok(r) = vello::Renderer::new(device, renderer_options()) {
                self.renderer = r;
            }
            return Err(format!("vello render failed, renderer reset: {e}"));
        }
        res.map_err(|e| format!("vello render failed: {e}"))
    }

    /// True when the frame just drawn may have left pictures out, so the
    /// host should draw it again (at most twice per change).
    ///
    /// vello keeps every picture of the last three frames in one 8192²
    /// atlas and leaves out what doesn't fit, without telling us. Zooming
    /// a big PDF page swaps whole sets of tiles, and the new set could
    /// find the atlas still full of the old one: tiles went missing, and
    /// with nothing else to repaint the window they stayed missing. Two
    /// more frames let vello drop the old set and place the new one.
    pub fn needs_another_frame(&self) -> bool {
        self.settle > 0
    }

    fn note_images(&mut self, list: &DisplayList) {
        let (shown, px) = shown_images(list);
        self.settle = settle_after(self.settle, &self.last_images, &shown, px);
        self.last_images = shown;
    }

    /// Render off-screen and read back straight RGBA8 rows (for tests and
    /// GPU screenshots). Blocks until the GPU is done.
    pub fn render_to_rgba(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, list: &DisplayList) -> Result<Vec<u8>, String> {
        let (w, h) = (list.width_px.max(1), list.height_px.max(1));
        let target = Target::new(device, w, h, wgpu::TextureUsages::COPY_SRC);
        self.render(device, queue, list, &target.view)?;
        let row = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zebra readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("zebra readback") });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .map_err(|e| format!("GPU poll failed: {e}"))?;
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("readback failed: {e}"))?;
        let data = slice.get_mapped_range().map_err(|e| format!("readback map failed: {e:?}"))?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let start = y * row as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        Ok(out)
    }
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    w: u32,
    h: u32,
}

impl Target {
    fn new(device: &wgpu::Device, w: u32, h: u32, extra: wgpu::TextureUsages) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("zebra canvas"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | extra,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Target { texture, view, w, h }
    }
}

/// The app's canvas widget. One per window.
pub struct VelloCanvas {
    gpu: Result<GpuRenderer, String>,
    target: Option<Target>,
    texture_id: Option<egui::TextureId>,
    reported: bool,
}

impl VelloCanvas {
    /// See [`GpuRenderer::last_ms`].
    pub fn last_ms(&self) -> [f32; 3] {
        self.gpu.as_ref().map(|g| g.last_ms).unwrap_or_default()
    }

    /// Create once, from `frame.wgpu_render_state()`.
    pub fn new(rs: &egui_wgpu::RenderState) -> Self {
        VelloCanvas { gpu: GpuRenderer::new(&rs.device), target: None, texture_id: None, reported: false }
    }

    /// The vello init error, if the GPU path is unavailable.
    pub fn error(&self) -> Option<&str> {
        self.gpu.as_ref().err().map(|s| s.as_str())
    }

    /// Paint `list` (built at `rect.size() * pixels_per_point`) into `rect`.
    /// Call every frame the canvas is visible.
    pub fn paint(&mut self, ui: &mut egui::Ui, rect: egui::Rect, list: &DisplayList, rs: &egui_wgpu::RenderState) {
        let gpu = match &mut self.gpu {
            Ok(g) => g,
            Err(e) => {
                if !self.reported {
                    eprintln!("zebra-render: {e}");
                    self.reported = true;
                }
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_gray(128));
                return;
            }
        };
        let max = rs.device.limits().max_texture_dimension_2d;
        let (w, h) = (list.width_px.clamp(1, max), list.height_px.clamp(1, max));
        if list.width_px == 0 || list.height_px == 0 {
            return;
        }
        let resized = !matches!(&self.target, Some(t) if t.w == w && t.h == h);
        if resized {
            let t = Target::new(&rs.device, w, h, wgpu::TextureUsages::empty());
            let mut r = rs.renderer.write();
            match self.texture_id {
                Some(id) => r.update_egui_texture_from_wgpu_texture(&rs.device, &t.view, wgpu::FilterMode::Linear, id),
                None => self.texture_id = Some(r.register_native_texture(&rs.device, &t.view, wgpu::FilterMode::Linear)),
            }
            drop(r);
            self.target = Some(t);
        }
        let target = self.target.as_ref().expect("target");
        let mut list_ref = std::borrow::Cow::Borrowed(list);
        if (w, h) != (list.width_px, list.height_px) {
            let l = list_ref.to_mut();
            l.width_px = w;
            l.height_px = h;
        }
        if let Err(e) = gpu.render(&rs.device, &rs.queue, &list_ref, &target.view) {
            eprintln!("zebra-render: {e}");
            // Show the background rather than a stale or black frame.
            let bg = list.background;
            ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(bg[0], bg[1], bg[2], bg[3]));
            return;
        }
        if gpu.needs_another_frame() {
            ui.ctx().request_repaint();
        }
        if let Some(id) = self.texture_id {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(id, rect, uv, egui::Color32::WHITE);
        }
    }

    /// Release the egui texture (call when the canvas goes away for good).
    pub fn free(&mut self, rs: &egui_wgpu::RenderState) {
        if let Some(id) = self.texture_id.take() {
            rs.renderer.write().free_texture(&id);
        }
        self.target = None;
    }
}

#[cfg(test)]
mod settle_tests {
    use super::*;
    use crate::display::ImageLevel;
    use std::sync::Arc;

    fn list(images: Vec<Arc<ImageLevel>>) -> DisplayList {
        DisplayList {
            items: images.into_iter().map(|image| Item::Image { image, transform: Default::default() }).collect(),
            ..Default::default()
        }
    }

    fn big(asset: &str, moving: bool) -> Arc<ImageLevel> {
        // 3000² = 9M px, over the 8M a frame may hold without settling.
        let l = ImageLevel::from_premultiplied(asset, 3000, 3000, vec![0; 3000 * 3000 * 4]);
        Arc::new(if moving { l.as_moving() } else { l })
    }

    /// Run frames of `lists` and count those that asked for another.
    fn settling(lists: &[DisplayList]) -> Vec<u8> {
        let (mut last, mut settle) = (Default::default(), 0);
        lists
            .iter()
            .map(|l| {
                let (shown, px) = shown_images(l);
                settle = settle_after(settle, &last, &shown, px);
                last = shown;
                settle
            })
            .collect()
    }

    #[test]
    fn a_big_gif_settles_once_not_every_frame() {
        let frames: Vec<DisplayList> = (0..8).map(|_| list(vec![big("gif", true)])).collect();
        assert_eq!(settling(&frames), [2, 1, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_big_playing_video_settles_once_not_every_frame() {
        // Leftover of W9: video frames (4096² at most, over the 8M px) were
        // new pictures every frame.
        let frame = |shade: u8| Arc::new(ImageLevel::video_frame("clip", 3000, 3000, vec![shade; 3000 * 3000 * 4]));
        let frames: Vec<DisplayList> = (0..6).map(|i| list(vec![frame(i as u8)])).collect();
        assert_eq!(settling(&frames), [2, 1, 0, 0, 0, 0]);
    }

    #[test]
    fn new_stills_still_settle() {
        let a = big("page", false);
        let frames = [list(vec![a.clone()]), list(vec![a.clone()]), list(vec![a]), list(vec![big("page", false)])];
        assert_eq!(settling(&frames), [2, 1, 0, 2]);
    }
}
