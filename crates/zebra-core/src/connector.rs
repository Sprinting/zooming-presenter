//! Connectors: lines whose ends stick to elements.
//!
//! After every command the editor calls [`refresh`], which puts each
//! attached end on the edge of its element (facing the other end), records
//! the exit direction, and refits the connector's box around its route. The
//! route itself is a pure function of the connector's data, so renderers and
//! hit tests need no document access.
use crate::geom::{Rect, Transform, Vec2};
use crate::model::{ConnectorData, Document, Element, ElementKind, FrameStyle, Route, ShapeKind, Size};

/// A connector's centre line in its own local coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum RoutePath {
    Poly(Vec<Vec2>),
    /// One cubic Bézier: start, control 1, control 2, end.
    Cubic([Vec2; 4]),
}

impl RoutePath {
    /// Points along the route (curves sampled), for hit tests and bounds.
    pub fn sample(&self) -> Vec<Vec2> {
        match self {
            RoutePath::Poly(p) => p.clone(),
            RoutePath::Cubic([a, b, c, d]) => (0..=24)
                .map(|i| {
                    let t = i as f64 / 24.0;
                    let u = 1.0 - t;
                    *a * (u * u * u) + *b * (3.0 * u * u * t) + *c * (3.0 * u * t * t) + *d * (t * t * t)
                })
                .collect(),
        }
    }
    /// Point halfway along the route, for the label.
    pub fn midpoint(&self) -> Vec2 {
        let pts = self.sample();
        let total: f64 = pts.windows(2).map(|w| (w[1] - w[0]).len()).sum();
        let mut left = total / 2.0;
        for w in pts.windows(2) {
            let l = (w[1] - w[0]).len();
            if l >= left && l > 0.0 {
                return w[0] + (w[1] - w[0]) * (left / l);
            }
            left -= l;
        }
        pts.first().copied().unwrap_or_default()
    }
    /// Unit direction the route arrives at its end (`at_end`) or leaves its start.
    pub fn tangent(&self, at_end: bool) -> Vec2 {
        let (a, b) = match self {
            RoutePath::Poly(p) if p.len() >= 2 => {
                if at_end {
                    (p[p.len() - 2], p[p.len() - 1])
                } else {
                    (p[1], p[0])
                }
            }
            RoutePath::Cubic([s, c1, c2, e]) => {
                if at_end {
                    (if (*e - *c2).len() > 1e-9 { *c2 } else { *s }, *e)
                } else {
                    (if (*s - *c1).len() > 1e-9 { *c1 } else { *e }, *s)
                }
            }
            _ => return Vec2::new(1.0, 0.0),
        };
        unit(b - a).unwrap_or(Vec2::new(1.0, 0.0))
    }
}

fn unit(v: Vec2) -> Option<Vec2> {
    let l = v.len();
    (l > 1e-12).then(|| v * (1.0 / l))
}

fn horizontal(d: Vec2) -> bool {
    d.x.abs() >= d.y.abs()
}

/// The route for given end points and exit directions.
pub fn route(kind: Route, p0: Vec2, d0: Vec2, p1: Vec2, d1: Vec2) -> RoutePath {
    match kind {
        Route::Straight => RoutePath::Poly(vec![p0, p1]),
        Route::Curved => {
            let k = (p1 - p0).len() * 0.4;
            let d0 = unit(d0).unwrap_or_else(|| unit(p1 - p0).unwrap_or_default());
            let d1 = unit(d1).unwrap_or_else(|| unit(p0 - p1).unwrap_or_default());
            RoutePath::Cubic([p0, p0 + d0 * k, p1 + d1 * k, p1])
        }
        Route::Elbow => {
            let d0 = unit(d0).unwrap_or_else(|| unit(p1 - p0).unwrap_or(Vec2::new(1.0, 0.0)));
            let d1 = unit(d1).unwrap_or_else(|| unit(p0 - p1).unwrap_or(Vec2::new(-1.0, 0.0)));
            let pts = match (horizontal(d0), horizontal(d1)) {
                (true, true) => {
                    let mx = (p0.x + p1.x) / 2.0;
                    vec![p0, Vec2::new(mx, p0.y), Vec2::new(mx, p1.y), p1]
                }
                (false, false) => {
                    let my = (p0.y + p1.y) / 2.0;
                    vec![p0, Vec2::new(p0.x, my), Vec2::new(p1.x, my), p1]
                }
                (true, false) => vec![p0, Vec2::new(p1.x, p0.y), p1],
                (false, true) => vec![p0, Vec2::new(p0.x, p1.y), p1],
            };
            RoutePath::Poly(pts)
        }
    }
}

/// The connector's route in its local coordinates.
pub fn local_route(c: &ConnectorData) -> RoutePath {
    route(c.route, Vec2::new(c.from.x, c.from.y), c.from.dir, Vec2::new(c.to.x, c.to.y), c.to.dir)
}

/// Distance from a local point to the route.
pub fn distance(c: &ConnectorData, p: Vec2) -> f64 {
    local_route(c)
        .sample()
        .windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let l2 = ab.x * ab.x + ab.y * ab.y;
            let t = if l2 > 0.0 { (((p - a).x * ab.x + (p - a).y * ab.y) / l2).clamp(0.0, 1.0) } else { 0.0 };
            (p - (a + ab * t)).len()
        })
        .fold(f64::INFINITY, f64::min)
}

