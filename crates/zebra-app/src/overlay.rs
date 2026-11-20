//! UI overlays (egui shapes drawn over the canvas) that stay cheap at any
//! zoom. An element's outline is in screen points, so deep inside a zoom it
//! can be millions of points long: dashing all of it made millions of egui
//! shapes a frame (gigabytes within seconds). Here only the part near the
//! visible area is drawn.
use egui::{Painter, Pos2, Shape, Stroke};
use kurbo::{Point, Rect};

/// How far past the painter's clip rect (in points) overlays are kept.
const MARGIN: f32 = 64.0;

/// The area overlays are cut to: the painter's clip rect plus a margin.
pub fn view_of(painter: &Painter) -> Rect {
    let r = painter.clip_rect().expand(MARGIN);
    Rect::new(r.min.x as f64, r.min.y as f64, r.max.x as f64, r.max.y as f64)
}

/// The visible runs of a polyline (closed: back to the first point), each
/// with its distance along the whole line.
pub fn visible_runs(pts: &[Pos2], closed: bool, view: Rect) -> Vec<(Vec<Pos2>, f32)> {
    let pts: Vec<Point> = pts.iter().map(|p| Point::new(p.x as f64, p.y as f64)).collect();
    zebra_render::clip_polyline(&pts, closed, view)
        .into_iter()
        .map(|(run, d)| (run.into_iter().map(|p| Pos2::new(p.x as f32, p.y as f32)).collect(), d as f32))
        .collect()
}

/// Dashed polyline for overlays: dashes only where it can be seen, the
/// pattern continuous along the whole line.
pub fn dashed(painter: &Painter, pts: &[Pos2], closed: bool, stroke: Stroke, dash: f32, gap: f32) {
    painter.extend(dashed_shapes(pts, closed, stroke, dash, gap, view_of(painter)));
}

/// The shapes [`dashed`] adds, for a given view (tests count them).
pub fn dashed_shapes(pts: &[Pos2], closed: bool, stroke: Stroke, dash: f32, gap: f32, view: Rect) -> Vec<Shape> {
    let period = (dash + gap).max(0.01);
    let mut out = vec![];
    for (run, d) in visible_runs(pts, closed, view) {
        // epaint starts each line with a gap of `offset`: start the run at
        // the next dash of the whole line's pattern.
        let offset = (period - d % period) % period;
        Shape::dashed_line_many_with_offset(&run, stroke, &[dash], &[gap], offset, &mut out);
    }
    out
}

/// Solid polyline for overlays, cut to what can be seen.
pub fn line(painter: &Painter, pts: &[Pos2], closed: bool, stroke: Stroke) {
    for (run, _) in visible_runs(pts, closed, view_of(painter)) {
        painter.add(Shape::line(run, stroke));
    }
}
