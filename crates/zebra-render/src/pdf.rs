//! krilla PDF export: one vector page per path step, text stays text.
use std::collections::HashMap;
use std::sync::Arc;

use kurbo::{Affine, BezPath, PathEl};
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Size, Transform};
use krilla::num::NormalizedF32;
use krilla::paint::{Fill, FillRule, LineCap, LineJoin, Stroke, StrokeDash};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::{Data, Document};
use zebra_core::{Editor, ZError};

use crate::display::{DisplayList, Item, Rgba};
use crate::{Renderer, SceneOptions};

/// An imported PDF read for its pages to go into an exported PDF as
/// themselves (B7: Keynote and Preview keep a placed PDF as vectors).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct Source(krilla::pdf::PdfDocument);

/// Stack for the export's thread: reading and copying a hostile PDF's
/// objects may go deep (a stack overflow can't be caught).
#[cfg(not(target_arch = "wasm32"))]
const EXPORT_STACK: usize = 256 << 20;

/// The PDF behind `key`, if page `index` of it can go into an exported
/// PDF as itself: read once per PDF (a file the reader can't read, or
/// that makes it panic, is drawn as pictures instead). A page the page
/// engine can't draw (too slow, too much memory: the canvas shows its
/// picture, a grey placeholder) isn't copied either: the export shows
/// what the canvas shows, not a page that may hang the reader's viewer.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn embeddable(r: &Renderer, key: &str, pdf: &zebra_core::Asset, index: usize, size: (f64, f64)) -> Option<Source> {
    let src = {
        let mut known = r.pdf_sources.lock().unwrap_or_else(|e| e.into_inner());
        let got = known.entry(key.to_string()).or_insert_with(|| {
            let data = Arc::new(pdf.bytes.clone());
            let (pages, p) = deep(move || krilla::pdf::Pdf::new(data).ok().map(|p| (p.pages().len(), Arc::new(p)))).flatten()?;
            Some((pages, Source(krilla::pdf::PdfDocument::new(p))))
        });
        match got {
            Some((pages, src)) if index < *pages => src.clone(),
            _ => return None,
        }
    };
    drawable(r, key, pdf, index as u32, size).then_some(src)
}

/// Whether the page engine draws page `index` (a small test drawing, once
/// per page). With no page engine (tests, tools) every page counts.
#[cfg(not(target_arch = "wasm32"))]
fn drawable(r: &Renderer, key: &str, pdf: &zebra_core::Asset, index: u32, (w, h): (f64, f64)) -> bool {
    let Some(raster) = r.page_raster() else { return true };
    let id = (key.to_string(), index);
    if let Some(&d) = r.pdf_drawable.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        return d;
    }
    const TEST_PX: u32 = 64;
    let ph = if w > 0.0 && h > 0.0 { ((TEST_PX as f64) * h / w).round().clamp(1.0, (TEST_PX * 64) as f64) as u32 } else { TEST_PX };
    let req = crate::pages::PageRequest { pdf: key.to_string(), bytes: pdf.bytes.clone(), index, page_px: (TEST_PX, ph), rect: (0, 0, TEST_PX, ph) };
    let d = raster(&req).is_some();
    r.pdf_drawable.lock().unwrap_or_else(|e| e.into_inner()).insert(id, d);
    d
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn export(r: &Renderer, editor: &Editor) -> Result<Vec<u8>, ZError> {
    let has_pages = editor.doc.elements.values().any(|e| matches!(&e.kind, zebra_core::ElementKind::Image(i) if i.page.as_ref().is_some_and(|p| editor.doc.assets.contains_key(&p.pdf))));
    if !has_pages {
        return export_once(r, editor, true).map_err(Failed::into_error);
    }
    // Imported PDF pages go in as themselves (B7). Reading and copying a
    // PDF's objects may go deep on a hostile file, and a stack overflow
    // can't be caught: that runs on a thread with a deep stack, from a
    // copy of the document (the editor stays on its own thread).
    let mut snap = Editor::new(editor.doc.clone());
    snap.view = editor.view.clone();
    snap.measure = r.measure();
    deep(move || export_vector(r, &snap)).unwrap_or_else(|| Err(ZError::new("export_failed", "PDF export failed")))
}

