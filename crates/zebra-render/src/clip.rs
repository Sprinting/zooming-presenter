//! Clip enormous device-space paths to (a margin around) the viewport.
//!
//! When the camera is deep inside a large element, its outline can be
//! millions of pixels long. Rasterisers handle that badly: a dashed outline
//! of an invisible frame becomes tens of millions of dashes, and vello's GPU
//! buffers overflow (wgpu validation error, then a crash or a stuck canvas).
//! Here such paths are reduced to the part near the screen before they reach
//! any backend. Paths that already fit are left untouched.

use kurbo::{BezPath, CubicBez, ParamCurve, PathEl, Point, Rect, Shape, Stroke};

use crate::display::Item;

/// Clip every oversized Fill/Stroke in `items` for a `w`x`h` px target.
pub(crate) fn clip_items(items: &mut Vec<Item>, w: f64, h: f64) {
    let m = 0.25 * w.max(h) + 64.0;
    let clip = Rect::new(-m, -m, w + m, h + m);
    let big = clip.width().max(clip.height());
    // Only paths reaching well beyond the clip area are processed.
    let limit = clip.inflate(big * 4.0, big * 4.0);
    let needs = |p: &BezPath, pad: f64| {
        let b = p.bounding_box();
        !(b.x0 >= limit.x0 - pad && b.y0 >= limit.y0 - pad && b.x1 <= limit.x1 + pad && b.y1 <= limit.y1 + pad)
            || !b.x0.is_finite()
            || !b.y1.is_finite()
    };
    if !items.iter().any(|it| match it {
        Item::Fill { path, .. } => needs(path, 0.0),
        Item::Stroke { path, stroke, .. } => needs(path, stroke.width),
        _ => false,
    }) {
        return;
    }
    let old = std::mem::take(items);
    for it in old {
        match it {
            Item::Fill { path, color } if needs(&path, 0.0) => {
                let p = clip_fill(&path, clip);
                if !p.elements().is_empty() {
                    items.push(Item::Fill { path: p, color });
                }
            }
            Item::Stroke { path, color, stroke } if needs(&path, stroke.width) => {
                let c = clip.inflate(stroke.width, stroke.width);
                for (p, off) in clip_stroke(&path, c, &stroke) {
                    let mut s = stroke.clone();
                    s.dash_offset = off;
                    items.push(Item::Stroke { path: p, color, stroke: s });
                }
            }
            other => items.push(other),
        }
    }
}

const TOL: f64 = 0.1;

/// Flatten `path` into polylines, replacing curve pieces whose control hull
/// misses `clip` by their chord (exact for clipping purposes: the region
/// between curve and chord lies inside the hull). Returns (points, closed).
fn polylines(path: &BezPath, clip: Rect) -> Vec<(Vec<Point>, bool)> {
    let mut out: Vec<(Vec<Point>, bool)> = vec![];
    let mut cur: Vec<Point> = vec![];
    let mut last = Point::ZERO;
    let flush = |cur: &mut Vec<Point>, out: &mut Vec<(Vec<Point>, bool)>, closed: bool| {
        if cur.len() >= 2 {
            out.push((std::mem::take(cur), closed));
        } else {
            cur.clear();
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                flush(&mut cur, &mut out, false);
                cur.push(p);
                last = p;
            }
            PathEl::LineTo(p) => {
                if cur.is_empty() {
                    cur.push(last);
                }
                cur.push(p);
                last = p;
            }
            PathEl::QuadTo(p1, p2) => {
                if cur.is_empty() {
                    cur.push(last);
                }
                let c = kurbo::QuadBez::new(last, p1, p2).raise();
                cubic(c, clip, &mut cur, 0);
                last = p2;
            }
            PathEl::CurveTo(p1, p2, p3) => {
                if cur.is_empty() {
                    cur.push(last);
                }
                cubic(CubicBez::new(last, p1, p2, p3), clip, &mut cur, 0);
                last = p3;
            }
            PathEl::ClosePath => {
                if let Some(&f) = cur.first() {
                    last = f;
                }
                flush(&mut cur, &mut out, true);
            }
        }
    }
    flush(&mut cur, &mut out, false);
    out
}

