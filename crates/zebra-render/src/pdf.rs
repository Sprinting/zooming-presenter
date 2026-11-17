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

pub(crate) fn export(r: &Renderer, editor: &Editor) -> Result<Vec<u8>, ZError> {
    let vp = [editor.view.viewport.x.max(1.0), editor.view.viewport.y.max(1.0)];
    let mut pages: Vec<(zebra_core::Camera, SceneOptions)> = vec![];
    if editor.doc.path.is_empty() {
        let cam = match editor.doc.bounds() {
            Some(b) => zebra_core::Camera::fit_rect(&b, editor.view.viewport, 0.06),
            None => zebra_core::Camera::default(),
        };
        pages.push((cam, SceneOptions { presenting: true, ..Default::default() }));
    } else {
        for i in 0..editor.doc.path.len() {
            let opts = SceneOptions {
                presenting: true,
                hidden: editor.hidden_builds_at(i, usize::MAX),
                ..Default::default()
            };
            pages.push((crate::cpu::camera_for_step(editor, i, vp), opts));
        }
    }

    let mut doc = Document::new();
    doc.set_metadata(krilla::metadata::Metadata::new().title(editor.doc.title.clone()));
    let mut res = Resources::default();
    let size = Size::from_wh(vp[0] as f32, vp[1] as f32).ok_or_else(|| ZError::new("invalid", "bad page size"))?;
    for (cam, opts) in &pages {
        let list = crate::display::build_with(r, editor, cam, vp, 1.0, opts, true);
        let mut page = doc.start_page_with(krilla::page::PageSettings::new(size));
        let mut s = page.surface();
        draw_list(&mut s, &list, editor, &mut res);
        s.finish();
        page.finish();
    }
    doc.finish().map_err(|e| ZError::new("export_failed", format!("PDF export failed: {e:?}")))
}

#[derive(Default)]
struct Resources {
    fonts: HashMap<(u64, u32), Option<Font>>,
    images: HashMap<String, Option<krilla::image::Image>>,
}

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
            Item::Text { layout, transform, color } => {
                s.set_stroke(None);
                s.set_fill(Some(fill_of(*color)));
                s.push_transform(&tf(transform));
                for run in &layout.runs {
                    let key = (run.font.data.id(), run.font.index);
                    let font = res
                        .fonts
                        .entry(key)
                        .or_insert_with(|| {
                            let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(run.font.data.clone());
                            Font::new(Data::from(data), run.font.index)
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
                        let data = Data::from(std::sync::Arc::new(a.bytes.clone()) as std::sync::Arc<dyn AsRef<[u8]> + Send + Sync>);
                        match a.ext.as_str() {
                            "png" => krilla::image::Image::from_png(data, true).ok(),
                            "jpg" | "jpeg" => krilla::image::Image::from_jpeg(data, true).ok(),
                            "gif" => krilla::image::Image::from_gif(data, true).ok(),
                            "webp" => krilla::image::Image::from_webp(data, true).ok(),
                            _ => None,
                        }
                    })
                    .clone();
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
        }
    }
}