/// A PDF the writer can't copy (too new a version, objects it can't
/// follow) is marked and the export made again with it drawn as pictures;
/// if anything panics, every page is drawn as pictures.
#[cfg(not(target_arch = "wasm32"))]
fn export_vector(r: &Renderer, editor: &Editor) -> Result<Vec<u8>, ZError> {
    let mut tries = 0;
    loop {
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| export_once(r, editor, false)));
        match run {
            Ok(Ok(bytes)) => return Ok(bytes),
            Ok(Err(Failed::Other(e))) => return Err(e),
            Ok(Err(Failed::Pdf(doc))) if tries < 64 => {
                tries += 1;
                let mut known = r.pdf_sources.lock().unwrap_or_else(|e| e.into_inner());
                for v in known.values_mut() {
                    if v.as_ref().is_some_and(|(_, s)| s.0 == doc) {
                        *v = None;
                    }
                }
            }
            Ok(Err(Failed::Pdf(_))) | Err(_) => {
                return match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| export_once(r, editor, true))) {
                    Ok(r) => r.map_err(Failed::into_error),
                    Err(_) => Err(ZError::new("export_failed", "PDF export failed")),
                };
            }
        }
    }
}

/// Run `f` on a thread with a deep stack (reading and copying a hostile
/// PDF's objects may go deep, and a stack overflow can't be caught), its
/// panic caught. None when it panicked or the thread couldn't start.
#[cfg(not(target_arch = "wasm32"))]
fn deep<T: Send>(f: impl FnOnce() -> T + Send) -> Option<T> {
    std::thread::scope(|sc| {
        let h = std::thread::Builder::new()
            .name("zebra-pdf-pages".into())
            .stack_size(EXPORT_STACK)
            .spawn_scoped(sc, move || std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok())
            .ok()?;
        h.join().ok().flatten()
    })
}

/// The web player has no PDF export to speak of: pages as pictures.
#[cfg(target_arch = "wasm32")]
pub(crate) fn export(r: &Renderer, editor: &Editor) -> Result<Vec<u8>, ZError> {
    export_once(r, editor, true).map_err(Failed::into_error)
}

enum Failed {
    /// An embedded PDF the writer couldn't copy.
    #[cfg(not(target_arch = "wasm32"))]
    Pdf(krilla::pdf::PdfDocument),
    Other(ZError),
}

impl Failed {
    fn into_error(self) -> ZError {
        match self {
            Failed::Other(e) => e,
            #[cfg(not(target_arch = "wasm32"))]
            Failed::Pdf(_) => ZError::new("export_failed", "PDF export failed: a PDF page couldn't be copied"),
        }
    }
}

fn export_once(r: &Renderer, editor: &Editor, raster_pages: bool) -> Result<Vec<u8>, Failed> {
    let vp = [editor.view.viewport.x.max(1.0), editor.view.viewport.y.max(1.0)];
    let mut pages: Vec<(zebra_core::Camera, SceneOptions)> = vec![];
    if editor.doc.path.is_empty() {
        let cam = match editor.doc.bounds() {
            Some(b) => zebra_core::Camera::fit_rect(&b, editor.view.viewport, 0.06),
            None => zebra_core::Camera::default(),
        };
        pages.push((cam, SceneOptions { presenting: true, raster_pages, ..Default::default() }));
    } else {
        for i in 0..editor.doc.path.len() {
            let opts = SceneOptions {
                presenting: true,
                hidden: editor.hidden_builds_at(i, usize::MAX),
                raster_pages,
                ..Default::default()
            };
            pages.push((crate::cpu::camera_for_step(editor, i, vp), opts));
        }
    }

    let mut doc = Document::new();
    doc.set_metadata(krilla::metadata::Metadata::new().title(editor.doc.title.clone()));
    let mut res = Resources::default();
    let size = Size::from_wh(vp[0] as f32, vp[1] as f32).ok_or_else(|| Failed::Other(ZError::new("invalid", "bad page size")))?;
    for (cam, opts) in &pages {
        let list = crate::display::build_with(r, editor, cam, vp, 1.0, opts, true);
        let mut page = doc.start_page_with(krilla::page::PageSettings::new(size));
        let mut s = page.surface();
        draw_list(&mut s, &list, editor, &mut res);
        s.finish();
        page.finish();
    }
    // Copying embedded pages happens here.
    doc.finish().map_err(|e| match e {
        #[cfg(not(target_arch = "wasm32"))]
        krilla::error::KrillaError::Pdf(d, ..) => Failed::Pdf(d),
        e => Failed::Other(ZError::new("export_failed", format!("PDF export failed: {e:?}"))),
    })
}