fn cubic(c: CubicBez, clip: Rect, out: &mut Vec<Point>, depth: u32) {
    let hull = Rect::from_points(c.p0, c.p1).union_pt(c.p2).union_pt(c.p3);
    if depth > 48 || !hull_touches(&hull, &clip) {
        out.push(c.p3);
        return;
    }
    let size = hull.width().max(hull.height());
    if size <= 4.0 * clip.width().max(clip.height()) {
        let mut tmp = BezPath::new();
        tmp.move_to(c.p0);
        tmp.curve_to(c.p1, c.p2, c.p3);
        kurbo::flatten(tmp, TOL, |el| {
            if let PathEl::LineTo(p) = el {
                out.push(p)
            }
        });
        return;
    }
    let (a, b) = c.subdivide();
    cubic(a, clip, out, depth + 1);
    cubic(b, clip, out, depth + 1);
}

fn hull_touches(a: &Rect, b: &Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Sutherland–Hodgman against the clip rectangle, per subpath. Valid for
/// filling with the non-zero rule (it may add degenerate edges on the border).
fn clip_fill(path: &BezPath, clip: Rect) -> BezPath {
    let mut out = BezPath::new();
    for (pts, _) in polylines(path, clip) {
        let mut poly = pts;
        for edge in 0..4 {
            poly = clip_edge(&poly, clip, edge);
            if poly.is_empty() {
                break;
            }
        }
        if poly.len() >= 3 {
            out.move_to(poly[0]);
            for p in &poly[1..] {
                out.line_to(*p);
            }
            out.close_path();
        }
    }
    out
}

fn clip_edge(poly: &[Point], r: Rect, edge: u8) -> Vec<Point> {
    let inside = |p: Point| match edge {
        0 => p.x >= r.x0,
        1 => p.x <= r.x1,
        2 => p.y >= r.y0,
        _ => p.y <= r.y1,
    };
    let cross = |a: Point, b: Point| -> Point {
        let t = match edge {
            0 => (r.x0 - a.x) / (b.x - a.x),
            1 => (r.x1 - a.x) / (b.x - a.x),
            2 => (r.y0 - a.y) / (b.y - a.y),
            _ => (r.y1 - a.y) / (b.y - a.y),
        };
        a.lerp(b, t.clamp(0.0, 1.0))
    };
    let mut out = Vec::with_capacity(poly.len() + 4);
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[(i + n - 1) % n], poly[i]);
        match (inside(a), inside(b)) {
            (true, true) => out.push(b),
            (true, false) => out.push(cross(a, b)),
            (false, true) => {
                out.push(cross(a, b));
                out.push(b);
            }
            (false, false) => {}
        }
    }
    out
}

