//! Clip enormous device-space paths to (a margin around) the viewport.
//!
//! When the camera is deep inside a large element, its outline can be
//! millions of pixels long. Rasterisers handle that badly: a dashed outline
//! of an invisible frame becomes tens of millions of dashes, and vello's GPU
//! buffers overflow (wgpu validation error, then a crash or a stuck canvas).
//! Here such paths are reduced to the part near the screen before they reach
//! any backend. Paths that already fit are left untouched.
//!
//! The same holds for text and pictures: a text box the camera is inside
//! sends only the glyphs near the screen, a glyph many screens tall becomes
//! a clipped outline, and pictures off screen are dropped. Last, a frame's
//! total geometry is capped ([`MAX_FRAME_COST`]) so no element kind can grow
//! a backend's buffers without bound.

use std::sync::Arc;

use kurbo::{Affine, BezPath, CubicBez, ParamCurve, PathEl, Point, Rect, Shape, Stroke};

use crate::display::{affine_scale, Item};
use crate::text::TextLayout;

/// Most geometry (path elements, dashes and glyphs) one frame may hand a
/// backend. Past it, further items are left out of that frame: a glitch at
/// an absurd zoom is better than gigabytes of renderer buffers.
pub const MAX_FRAME_COST: usize = 3_000_000;

/// Clip every oversized item in `items` for a `w`x`h` px target.
pub(crate) fn clip_items(items: &mut Vec<Item>, w: f64, h: f64) {
    tame_strokes(items, w, h);
    clip_paths(items, w, h);
    clip_text_and_images(items, w, h);
    cap_items(items, MAX_FRAME_COST);
}

/// Widest stroke handed to a backend, in viewport diagonals: wider than
/// this covers the whole target wherever its path is.
const MAX_STROKE_DIAGONALS: f64 = 4.0;
/// Segments shorter than this fraction of the stroke width are left out.
const DEGENERATE: f64 = 1e-6;

/// Strokes as backends can take them (hostile-input audit C3): no wider
/// than a few viewport diagonals (vello_cpu flattened a 1e19 px outline
/// into 16 GB), and without segments that are near-zero long next to the
/// width (vello_cpu panicked inside its thread pool on one, which aborts
/// the process). A subpath of nothing but such segments stays a dot.
fn tame_strokes(items: &mut Vec<Item>, w: f64, h: f64) {
    let max = MAX_STROKE_DIAGONALS * w.hypot(h).max(1.0);
    items.retain_mut(|it| {
        let Item::Stroke { path, stroke, .. } = it else { return true };
        if !stroke.width.is_finite() || stroke.width <= 0.0 {
            return false;
        }
        stroke.width = stroke.width.min(max);
        if let Some(p) = drop_degenerate(path, stroke.width * DEGENERATE) {
            *path = p;
        }
        !path.elements().is_empty()
    });
}

/// `path` without segments whose points all lie within `eps` of where the
/// segment starts; None if there are none.
fn drop_degenerate(path: &BezPath, eps: f64) -> Option<BezPath> {
    let near = |a: Point, b: Point| (a.x - b.x).abs() <= eps && (a.y - b.y).abs() <= eps;
    let mut last = Point::ZERO;
    let mut any = false;
    for el in path.elements() {
        let tiny = match *el {
            PathEl::MoveTo(p) => {
                last = p;
                false
            }
            PathEl::LineTo(p) => near(last, p),
            PathEl::QuadTo(a, p) => near(last, a) && near(last, p),
            PathEl::CurveTo(a, b, p) => near(last, a) && near(last, b) && near(last, p),
            PathEl::ClosePath => false,
        };
        if tiny {
            any = true;
            break;
        }
        if let PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) = *el {
            last = p;
        }
    }
    if !any {
        return None;
    }
    let mut out = BezPath::new();
    // Per subpath: its start, the last point kept, whether a segment was kept.
    let mut start = Point::ZERO;
    let mut last = Point::ZERO;
    let mut kept = false;
    let mut dropped = false;
    let end_sub = |out: &mut BezPath, start: Point, kept: bool, dropped: bool| {
        if !kept && dropped {
            // A dot: one zero-length segment, so round caps still draw it.
            out.line_to(start);
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                end_sub(&mut out, start, kept, dropped);
                out.move_to(p);
                (start, last, kept, dropped) = (p, p, false, false);
            }
            PathEl::LineTo(p) => {
                if near(last, p) {
                    dropped = true;
                } else {
                    out.line_to(p);
                    (last, kept) = (p, true);
                }
            }
            PathEl::QuadTo(a, p) => {
                if near(last, a) && near(last, p) {
                    dropped = true;
                } else {
                    out.quad_to(a, p);
                    (last, kept) = (p, true);
                }
            }
            PathEl::CurveTo(a, b, p) => {
                if near(last, a) && near(last, b) && near(last, p) {
                    dropped = true;
                } else {
                    out.curve_to(a, b, p);
                    (last, kept) = (p, true);
                }
            }
            PathEl::ClosePath => {
                end_sub(&mut out, start, kept, dropped);
                out.close_path();
                (last, kept, dropped) = (start, false, false);
            }
        }
    }
    end_sub(&mut out, start, kept, dropped);
    Some(out)
}