#[derive(Default)]
struct Resources {
    /// Per font face and variable-font location (axis tag, value bits):
    /// krilla cuts a variable font to a plain one at that location, so
    /// the PDF carries only the weights the deck uses.
    fonts: HashMap<(u64, u32, Vec<([u8; 4], u32)>), Option<Font>>,
    images: HashMap<String, Option<krilla::image::Image>>,
    /// Pictures the PDF can't embed as files (SVG symbols...), as pixels:
    /// one per asset and size, however many times they are drawn.
    pixels: HashMap<(String, u32, u32), krilla::image::Image>,
    /// Bytes the originals embedded so far take decoded (PNG, GIF and
    /// WebP are decoded to be embedded; JPEG goes in as it is).
    decoded: u64,
    /// Per font face: whether the PDF may carry it (`font_rights::pdf_use`).
    may_embed: HashMap<(u64, u32), bool>,
}

/// Pixels per point text in a font the PDF can't carry is drawn at (about
/// 300 dots per inch on paper).
const TEXT_PICTURE_SCALE: f64 = 300.0 / 72.0;
/// Most pixels one run of such text is drawn with: what a document may
/// take in memory ([`zebra_core::io::memory_budget`]), at 4 bytes a pixel
/// and twice over (the drawing and the PDF's copy). A huge heading at a
/// close zoom is drawn coarser rather than taking more; never cut.
fn text_picture_pixels() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    let budget = zebra_core::io::memory_budget();
    #[cfg(target_arch = "wasm32")]
    let budget = 1u64 << 31;
    (budget / 8).max(1 << 20) as f64
}

/// A run of text in a font the PDF may not carry (Apple's, or one that
/// doesn't say it is open), drawn as a picture where it stands (fonts in
/// export rule: such fonts are never embedded).
fn text_as_picture(s: &mut Surface<'_>, layout: &crate::text::TextLayout, ri: usize, color: Rgba, transform: &Affine) {
    let run = &layout.runs[ri];
    if run.glyphs.is_empty() {
        return;
    }
    let size = run.font_size as f64;
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for g in &run.glyphs {
        x0 = x0.min(g.x as f64 - size * 0.3);
        x1 = x1.max((g.x + g.advance) as f64 + size * 0.3);
        y0 = y0.min(g.y as f64 - size * 1.2);
        y1 = y1.max(g.y as f64 + size * 0.5);
    }
    let (bw, bh) = (x1 - x0, y1 - y0);
    if !(bw > 0.0 && bh > 0.0 && bw.is_finite() && bh.is_finite()) {
        return;
    }
    let on_page = crate::display::affine_scale(transform);
    let mut k = on_page * TEXT_PICTURE_SCALE;
    let most = text_picture_pixels();
    if bw * bh * k * k > most {
        k = (most / (bw * bh)).sqrt();
    }
    let (pw, ph) = ((bw * k).ceil().max(1.0) as u32, (bh * k).ceil().max(1.0) as u32);
    let one = crate::text::TextLayout { runs: vec![run.clone()], decorations: vec![], links: vec![], ..layout.clone() };
    let list = DisplayList {
        width_px: pw,
        height_px: ph,
        background: [0, 0, 0, 0],
        items: vec![Item::Text { layout: Arc::new(one), transform: Affine::scale(k) * Affine::translate((-x0, -y0)), color, run_colors: Arc::from(vec![color]) }],
        next_frame_in: None,
        images_pending: false,
    };
    let mut px = crate::cpu::render_pixmap(&list).data_as_u8_slice().to_vec();
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let img = krilla::image::Image::from_rgba8(px, pw, ph);
    if let Some(sz) = Size::from_wh(bw as f32, bh as f32) {
        s.push_transform(&tf(&(Affine::translate((x0, y0)))));
        s.draw_image(img, sz);
        s.pop();
    }
}

/// Decoded bytes of original PNG, GIF and WebP files one PDF export may
/// hold. Past it a picture goes in as the pixels drawn for the page: like
/// Keynote's Reduce File Size, scaled to the resolution the presentation
/// shows it at, so it looks the same, and a 70,000-pixel PNG never needs
/// 19 GB to export.
pub const PDF_DECODE_BUDGET: u64 = crate::images::DECODE_BYTES;

