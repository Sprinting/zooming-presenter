//! Geometry for the shape library and the icon library.
//!
//! Every shape is built in the element's local box, centred on the origin
//! (x right, y down), `w` by `h`. Lines and arrows are handled by the
//! display list; this module covers the closed shapes.
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::sync::{Arc, Mutex, OnceLock};

use kurbo::{Affine, BezPath, Circle, Ellipse, Point, Rect, RoundedRect, Shape};
use zebra_core::ShapeKind;

/// A closed shape: the outline that gets filled and stroked, plus optional
/// detail lines (the cylinder's rim, the sticky note's fold) that are only
/// stroked.
pub struct ShapeGeom {
    pub outline: BezPath,
    pub detail: Option<BezPath>,
}

fn poly(pts: &[(f64, f64)]) -> BezPath {
    let mut p = BezPath::new();
    for (i, q) in pts.iter().enumerate() {
        if i == 0 {
            p.move_to(*q);
        } else {
            p.line_to(*q);
        }
    }
    p.close_path();
    p
}

/// Scale a path non-uniformly so its bounding box is exactly the w x h box.
fn fit(mut p: BezPath, w: f64, h: f64) -> BezPath {
    let b = p.bounding_box();
    if b.width() > 0.0 && b.height() > 0.0 {
        p.apply_affine(Affine::scale_non_uniform(w / b.width(), h / b.height()) * Affine::translate((-b.center().x, -b.center().y)));
    }
    p
}

/// A regular polygon (or star when `inner` is set) fitted to the box.
fn radial(n: usize, inner: Option<f64>, w: f64, h: f64) -> BezPath {
    let steps = if inner.is_some() { n * 2 } else { n };
    let pts: Vec<(f64, f64)> = (0..steps)
        .map(|i| {
            let a = -FRAC_PI_2 + TAU * i as f64 / steps as f64;
            let r = if i % 2 == 1 { inner.unwrap_or(1.0) } else { 1.0 };
            (a.cos() * r, a.sin() * r)
        })
        .collect();
    fit(poly(&pts), w, h)
}

/// Block arrow pointing right, in a w x h box.
fn block_arrow(w: f64, h: f64, both: bool) -> BezPath {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let head = (h * 0.9).min(w * if both { 0.3 } else { 0.45 });
    let t = hh * 0.5;
    if both {
        poly(&[(-hw, 0.0), (-hw + head, -hh), (-hw + head, -t), (hw - head, -t), (hw - head, -hh), (hw, 0.0), (hw - head, hh), (hw - head, t), (-hw + head, t), (-hw + head, hh)])
    } else {
        poly(&[(-hw, -t), (hw - head, -t), (hw - head, -hh), (hw, 0.0), (hw - head, hh), (hw - head, t), (-hw, t)])
    }
}

fn rounded(w: f64, h: f64, r: f64) -> BezPath {
    RoundedRect::from_rect(Rect::new(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0), r.min(w / 2.0).min(h / 2.0)).to_path(0.01)
}

/// A box with a tail at the bottom-left, its body occupying the top 78%.
fn bubble(w: f64, h: f64, r: f64) -> BezPath {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let bottom = hh - h * 0.22;
    let r = r.min(w / 2.0).min((bottom + hh) / 2.0);
    let (t0, t1, tip) = (-hw + w * 0.18, -hw + w * 0.36, (-hw + w * 0.1, hh));
    let mut p = BezPath::new();
    p.move_to((-hw + r, -hh));
    p.line_to((hw - r, -hh));
    p.quad_to((hw, -hh), (hw, -hh + r));
    p.line_to((hw, bottom - r));
    p.quad_to((hw, bottom), (hw - r, bottom));
    p.line_to((t1, bottom));
    p.line_to(tip);
    p.line_to((t0, bottom));
    p.line_to((-hw + r, bottom));
    p.quad_to((-hw, bottom), (-hw, bottom - r));
    p.line_to((-hw, -hh + r));
    p.quad_to((-hw, -hh), (-hw + r, -hh));
    p.close_path();
    p
}

