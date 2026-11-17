//! A simple egui-painter rendering of the document. Used only when the GPU
//! canvas is unavailable (UI tests without wgpu, or if vello fails to start),
//! so the app is never a blank window. The real renderer is zebra-render.
use std::collections::HashSet;

use egui::{epaint::TextShape, Color32, Painter, Pos2, Shape, Stroke};
use zebra_core::{Camera, ElementKind, FrameStyle, ShapeKind, Theme, Vec2};

use crate::style::{c32, doc_family};

pub fn paint(
    painter: &Painter,
    editor: &zebra_core::Editor,
    cam: &Camera,
    origin: Pos2,
    vp: Vec2,
    _presenting: bool,
    hidden: &HashSet<String>,
    skip: Option<&str>,
    opacity: &[(String, f32)],
) {
    let theme = editor.theme();
    painter.rect_filled(painter.clip_rect(), 0.0, c32(theme.color(&"theme:background".into())));
    let w2s = |p: Vec2| {
        let s = cam.world_to_screen(p, vp);
        Pos2::new(origin.x + s.x as f32, origin.y + s.y as f32)
    };
    let visible = cam.visible_bounds(vp).inflate(10.0 / cam.zoom);
    for e in editor.doc.ordered() {
        if hidden.contains(&e.id) || skip == Some(e.id.as_str()) || !visible.intersects(&e.bounds()) {
            continue;
        }
        let alpha = opacity.iter().find(|(id, _)| *id == e.id).map(|o| o.1).unwrap_or(1.0);
        let col = |c: &str| c32(theme.color(&c.to_string())).gamma_multiply(alpha);
        let px = (e.transform.scale * cam.zoom) as f32;
        let (hw, hh) = (e.size.w / 2.0, e.size.h / 2.0);
        let local = |x: f64, y: f64| w2s(e.transform.apply(Vec2::new(x, y)));
        let quad = || vec![local(-hw, -hh), local(hw, -hh), local(hw, hh), local(-hw, hh)];
        match &e.kind {
            ElementKind::Frame(f) => {
                let fc = col(f.color.as_deref().unwrap_or("theme:frame"));
                let sw = (e.size.w.min(e.size.h) * 0.012 * px as f64).clamp(1.0, 8.0) as f32;
                match f.style {
                    FrameStyle::Bracket => {
                        let arm = e.size.w.min(e.size.h) * 0.08;
                        for sx in [-1.0, 1.0] {
                            let x = sx * hw;
                            let pts = vec![local(x - sx * arm, -hh), local(x, -hh), local(x, hh), local(x - sx * arm, hh)];
                            painter.add(Shape::line(pts, Stroke::new(sw, fc)));
                        }
                    }
                    FrameStyle::Circle => {
                        let c = w2s(e.transform.center());
                        let r = (hw.min(hh) as f32) * px;
                        painter.circle(c, r, col("theme:frame_fill").gamma_multiply(0.6), Stroke::new(sw, fc));
                    }
                    FrameStyle::Rectangle => {
                        painter.add(Shape::convex_polygon(quad(), col("theme:frame_fill"), Stroke::NONE));
                    }
                    FrameStyle::Invisible => {}
                }
            }
            ElementKind::Text(t) => {
                let style = theme.text_style(t.style);
                let size = style.size as f32 * px;
                if !(1.5..4000.0).contains(&size) {
                    continue;
                }
                let color = col(t.color.as_deref().unwrap_or(match t.style {
                    zebra_core::TextStyle::Title => "theme:title",
                    zebra_core::TextStyle::Subtitle => "theme:subtitle",
                    zebra_core::TextStyle::Body => "theme:body",
                }));
                let text = if t.bullets {
                    t.text.lines().map(|l| format!("• {l}")).collect::<Vec<_>>().join("\n")
                } else {
                    t.text.clone()
                };
                let font = egui::FontId::new(size, doc_family(&style.font, t.bold, t.italic));
                let wrap = if t.fixed_width { e.size.w as f32 * px } else { f32::INFINITY };
                let galley = painter.layout(text, font, color, wrap);
                let tl = local(-hw, -hh);
                painter.add(Shape::Text(TextShape::new(tl, galley, color).with_angle(e.transform.rotation.to_radians() as f32 - cam.rotation.to_radians() as f32)));
            }
            ElementKind::Shape(s) => {
                let fill = s.fill.as_deref().map(col).unwrap_or(Color32::TRANSPARENT);
                let stroke = s.stroke.as_deref().map(|c| Stroke::new((s.stroke_width as f32 * px).max(1.0), col(c))).unwrap_or(Stroke::NONE);
                if s.shape.is_line() {
                    let st = if stroke == Stroke::NONE { Stroke::new((s.stroke_width as f32 * px).max(1.0), fill) } else { stroke };
                    painter.line_segment([local(-hw, 0.0), local(hw, 0.0)], st);
                    let head = (e.size.h * 0.5).min(e.size.w * 0.3).max(s.stroke_width * 3.0);
                    if s.shape != ShapeKind::Line {
                        painter.add(Shape::line(vec![local(hw - head, -head * 0.6), local(hw, 0.0), local(hw - head, head * 0.6)], st));
                    }
                    if s.shape == ShapeKind::DoubleArrow {
                        painter.add(Shape::line(vec![local(-hw + head, -head * 0.6), local(-hw, 0.0), local(-hw + head, head * 0.6)], st));
                    }
                } else {
                    let fill = if s.fill.is_none() && s.stroke.is_none() { col("theme:shape1") } else { fill };
                    let g = zebra_render::shapes::geometry(s.shape, e.size.w, e.size.h, s.corner_radius);
                    for pts in flatten(&g.outline, &local) {
                        painter.add(Shape::convex_polygon(pts, fill, stroke));
                    }
                }
            }
            ElementKind::Icon(i) => {
                if let Some(path) = zebra_render::shapes::icon_path(&i.name) {
                    let mut p = (*path).clone();
                    p.apply_affine(zebra_render::shapes::icon_affine(e.size.w, e.size.h));
                    let st = Stroke::new((i.stroke_width * e.size.w.min(e.size.h) / 24.0 * px as f64).max(1.0) as f32, col(&i.color));
                    for pts in flatten(&p, &local) {
                        painter.add(Shape::line(pts, st));
                    }
                }
            }
            ElementKind::Image(_) => {
                painter.add(Shape::convex_polygon(quad(), Color32::from_gray(225).gamma_multiply(alpha), Stroke::new(1.0, Color32::from_gray(180))));
                painter.line_segment([local(-hw, -hh), local(hw, hh)], Stroke::new(1.0, Color32::from_gray(190)));
                painter.line_segment([local(hw, -hh), local(-hw, hh)], Stroke::new(1.0, Color32::from_gray(190)));
            }
            ElementKind::Ink(k) => {
                let mut c = col(&k.color);
                if k.highlighter {
                    c = c.gamma_multiply(0.4);
                }
                let pts: Vec<Pos2> = k.points.iter().map(|p| local(p.x, p.y)).collect();
                painter.add(Shape::line(pts, Stroke::new((k.width as f32 * px).max(1.0), c)));
            }
        }
    }
}

/// Theme lookups used across the UI.
pub fn theme_color(theme: &Theme, c: &str) -> Color32 {
    c32(theme.color(&c.to_string()))
}

/// Flatten a local-space path into screen polylines, one per subpath.
fn flatten(path: &kurbo::BezPath, local: &dyn Fn(f64, f64) -> Pos2) -> Vec<Vec<Pos2>> {
    let mut out: Vec<Vec<Pos2>> = Vec::new();
    kurbo::flatten(path, 0.5, |el| match el {
        kurbo::PathEl::MoveTo(p) => out.push(vec![local(p.x, p.y)]),
        kurbo::PathEl::LineTo(p) => {
            if let Some(v) = out.last_mut() {
                v.push(local(p.x, p.y));
            }
        }
        kurbo::PathEl::ClosePath => {
            if let Some(v) = out.last_mut() {
                if let Some(&f) = v.first() {
                    v.push(f);
                }
            }
        }
        _ => {}
    });
    out
}