/// Liang–Barsky: parameter range of segment a→b inside r.
fn clip_segment(a: Point, b: Point, r: Rect) -> Option<(f64, f64)> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 1.0;
    for (p, q) in [(-dx, a.x - r.x0), (dx, r.x1 - a.x), (-dy, a.y - r.y0), (dy, r.y1 - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    Some((t0, t1))
}

/// Visible runs of a stroked path, each with the dash offset that keeps the
/// dash pattern continuous with the unclipped path.
fn clip_stroke(path: &BezPath, clip: Rect, st: &Stroke) -> Vec<(BezPath, f64)> {
    let dashed = !st.dash_pattern.is_empty();
    let mut runs = vec![];
    for (mut pts, closed) in polylines(path, clip) {
        if closed {
            pts.push(pts[0]);
        }
        let mut dist = 0.0;
        // (path, dash offset, last point if the run reached its segment end)
        let mut run: Option<(BezPath, f64, Option<Point>)> = None;
        let mut done: Vec<(BezPath, f64)> = vec![];
        let mut starts_at_start = false;
        let nseg = pts.len() - 1;
        for i in 0..nseg {
            let (a, b) = (pts[i], pts[i + 1]);
            let len = a.distance(b);
            match clip_segment(a, b, clip) {
                Some((t0, t1)) => {
                    let (p0, p1) = (a.lerp(b, t0), a.lerp(b, t1));
                    let continues = t0 == 0.0 && run.as_ref().is_some_and(|r| r.2 == Some(a));
                    if !continues {
                        if let Some((p, off, _)) = run.take() {
                            done.push((p, off));
                        }
                        if i == 0 && t0 == 0.0 {
                            starts_at_start = true;
                        }
                        let mut p = BezPath::new();
                        p.move_to(p0);
                        run = Some((p, st.dash_offset + dist + t0 * len, None));
                    }
                    let r = run.as_mut().unwrap();
                    r.0.line_to(p1);
                    r.2 = (t1 == 1.0).then_some(b);
                }
                None => {
                    if let Some((p, off, _)) = run.take() {
                        done.push((p, off));
                    }
                }
            }
            dist += len;
        }
        let ends_at_end = run.as_ref().is_some_and(|r| r.2 == Some(pts[nseg]));
        if let Some((p, off, _)) = run.take() {
            done.push((p, off));
        }
        // A closed solid outline visible across its start point: join the last
        // run onto the first so that corner's join is drawn.
        if closed && !dashed && starts_at_start && ends_at_end {
            if done.len() == 1 {
                done[0].0.close_path();
            } else {
                let (last, _) = done.pop().unwrap();
                let (first, off) = done.remove(0);
                let mut merged = last;
                for el in first.elements().iter().skip(1) {
                    merged.push(*el);
                }
                done.insert(0, (merged, off));
            }
        }
        let real = done;
        runs.extend(real);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Ellipse, Shape};

    #[test]
    fn small_paths_untouched() {
        let mut items = vec![Item::Fill { path: Rect::new(10.0, 10.0, 50.0, 50.0).to_path(0.1), color: [0; 4] }];
        let before = format!("{items:?}");
        clip_items(&mut items, 100.0, 100.0);
        assert_eq!(before, format!("{items:?}"));
    }

    #[test]
    fn huge_ellipse_fill_is_bounded() {
        let e = Ellipse::new((50.0, 1e9), (1e9, 1e9), 0.0);
        let mut items = vec![Item::Fill { path: e.to_path(0.1), color: [1; 4] }];
        clip_items(&mut items, 100.0, 100.0);
        let Item::Fill { path, .. } = &items[0] else { panic!() };
        let b = path.bounding_box();
        assert!(b.x0 >= -100.0 && b.x1 <= 200.0 && b.y0 >= -100.0 && b.y1 <= 200.0, "{b:?}");
        assert!(path.elements().len() < 2000);
        // the view is inside the ellipse (centre 1e9 below, radius 1e9 reaches y=0)
        assert!(path.contains(Point::new(50.0, 50.0)));
    }

    #[test]
    fn huge_dashed_rect_has_few_dashes() {
        let r = Rect::new(-1e8, -1e8, 1e8, 50.0);
        let st = Stroke::new(1.0).with_dashes(0.0, [6.0, 4.0]);
        let mut items = vec![Item::Stroke { path: r.to_path(0.1), color: [1; 4], stroke: st }];
        clip_items(&mut items, 100.0, 100.0);
        assert_eq!(items.len(), 1, "only the bottom edge is near the view");
        let Item::Stroke { path, stroke, .. } = &items[0] else { panic!() };
        let b = path.bounding_box();
        assert!(b.width() < 400.0);
        // phase continuity: the run starts 1e8 + ~(1e8 - 89) px into the outline
        assert!(stroke.dash_offset > 1e8);
    }

    #[test]
    fn huge_solid_rect_stroke_keeps_corner_join() {
        let r = Rect::new(20.0, 20.0, 1e9, 1e9);
        let mut items = vec![Item::Stroke { path: r.to_path(0.1), color: [1; 4], stroke: Stroke::new(4.0) }];
        clip_items(&mut items, 100.0, 100.0);
        assert_eq!(items.len(), 1, "start corner visible: runs merged into one");
    }
}