/// Puffy outline: arcs bulging out between points on an ellipse.
fn cloud(n: usize, w: f64, h: f64) -> BezPath {
    let mut p = BezPath::new();
    let at = |a: f64, k: f64| Point::new(a.cos() * k, a.sin() * k);
    let step = TAU / n as f64;
    for i in 0..n {
        let a0 = -FRAC_PI_2 + step * i as f64 + 0.15 * (i % 2) as f64 * step;
        let a1 = -FRAC_PI_2 + step * (i + 1) as f64 + 0.15 * ((i + 1) % 2) as f64 * step;
        if i == 0 {
            p.move_to(at(a0, 1.0));
        }
        let (c0, c1) = (at(a0 + (a1 - a0) * 0.1, 1.45), at(a1 - (a1 - a0) * 0.1, 1.45));
        p.curve_to(c0, c1, at(a1, 1.0));
    }
    p.close_path();
    fit(p, w, h)
}

pub fn geometry(kind: ShapeKind, w: f64, h: f64, corner_radius: f64) -> ShapeGeom {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let m = w.min(h);
    let cr = |default: f64| if corner_radius > 0.0 { corner_radius } else { default };
    let mut detail = None;
    use ShapeKind as K;
    let outline = match kind {
        K::Rectangle => rounded(w, h, corner_radius),
        K::RoundedRect => rounded(w, h, cr(m * 0.15)),
        K::Pill => rounded(w, h, m / 2.0),
        K::Ellipse => Ellipse::new((0.0, 0.0), (hw, hh), 0.0).to_path(0.01),
        K::Triangle => poly(&[(0.0, -hh), (hw, hh), (-hw, hh)]),
        K::RightTriangle => poly(&[(-hw, -hh), (hw, hh), (-hw, hh)]),
        K::Diamond => poly(&[(0.0, -hh), (hw, 0.0), (0.0, hh), (-hw, 0.0)]),
        K::Pentagon => radial(5, None, w, h),
        K::Hexagon => {
            let d = (w * 0.25).min(h * 0.5);
            poly(&[(-hw, 0.0), (-hw + d, -hh), (hw - d, -hh), (hw, 0.0), (hw - d, hh), (-hw + d, hh)])
        }
        K::Octagon => {
            let (dx, dy) = (w * 0.29, h * 0.29);
            poly(&[(-hw + dx, -hh), (hw - dx, -hh), (hw, -hh + dy), (hw, hh - dy), (hw - dx, hh), (-hw + dx, hh), (-hw, hh - dy), (-hw, -hh + dy)])
        }
        K::Parallelogram => {
            let d = (w * 0.25).min(h);
            poly(&[(-hw + d, -hh), (hw, -hh), (hw - d, hh), (-hw, hh)])
        }
        K::Trapezoid => {
            let d = (w * 0.2).min(h);
            poly(&[(-hw + d, -hh), (hw - d, -hh), (hw, hh), (-hw, hh)])
        }
        K::Star => radial(5, Some(0.42), w, h),
        K::Sparkle => {
            // Four points with concave sides.
            let mut p = BezPath::new();
            let pts = [(0.0, -hh), (hw, 0.0), (0.0, hh), (-hw, 0.0)];
            p.move_to(pts[0]);
            for i in 0..4 {
                p.quad_to((0.0, 0.0), pts[(i + 1) % 4]);
            }
            p.close_path();
            p
        }
        K::Burst => radial(12, Some(0.72), w, h),
        K::Heart => {
            let mut p = BezPath::new();
            p.move_to((0.0, 1.0));
            p.curve_to((-0.55, 0.62), (-1.0, 0.25), (-1.0, -0.3));
            p.curve_to((-1.0, -0.85), (-0.25, -1.05), (0.0, -0.55));
            p.curve_to((0.25, -1.05), (1.0, -0.85), (1.0, -0.3));
            p.curve_to((1.0, 0.25), (0.55, 0.62), (0.0, 1.0));
            p.close_path();
            fit(p, w, h)
        }
        K::Plus => {
            let (tx, ty) = (w / 6.0, h / 6.0);
            poly(&[(-tx, -hh), (tx, -hh), (tx, -ty), (hw, -ty), (hw, ty), (tx, ty), (tx, hh), (-tx, hh), (-tx, ty), (-hw, ty), (-hw, -ty), (-tx, -ty)])
        }
        K::Ring => {
            let mut p = Ellipse::new((0.0, 0.0), (hw, hh), 0.0).to_path(0.01);
            let inner = Ellipse::new((0.0, 0.0), (hw * 0.55, hh * 0.55), 0.0).to_path(0.01).reverse_subpaths();
            p.extend(inner);
            p
        }
        K::Moon => {
            // Crescent: outer arc of the full circle, inner arc of an offset circle.
            let mut p = BezPath::new();
            p.move_to((0.25, -0.968));
            p.curve_to((-0.9, -0.85), (-1.25, 0.45), (-0.45, 0.89));
            p.curve_to((-0.1, 1.08), (0.35, 1.02), (0.6, 0.8));
            p.curve_to((-0.1, 0.75), (-0.45, 0.1), (-0.25, -0.4));
            p.curve_to((-0.15, -0.7), (0.05, -0.9), (0.25, -0.968));
            p.close_path();
            fit(p, w, h)
        }
        K::Lightning => fit(poly(&[(0.55, 0.0), (0.0, 0.55), (0.3, 0.55), (0.1, 1.0), (0.75, 0.38), (0.45, 0.38), (0.7, 0.0)]), w, h),
        K::Cloud => cloud(9, w, h),
        K::SpeechBubble => bubble(w, h, cr(m * 0.2)),
        K::Callout => bubble(w, h, corner_radius),
        K::ThoughtBubble => {
            let body_h = h * 0.78;
            let mut p = cloud(9, w, body_h);
            p.apply_affine(Affine::translate((0.0, -hh + body_h / 2.0)));
            let r = m * 0.07;
            p.extend(Circle::new((-hw + w * 0.24, hh - r * 1.9), r).to_path(0.01));
            p.extend(Circle::new((-hw + w * 0.12, hh - r * 0.6), r * 0.55).to_path(0.01));
            p
        }
        K::StickyNote => {
            let f = m * 0.16;
            let r = cr(m * 0.02);
            let mut p = BezPath::new();
            p.move_to((-hw + r, -hh));
            p.line_to((hw - r, -hh));
            p.quad_to((hw, -hh), (hw, -hh + r));
            p.line_to((hw, hh - f));
            p.line_to((hw - f, hh));
            p.line_to((-hw + r, hh));
            p.quad_to((-hw, hh), (-hw, hh - r));
            p.line_to((-hw, -hh + r));
            p.quad_to((-hw, -hh), (-hw + r, -hh));
            p.close_path();
            detail = Some(poly(&[(hw, hh - f), (hw - f, hh - f), (hw - f, hh)]));
            p
        }
        K::Document => {
            let wave = h * 0.08;
            let mut p = BezPath::new();
            p.move_to((-hw, -hh));
            p.line_to((hw, -hh));
            p.line_to((hw, hh - wave * 2.0));
            p.curve_to((hw * 0.5, hh - wave * 4.0), (-hw * 0.4, hh + wave * 0.5), (-hw, hh - wave));
            p.close_path();
            p
        }
        K::Cylinder => {
            let ry = (h * 0.12).min(w * 0.3);
            let top = -hh + ry;
            let bot = hh - ry;
            let mut p = BezPath::new();
            p.move_to((-hw, top));
            // Upper half of the top ellipse.
            p.curve_to((-hw, top - ry * 1.333), (hw, top - ry * 1.333), (hw, top));
            p.line_to((hw, bot));
            p.curve_to((hw, bot + ry * 1.333), (-hw, bot + ry * 1.333), (-hw, bot));
            p.close_path();
            let mut d = BezPath::new();
            d.move_to((-hw, top));
            d.curve_to((-hw, top + ry * 1.333), (hw, top + ry * 1.333), (hw, top));
            detail = Some(d);
            p
        }
        K::Cube => {
            let d = m * 0.22;
            let p = poly(&[(-hw, -hh + d), (-hw + d, -hh), (hw, -hh), (hw, hh - d), (hw - d, hh), (-hw, hh)]);
            let mut det = BezPath::new();
            det.move_to((-hw, -hh + d));
            det.line_to((hw - d, -hh + d));
            det.line_to((hw, -hh));
            det.move_to((hw - d, -hh + d));
            det.line_to((hw - d, hh));
            detail = Some(det);
            p
        }
        K::Delay => {
            let r = hh.min(w);
            let mut p = BezPath::new();
            p.move_to((-hw, -hh));
            p.line_to((hw - r, -hh));
            p.curve_to((hw - r + r * 0.552, -hh), (hw, -hh + r * (1.0 - 0.552)), (hw, -hh + r));
            p.line_to((hw, hh - r));
            p.curve_to((hw, hh - r + r * 0.552), (hw - r + r * 0.552, hh), (hw - r, hh));
            p.line_to((-hw, hh));
            p.close_path();
            p
        }
        K::Chevron => {
            let d = (w * 0.3).min(hh);
            poly(&[(-hw, -hh), (hw - d, -hh), (hw, 0.0), (hw - d, hh), (-hw, hh), (-hw + d, 0.0)])
        }
        K::Tag => {
            let d = (w * 0.3).min(hh);
            poly(&[(-hw, -hh), (hw - d, -hh), (hw, 0.0), (hw - d, hh), (-hw, hh)])
        }
        K::BlockArrowRight => block_arrow(w, h, false),
        K::BlockArrowBoth => block_arrow(w, h, true),
        K::BlockArrowLeft => {
            let mut p = block_arrow(w, h, false);
            p.apply_affine(Affine::FLIP_X);
            p
        }
        K::BlockArrowUp | K::BlockArrowDown => {
            // Build pointing right in the transposed box, then rotate.
            let mut p = block_arrow(h, w, false);
            p.apply_affine(Affine::rotate(if kind == K::BlockArrowUp { -FRAC_PI_2 } else { FRAC_PI_2 }));
            p
        }
        // Lines are drawn by the caller; give them a box for hit tests.
        K::Line | K::Arrow | K::DoubleArrow => Rect::new(-hw, -hh, hw, hh).to_path(0.01),
    };
    ShapeGeom { outline, detail }
}