/// The area kept around a `w`x`h` px target, and the larger area beyond
/// which items are processed at all.
fn areas(w: f64, h: f64) -> (Rect, Rect) {
    let m = 0.25 * w.max(h) + 64.0;
    let clip = Rect::new(-m, -m, w + m, h + m);
    let big = clip.width().max(clip.height());
    (clip, clip.inflate(big * 4.0, big * 4.0))
}

fn inside(b: &Rect, limit: &Rect) -> bool {
    b.x0 >= limit.x0 && b.y0 >= limit.y0 && b.x1 <= limit.x1 && b.y1 <= limit.y1
}

fn finite(b: &Rect) -> bool {
    b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()
}

/// Fill/Stroke/clip paths reaching far outside the target, cut to it.
fn clip_paths(items: &mut Vec<Item>, w: f64, h: f64) {
    // Only paths reaching well beyond the clip area are processed.
    let (clip, limit) = areas(w, h);
    let needs = |p: &BezPath, pad: f64| {
        let b = p.bounding_box();
        !(b.x0 >= limit.x0 - pad && b.y0 >= limit.y0 - pad && b.x1 <= limit.x1 + pad && b.y1 <= limit.y1 + pad)
            || !b.x0.is_finite()
            || !b.y1.is_finite()
    };
    if !items.iter().any(|it| match it {
        Item::Fill { path, .. } => needs(path, 0.0),
        Item::Stroke { path, stroke, .. } => needs(path, stroke.width),
        Item::PushClip(path) => needs(path, 0.0),
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
            // A clip region can be cut to the visible area like a fill.
            Item::PushClip(path) if needs(&path, 0.0) => items.push(Item::PushClip(clip_fill(&path, clip))),
            other => items.push(other),
        }
    }
}

/// A glyph whose em is more than this many target sizes tall is drawn as
/// a clipped outline instead of a glyph: rasterisers flatten glyph curves
/// at their full size, so their work grows without bound with the zoom.
const GIANT_GLYPH: f64 = 4.0;

/// Text whose glyphs reach far outside the target keeps only the glyphs
/// near it; giant glyphs become clipped outlines. Pictures entirely
/// outside the target are dropped.
fn clip_text_and_images(items: &mut Vec<Item>, w: f64, h: f64) {
    let (clip, limit) = areas(w, h);
    let big = clip.width().max(clip.height());
    let needs = |it: &Item| match it {
        Item::Text { layout, transform, .. } => {
            let em = layout.runs.iter().map(|r| r.font_size as f64).fold(0.0, f64::max);
            let b = transform.transform_rect_bbox(Rect::new(0.0, 0.0, layout.width, layout.height).inflate(2.0 * em, 2.0 * em));
            !finite(&b) || !inside(&b, &limit) || em * affine_scale(transform) > GIANT_GLYPH * big
        }
        Item::Image { image, transform } => {
            let b = transform.transform_rect_bbox(Rect::new(0.0, 0.0, image.width as f64, image.height as f64));
            !finite(&b) || !hull_touches(&b, &clip)
        }
        _ => false,
    };
    if !items.iter().any(needs) {
        return;
    }
    let old = std::mem::take(items);
    for it in old {
        if !needs(&it) {
            items.push(it);
            continue;
        }
        match it {
            Item::Text { layout, transform, color, run_colors } => {
                let (kept, outlines) = cull_glyphs(&layout, &transform, clip, big);
                if let Some(kept) = kept {
                    items.push(Item::Text { layout: kept, transform, color, run_colors: run_colors.clone() });
                }
                for (ri, path) in outlines {
                    let path = clip_fill(&path, clip);
                    if !path.elements().is_empty() {
                        items.push(Item::Fill { path, color: run_colors.get(ri).copied().unwrap_or(color) });
                    }
                }
            }
            // Off screen (or not finite): nothing to draw.
            Item::Image { .. } => {}
            other => items.push(other),
        }
    }
}