fn fill_of(c: Rgba) -> Fill {
    Fill {
        paint: rgb::Color::new(c[0], c[1], c[2]).into(),
        opacity: NormalizedF32::new(c[3] as f32 / 255.0).unwrap_or(NormalizedF32::ONE),
        rule: FillRule::NonZero,
    }
}

fn to_path(p: &BezPath) -> Option<krilla::geom::Path> {
    let mut b = PathBuilder::new();
    for el in p.elements() {
        match *el {
            PathEl::MoveTo(a) => b.move_to(a.x as f32, a.y as f32),
            PathEl::LineTo(a) => b.line_to(a.x as f32, a.y as f32),
            PathEl::QuadTo(a, c) => b.quad_to(a.x as f32, a.y as f32, c.x as f32, c.y as f32),
            PathEl::CurveTo(a, c, d) => {
                b.cubic_to(a.x as f32, a.y as f32, c.x as f32, c.y as f32, d.x as f32, d.y as f32)
            }
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

fn tf(a: &Affine) -> Transform {
    let c = a.as_coeffs().map(|v| v as f32);
    Transform::from_row(c[0], c[1], c[2], c[3], c[4], c[5])
}

fn draw_list(s: &mut Surface<'_>, list: &DisplayList, editor: &Editor, res: &mut Resources) {
    let (w, h) = (list.width_px as f64, list.height_px as f64);
    if let Some(p) = to_path(&kurbo::Shape::to_path(&kurbo::Rect::new(0.0, 0.0, w, h), 0.1)) {
        s.set_stroke(None);
        s.set_fill(Some(fill_of(list.background)));
        s.draw_path(&p);
    }
    for item in &list.items {
        match item {
            Item::Fill { path, color } => {
                if let Some(p) = to_path(path) {
                    s.set_stroke(None);
                    s.set_fill(Some(fill_of(*color)));
                    s.draw_path(&p);
                }
            }
            Item::Stroke { path, color, stroke } => {
                if let Some(p) = to_path(path) {
                    let f = fill_of(*color);
                    s.set_fill(None);
                    s.set_stroke(Some(Stroke {
                        paint: f.paint,
                        width: stroke.width as f32,
                        miter_limit: stroke.miter_limit as f32,
                        line_cap: match stroke.start_cap {
                            kurbo::Cap::Butt => LineCap::Butt,
                            kurbo::Cap::Round => LineCap::Round,
                            kurbo::Cap::Square => LineCap::Square,
                        },
                        line_join: match stroke.join {
                            kurbo::Join::Bevel => LineJoin::Bevel,
                            kurbo::Join::Miter => LineJoin::Miter,
                            kurbo::Join::Round => LineJoin::Round,
                        },
                        opacity: f.opacity,
                        dash: (!stroke.dash_pattern.is_empty()).then(|| StrokeDash {
                            array: stroke.dash_pattern.iter().map(|d| *d as f32).collect(),
                            offset: stroke.dash_offset as f32,
                        }),
                    }));
                    s.draw_path(&p);
                    s.set_stroke(None);
                }
            }
            Item::Text { layout, transform, color, run_colors } => {
                s.set_stroke(None);
                s.push_transform(&tf(transform));
                for (ri, run) in layout.runs.iter().enumerate() {
                    let run_color = run_colors.get(ri).copied().unwrap_or(*color);
                    let face = (run.font.data.id(), run.font.index);
                    let may = *res.may_embed.entry(face).or_insert_with(|| crate::font_rights::pdf_use(run.font.data.data(), run.font.index) == crate::font_rights::WebUse::Ship);
                    if !may {
                        text_as_picture(s, layout, ri, run_color, transform);
                        continue;
                    }
                    s.set_fill(Some(fill_of(run_color)));
                    let at: Vec<([u8; 4], u32)> = run.variations.iter().map(|(t, v)| (*t, v.to_bits())).collect();
                    let key = (run.font.data.id(), run.font.index, at);
                    let font = res
                        .fonts
                        .entry(key)
                        .or_insert_with(|| {
                            let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(run.font.data.clone());
                            if run.variations.is_empty() {
                                Font::new(Data::from(data), run.font.index)
                            } else {
                                let vars: Vec<(krilla::text::Tag, f32)> = run.variations.iter().map(|(t, v)| (krilla::text::Tag::new(t), *v)).collect();
                                Font::new_variable(Data::from(data), run.font.index, &vars)
                            }
                        })
                        .clone();
                    let Some(font) = font else { continue };
                    let size = run.font_size;
                    let g0 = run.glyphs[0];
                    let glyphs: Vec<KrillaGlyph> = run
                        .glyphs
                        .iter()
                        .enumerate()
                        .map(|(i, g)| {
                            let adv = run.glyphs.get(i + 1).map(|n| n.x - g.x).unwrap_or(g.advance);
                            KrillaGlyph {
                                glyph_id: GlyphId::new(g.id),
                                text_range: g.start as usize..g.end as usize,
                                x_advance: adv / size,
                                x_offset: 0.0,
                                y_offset: -(g.y - g0.y) / size,
                                y_advance: 0.0,
                                location: None,
                            }
                        })
                        .collect();
                    if let Some(skew) = run.skew {
                        // Synthetic italic: shear around the baseline.
                        let k = -(skew as f64).to_radians().tan();
                        let t = Affine::translate((0.0, g0.y as f64))
                            * Affine::skew(k, 0.0)
                            * Affine::translate((0.0, -g0.y as f64));
                        s.push_transform(&tf(&t));
                    }
                    s.draw_glyphs(Point::from_xy(g0.x, g0.y), &glyphs, font, &layout.text, size, false);
                    if run.skew.is_some() {
                        s.pop();
                    }
                }
                s.pop();
            }
            Item::Image { image, transform } => {
                let img = res
                    .images
                    .entry(image.asset.clone())
                    .or_insert_with(|| {
                        let a = editor.doc.assets.get(&image.asset)?;
                        if !matches!(a.ext.as_str(), "jpg" | "jpeg") {
                            // Its size decoded, from the header; unknown or
                            // over what's left: the drawn pixels instead.
                            let head = &a.bytes.as_ref()[..a.bytes.as_ref().len().min(256 << 10)];
                            let px = zebra_core::media::picture_header(head).map(|h| h.pixels())?;
                            let need = px.saturating_mul(4);
                            if res.decoded.saturating_add(need) > PDF_DECODE_BUDGET {
                                return None;
                            }
                            res.decoded += need;
                        }
                        let data = Data::from(std::sync::Arc::new(a.bytes.clone()) as std::sync::Arc<dyn AsRef<[u8]> + Send + Sync>);
                        match a.ext.as_str() {
                            "png" => krilla::image::Image::from_png(data, true).ok(),
                            "jpg" | "jpeg" => krilla::image::Image::from_jpeg(data, true).ok(),
                            "gif" => krilla::image::Image::from_gif(data, true).ok(),
                            "webp" => krilla::image::Image::from_webp(data, true).ok(),
                            _ => None,
                        }
                    })
                    .clone()
                    // SVG symbols (and anything else without a file PDF can
                    // embed): the pixels drawn for the page.
                    .or_else(|| {
                        let key = (image.asset.clone(), image.width, image.height);
                        if let Some(i) = res.pixels.get(&key) {
                            return Some(i.clone());
                        }
                        let mut px = image.data.data.data().to_vec();
                        for p in px.chunks_exact_mut(4) {
                            let a = p[3] as u32;
                            if a != 0 && a != 255 {
                                for c in &mut p[..3] {
                                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                                }
                            }
                        }
                        let i = krilla::image::Image::from_rgba8(px, image.width, image.height);
                        res.pixels.insert(key, i.clone());
                        Some(i)
                    });
                let Some(img) = img else { continue };
                if let Some(sz) = Size::from_wh(image.width as f32, image.height as f32) {
                    s.push_transform(&tf(transform));
                    s.draw_image(img, sz);
                    s.pop();
                }
            }
            Item::PushOpacity(a) => s.push_opacity(NormalizedF32::new(a.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE)),
            Item::PopOpacity => s.pop(),
            Item::PushClip(path) => {
                // Always push, so the matching PopClip pops this layer.
                let p = to_path(path);
                match p {
                    Some(p) => s.push_clip_path(&p, &FillRule::NonZero),
                    None => s.push_opacity(NormalizedF32::ONE),
                }
            }
            Item::PopClip => s.pop(),
            #[cfg(not(target_arch = "wasm32"))]
            Item::PdfPage { source, index, size, transform } => {
                if let Some(sz) = Size::from_wh(size.0 as f32, size.1 as f32) {
                    s.push_transform(&tf(transform));
                    s.draw_pdf_page(&source.0, sz, *index);
                    s.pop();
                }
            }
        }
    }
}