fn round_outline(e: &Element) -> bool {
    match &e.kind {
        ElementKind::Shape(s) => matches!(s.shape, ShapeKind::Ellipse | ShapeKind::Ring),
        ElementKind::Frame(f) => f.style == FrameStyle::Circle,
        _ => false,
    }
}

/// Where a line from `e`'s centre towards `aim` leaves its outline, and
/// the exit direction. Elbow routes leave from the middle of a side.
fn edge_point(e: &Element, aim: Vec2, elbow: bool) -> (Vec2, Vec2) {
    let c = e.transform.center();
    if elbow {
        let b = e.bounds();
        let (hw, hh) = (b.width() / 2.0, b.height() / 2.0);
        let d = aim - c;
        return if d.x.abs() * hh >= d.y.abs() * hw {
            let s = if d.x >= 0.0 { 1.0 } else { -1.0 };
            (Vec2::new(c.x + s * hw, c.y), Vec2::new(s, 0.0))
        } else {
            let s = if d.y >= 0.0 { 1.0 } else { -1.0 };
            (Vec2::new(c.x, c.y + s * hh), Vec2::new(0.0, s))
        };
    }
    let l = e.transform.inverse(aim);
    let (hw, hh) = (e.size.w / 2.0, e.size.h / 2.0);
    let t = if round_outline(e) {
        let q = (l.x / hw.max(1e-12)).powi(2) + (l.y / hh.max(1e-12)).powi(2);
        if q > 0.0 { 1.0 / q.sqrt() } else { 1.0 }
    } else {
        let tx = if l.x.abs() > 1e-12 { hw / l.x.abs() } else { f64::INFINITY };
        let ty = if l.y.abs() > 1e-12 { hh / l.y.abs() } else { f64::INFINITY };
        tx.min(ty)
    };
    let local = if t.is_finite() { l * t.min(1.0) } else { Vec2::ZERO };
    let p = e.transform.apply(local);
    (p, unit(p - c).unwrap_or(Vec2::new(1.0, 0.0)))
}

/// Recompute every connector from the elements it is attached to. Ends
/// whose element is gone become free where they last were.
pub fn refresh(doc: &mut Document) {
    let ids: Vec<String> = doc.elements.iter().filter(|(_, e)| matches!(e.kind, ElementKind::Connector(_))).map(|(id, _)| id.clone()).collect();
    for id in ids {
        let e = &doc.elements[&id];
        let ElementKind::Connector(c) = &e.kind else { continue };
        let tf = e.transform;
        let target = |end: &crate::model::Endpoint| {
            end.element.as_ref().filter(|t| **t != id).and_then(|t| doc.elements.get(t)).filter(|t| !matches!(t.kind, ElementKind::Connector(_)))
        };
        let (ta, tb) = (target(&c.from), target(&c.to));
        let free = |end: &crate::model::Endpoint| tf.apply(Vec2::new(end.x, end.y));
        let (wa, wb) = (free(&c.from), free(&c.to));
        let aim_a = tb.map(|t| t.transform.center()).unwrap_or(wb);
        let aim_b = ta.map(|t| t.transform.center()).unwrap_or(wa);
        let elbow = c.route == Route::Elbow;
        let (pa, da) = match ta {
            Some(t) => edge_point(t, aim_a, elbow),
            None => (wa, Vec2::ZERO),
        };
        let (pb, db) = match tb {
            Some(t) => edge_point(t, aim_b, elbow),
            None => (wb, Vec2::ZERO),
        };
        let r = route(c.route, pa, da, pb, db);
        let mut pts = r.sample();
        if let RoutePath::Cubic(cs) = &r {
            pts.extend_from_slice(cs);
        }
        let b = Rect::from_points(pts).unwrap_or(Rect::from_center(pa, 0.0, 0.0));
        let center = b.center();
        let mut n = c.clone();
        n.from.element = ta.map(|t| t.id.clone());
        n.to.element = tb.map(|t| t.id.clone());
        (n.from.x, n.from.y, n.from.dir) = (pa.x - center.x, pa.y - center.y, da);
        (n.to.x, n.to.y, n.to.dir) = (pb.x - center.x, pb.y - center.y, db);
        let transform = Transform::at(center.x, center.y);
        let size = Size { w: b.width(), h: b.height() };
        let e = doc.elements.get_mut(&id).unwrap();
        if e.transform != transform || e.size != size || e.kind != ElementKind::Connector(n.clone()) {
            e.transform = transform;
            e.size = size;
            e.kind = ElementKind::Connector(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elbow_routes_are_right_angled() {
        let r = route(Route::Elbow, Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(100.0, 50.0), Vec2::new(-1.0, 0.0));
        let RoutePath::Poly(p) = r else { panic!() };
        assert_eq!(p.len(), 4);
        for w in p.windows(2) {
            assert!(w[0].x == w[1].x || w[0].y == w[1].y, "{p:?}");
        }
    }

    #[test]
    fn midpoint_and_tangent() {
        let r = RoutePath::Poly(vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)]);
        assert_eq!(r.midpoint(), Vec2::new(10.0, 0.0));
        assert_eq!(r.tangent(true), Vec2::new(0.0, 1.0));
        assert_eq!(r.tangent(false), Vec2::new(-1.0, 0.0));
    }
}