/// The glyphs of `layout` that can reach `clip` (None when none can), and
/// the giant ones as device-space outlines with their run index. Runs keep
/// their order (run colours are indexed by run).
fn cull_glyphs(layout: &Arc<TextLayout>, transform: &Affine, clip: Rect, big: f64) -> (Option<Arc<TextLayout>>, Vec<(usize, BezPath)>) {
    let s = affine_scale(transform);
    let mut out = (**layout).clone();
    let mut outlines = vec![];
    let mut any = false;
    for (ri, run) in out.runs.iter_mut().enumerate() {
        let em = run.font_size as f64;
        let giant = em * s > GIANT_GLYPH * big;
        let font = run.font.clone();
        let (size, coords, skew) = (run.font_size, run.coords.clone(), run.skew);
        run.glyphs.retain(|g| {
            let (x, y) = (g.x as f64, g.y as f64);
            let r = Rect::new(x - em, y - 1.5 * em, x + (g.advance as f64).max(0.0) + em, y + 0.7 * em);
            let b = transform.transform_rect_bbox(r);
            if !finite(&b) || !hull_touches(&b, &clip) {
                return false;
            }
            if giant {
                if let Some(mut p) = glyph_outline(&font, g.id, size, &coords, skew) {
                    p.apply_affine(*transform * Affine::translate((x, y)));
                    outlines.push((ri, p));
                }
                return false;
            }
            true
        });
        any |= !run.glyphs.is_empty();
    }
    (any.then(|| Arc::new(out)), outlines)
}

/// A glyph's outline in layout space (y down, origin on the baseline).
fn glyph_outline(font: &crate::text::FontData, id: u32, size: f32, coords: &[i16], skew: Option<f32>) -> Option<BezPath> {
    use skrifa::instance::{LocationRef, Size};
    use skrifa::outline::{DrawSettings, OutlinePen};
    use skrifa::raw::types::F2Dot14;
    use skrifa::MetadataProvider;
    struct Pen(BezPath, f64);
    impl Pen {
        fn p(&self, x: f32, y: f32) -> Point {
            // Font space is y up; synthetic italics lean the top to the right.
            Point::new(x as f64 + self.1 * y as f64, -(y as f64))
        }
    }
    impl OutlinePen for Pen {
        fn move_to(&mut self, x: f32, y: f32) {
            let p = self.p(x, y);
            self.0.move_to(p);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            let p = self.p(x, y);
            self.0.line_to(p);
        }
        fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
            let (c, p) = (self.p(cx, cy), self.p(x, y));
            self.0.quad_to(c, p);
        }
        fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
            let (a, b, p) = (self.p(ax, ay), self.p(bx, by), self.p(x, y));
            self.0.curve_to(a, b, p);
        }
        fn close(&mut self) {
            self.0.close_path();
        }
    }
    let f = skrifa::FontRef::from_index(font.data.as_ref(), font.index).ok()?;
    let glyph = f.outline_glyphs().get(skrifa::GlyphId::new(id))?;
    let coords: Vec<F2Dot14> = coords.iter().map(|c| F2Dot14::from_bits(*c)).collect();
    let lean = skew.map_or(0.0, |a| (a as f64).to_radians().tan());
    let mut pen = Pen(BezPath::new(), lean);
    glyph.draw(DrawSettings::unhinted(Size::new(size), LocationRef::new(&coords)), &mut pen).ok()?;
    Some(pen.0)
}

/// For UI overlays drawn outside the display list (egui): the runs of a
/// screen-space polyline inside `clip`, each with its distance from the
/// line's start (so a dash pattern stays continuous). An outline millions
/// of pixels long, seen from deep inside, comes back as a few short runs.
pub fn clip_polyline(pts: &[Point], closed: bool, clip: Rect) -> Vec<(Vec<Point>, f64)> {
    let mut runs: Vec<(Vec<Point>, f64)> = vec![];
    if pts.len() < 2 {
        return runs;
    }
    let n = if closed { pts.len() } else { pts.len() - 1 };
    let mut dist = 0.0;
    let mut open = false;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        let len = a.distance(b);
        if !(a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()) {
            open = false;
            continue;
        }
        match clip_segment(a, b, clip) {
            Some((t0, t1)) => {
                let (p0, p1) = (a.lerp(b, t0), a.lerp(b, t1));
                if !(open && t0 == 0.0) {
                    runs.push((vec![p0], dist + t0 * len));
                }
                runs.last_mut().unwrap().0.push(p1);
                open = t1 == 1.0;
            }
            None => open = false,
        }
        dist += len;
    }
    runs
}