/// Parsed icon outlines on the 24x24 grid, cached by name.
pub fn icon_path(name: &str) -> Option<Arc<BezPath>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<BezPath>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(p) = cache.lock().unwrap().get(name) {
        return Some(p.clone());
    }
    let icon = zebra_core::icons::get(name)?;
    let mut path = BezPath::new();
    for d in icon.path_data() {
        if let Ok(p) = BezPath::from_svg(d) {
            path.extend(p);
        }
    }
    let path = Arc::new(path);
    cache.lock().unwrap().insert(name.to_string(), path.clone());
    Some(path)
}

/// Transform from the 24-unit icon grid to a w x h local box (uniform, centred).
pub fn icon_affine(w: f64, h: f64) -> Affine {
    let k = w.min(h) / 24.0;
    Affine::scale(k) * Affine::translate((-12.0, -12.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_fits_its_box() {
        for &k in ShapeKind::ALL {
            let g = geometry(k, 300.0, 200.0, 0.0);
            let b = g.outline.bounding_box();
            assert!(b.width() > 100.0 && b.height() > 60.0, "{k:?} too small: {b:?}");
            assert!(b.x0 >= -150.5 && b.x1 <= 150.5 && b.y0 >= -100.5 && b.y1 <= 100.5, "{k:?} spills: {b:?}");
            if !k.is_line() {
                assert!(g.outline.area().abs() > 300.0 * 200.0 * 0.15, "{k:?} area {}", g.outline.area());
            }
        }
    }

    #[test]
    fn every_icon_parses() {
        let bad: Vec<&str> = zebra_core::icons::all()
            .iter()
            .filter(|i| i.path_data().any(|d| BezPath::from_svg(d).is_err()))
            .map(|i| i.name)
            .collect();
        assert!(bad.is_empty(), "unparseable icons: {bad:?}");
        let p = icon_path("lightbulb").unwrap();
        let b = p.bounding_box();
        assert!(b.x0 >= 0.0 && b.x1 <= 24.0 && b.y0 >= 0.0 && b.y1 <= 24.0);
    }
}