/// Rough work an item hands a backend: path elements, dashes, glyphs.
fn cost(it: &Item) -> usize {
    match it {
        Item::Fill { path, .. } | Item::PushClip(path) => path.elements().len(),
        Item::Stroke { path, stroke, .. } => {
            let n = path.elements().len();
            let period: f64 = stroke.dash_pattern.iter().sum();
            if period <= 0.0 {
                return n;
            }
            // Upper bound on the dashes: control polygon length / period.
            let mut len = 0.0;
            let mut last = Point::ZERO;
            let mut start = Point::ZERO;
            let mut step = |p: Point, last: &mut Point| {
                len += last.distance(p);
                *last = p;
            };
            for el in path.elements() {
                match *el {
                    PathEl::MoveTo(p) => {
                        last = p;
                        start = p;
                    }
                    PathEl::LineTo(p) => step(p, &mut last),
                    PathEl::QuadTo(a, b) => {
                        step(a, &mut last);
                        step(b, &mut last);
                    }
                    PathEl::CurveTo(a, b, c) => {
                        step(a, &mut last);
                        step(b, &mut last);
                        step(c, &mut last);
                    }
                    PathEl::ClosePath => step(start, &mut last),
                }
            }
            let dashes = len / period;
            n.saturating_add(if dashes.is_finite() { dashes.min(1e15) as usize } else { usize::MAX / 2 })
        }
        Item::Text { layout, .. } => layout.runs.iter().map(|r| r.glyphs.len()).sum(),
        Item::Image { .. } => 4,
        _ => 0,
    }
}

/// The geometry a display list hands a backend (see [`MAX_FRAME_COST`]).
pub fn frame_cost(items: &[Item]) -> usize {
    items.iter().fold(0usize, |t, it| t.saturating_add(cost(it)))
}

/// Leave out drawing items once the frame's total cost passes `budget`.
/// Layer pushes and pops stay so layers stay balanced; a clip pushed after
/// that point clips to nothing.
fn cap_items(items: &mut Vec<Item>, budget: usize) {
    let mut total = 0usize;
    let mut over = false;
    let old = std::mem::take(items);
    for it in old {
        match it {
            Item::PushOpacity(_) | Item::PopOpacity | Item::PopClip => items.push(it),
            Item::PushClip(_) if over => items.push(Item::PushClip(BezPath::new())),
            it => {
                if over {
                    continue;
                }
                total = total.saturating_add(cost(&it));
                if total > budget {
                    over = true;
                    if matches!(it, Item::PushClip(_)) {
                        items.push(Item::PushClip(BezPath::new()));
                    }
                    continue;
                }
                items.push(it);
            }
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
    fn absurd_stroke_width_is_clamped_to_the_view() {
        let mut items = vec![Item::Stroke { path: Rect::new(10.0, 10.0, 50.0, 50.0).to_path(0.1), color: [1; 4], stroke: Stroke::new(1e19) }];
        clip_items(&mut items, 100.0, 100.0);
        let Item::Stroke { stroke, .. } = &items[0] else { panic!() };
        assert!(stroke.width <= 4.0 * 100f64.hypot(100.0) + 1e-9, "{}", stroke.width);
    }

    #[test]
    fn near_zero_segments_are_dropped_but_dots_stay() {
        let mut p = BezPath::new();
        p.move_to((10.0, 10.0));
        p.line_to((10.0, 10.0 + 1e-9));
        p.line_to((60.0, 10.0));
        let mut dot = BezPath::new();
        dot.move_to((30.0, 30.0));
        dot.line_to((30.0, 30.0));
        let mut items = vec![
            Item::Stroke { path: p, color: [1; 4], stroke: Stroke::new(100.0) },
            Item::Stroke { path: dot, color: [1; 4], stroke: Stroke::new(8.0) },
        ];
        clip_items(&mut items, 100.0, 100.0);
        let Item::Stroke { path, .. } = &items[0] else { panic!() };
        assert_eq!(path.elements(), &[PathEl::MoveTo(Point::new(10.0, 10.0)), PathEl::LineTo(Point::new(60.0, 10.0))]);
        let Item::Stroke { path, .. } = &items[1] else { panic!() };
        assert_eq!(path.elements().len(), 2, "a dot keeps its zero-length segment");
    }

    #[test]
    fn huge_solid_rect_stroke_keeps_corner_join() {
        let r = Rect::new(20.0, 20.0, 1e9, 1e9);
        let mut items = vec![Item::Stroke { path: r.to_path(0.1), color: [1; 4], stroke: Stroke::new(4.0) }];
        clip_items(&mut items, 100.0, 100.0);
        assert_eq!(items.len(), 1, "start corner visible: runs merged into one");
    }
}
