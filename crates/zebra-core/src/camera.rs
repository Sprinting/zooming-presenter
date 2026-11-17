//! Camera math and the Prezi "fly" transition (van Wijk & Nuij 2003).
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geom::{Rect, Vec2};
use crate::model::ViewRect;

pub const MIN_ZOOM: f64 = 1e-6;
pub const MAX_ZOOM: f64 = 1e6;

/// A camera looking at the world. `zoom` is screen pixels (logical points) per
/// world unit. `rotation` is the view roll in degrees: a frame rotated by `r`
/// is shown upright by a camera with rotation `r`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Camera {
    pub cx: f64,
    pub cy: f64,
    pub zoom: f64,
    #[serde(default)]
    pub rotation: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 }
    }
}

impl Camera {
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.cx, self.cy)
    }
    /// World -> screen (screen origin top-left, `viewport` in logical points).
    pub fn world_to_screen(&self, p: Vec2, viewport: Vec2) -> Vec2 {
        (p - self.center()).rotate_deg(-self.rotation) * self.zoom + viewport * 0.5
    }
    pub fn screen_to_world(&self, s: Vec2, viewport: Vec2) -> Vec2 {
        ((s - viewport * 0.5) * (1.0 / self.zoom)).rotate_deg(self.rotation) + self.center()
    }
    /// Zoom by `factor` keeping the world point under `anchor` (screen) fixed.
    pub fn zoom_at(&mut self, factor: f64, anchor: Vec2, viewport: Vec2) {
        let before = self.screen_to_world(anchor, viewport);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.screen_to_world(anchor, viewport);
        self.cx += before.x - after.x;
        self.cy += before.y - after.y;
    }
    /// Pan by a screen-space delta.
    pub fn pan_screen(&mut self, d: Vec2) {
        let w = d.rotate_deg(self.rotation) * (1.0 / self.zoom);
        self.cx -= w.x;
        self.cy -= w.y;
    }
    /// Camera that fits a rotated world rectangle into the viewport with a
    /// margin fraction (0.05 = 5% each side).
    pub fn fit(view: &ViewRect, viewport: Vec2, margin: f64) -> Camera {
        let vw = viewport.x * (1.0 - 2.0 * margin);
        let vh = viewport.y * (1.0 - 2.0 * margin);
        let zoom = (vw / view.width.max(1e-9)).min(vh / view.height.max(1e-9));
        Camera { cx: view.cx, cy: view.cy, zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM), rotation: view.rotation }
    }
    pub fn fit_rect(r: &Rect, viewport: Vec2, margin: f64) -> Camera {
        let c = r.center();
        Self::fit(
            &ViewRect { cx: c.x, cy: c.y, width: r.width(), height: r.height(), rotation: 0.0 },
            viewport,
            margin,
        )
    }
    /// The world rectangle the camera shows.
    pub fn view_rect(&self, viewport: Vec2) -> ViewRect {
        ViewRect {
            cx: self.cx,
            cy: self.cy,
            width: viewport.x / self.zoom,
            height: viewport.y / self.zoom,
            rotation: self.rotation,
        }
    }
    /// Axis-aligned world bounds of what is visible (for culling).
    pub fn visible_bounds(&self, viewport: Vec2) -> Rect {
        let pts = [Vec2::ZERO, Vec2::new(viewport.x, 0.0), viewport, Vec2::new(0.0, viewport.y)]
            .map(|s| self.screen_to_world(s, viewport));
        Rect::from_points(pts).unwrap()
    }
}

/// An in-flight camera animation. Time is injected (seconds), so the API and
/// tests can step it deterministically.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraFlight {
    pub from: Camera,
    pub to: Camera,
    pub start: f64,
    pub duration: f64,
    /// Zoom out mid-flight when far apart (Prezi feel). False = straight tween.
    pub arc: bool,
}

const RHO: f64 = 1.3;

impl CameraFlight {
    /// Duration grows with the square root of the path "cost" (in screens),
    /// so short hops stay snappy and very far jumps don't drag: ~0.5 s for a
    /// tiny nudge, ~1.1 s for one screen, capped at 2.4 s. Degenerate input
    /// (NaN/zero zoom, non-finite centres) falls back to a safe default.
    pub fn new(from: Camera, to: Camera, start: f64, viewport: Vec2, reduce_motion: bool) -> Self {
        let s = path_length(&from, &to, viewport);
        let duration = if reduce_motion {
            0.35
        } else if s.is_finite() {
            (0.5 + 0.6 * s.max(0.0).sqrt()).clamp(0.5, 2.4)
        } else {
            1.2
        };
        CameraFlight { from, to, start, duration, arc: !reduce_motion }
    }
    pub fn with_duration(mut self, d: f64) -> Self {
        self.duration = if d.is_finite() { d.max(0.0) } else { 0.0 };
        self
    }
    pub fn done(&self, now: f64) -> bool {
        now >= self.start + self.duration
    }
    pub fn progress(&self, now: f64) -> f64 {
        if self.duration <= 0.0 { 1.0 } else { ((now - self.start) / self.duration).clamp(0.0, 1.0) }
    }
    /// Camera at time `now`.
    pub fn sample(&self, now: f64, viewport: Vec2) -> Camera {
        let t = ease_in_out(self.progress(now));
        if t >= 1.0 {
            return Camera { zoom: sane_zoom(self.to.zoom), ..self.to };
        }
        if t <= 0.0 {
            return Camera { zoom: sane_zoom(self.from.zoom), ..self.from };
        }
        let rot = lerp_angle(self.from.rotation, self.to.rotation, t);
        let z0 = sane_zoom(self.from.zoom);
        let z1 = sane_zoom(self.to.zoom);
        let straight = || Camera {
            cx: lerp(self.from.cx, self.to.cx, t),
            cy: lerp(self.from.cy, self.to.cy, t),
            zoom: (z0.ln() + (z1.ln() - z0.ln()) * t).exp().clamp(MIN_ZOOM, MAX_ZOOM),
            rotation: rot,
        };
        let vpw = if viewport.x.is_finite() && viewport.x > 0.0 { viewport.x } else { 1.0 };
        if !self.arc {
            return straight();
        }
        // van Wijk & Nuij: interpolate (centre, width) along the optimal path.
        let w0 = vpw / z0;
        let w1 = vpw / z1;
        let c0 = self.from.center();
        let c1 = self.to.center();
        let u1 = (c1 - c0).len();
        let (centre, w) = if u1 < 1e-9 * w0.max(w1) {
            // Pure zoom.
            let w = (w0.ln() + (w1.ln() - w0.ln()) * t).exp();
            (c0, w)
        } else {
            let r = |i: f64| {
                let wi = if i == 0.0 { w0 } else { w1 };
                let b = (w1 * w1 - w0 * w0 + (if i == 0.0 { 1.0 } else { -1.0 }) * RHO.powi(4) * u1 * u1)
                    / (2.0 * wi * RHO * RHO * u1);
                ((b * b + 1.0).sqrt() - b).ln()
            };
            let r0 = r(0.0);
            let r1 = r(1.0);
            let big_s = (r1 - r0) / RHO;
            let s = t * big_s;
            let u = w0 / (RHO * RHO) * (r0.cosh() * (RHO * s + r0).tanh() - r0.sinh());
            let w = w0 * r0.cosh() / (RHO * s + r0).cosh();
            let dir = (c1 - c0) * (1.0 / u1);
            (c0 + dir * u, w)
        };
        if !(centre.x.is_finite() && centre.y.is_finite() && w.is_finite() && w > 0.0) {
            // Numerical blow-up (extreme zoom ratios): degrade to a log tween.
            return straight();
        }
        Camera { cx: centre.x, cy: centre.y, zoom: (vpw / w).clamp(MIN_ZOOM, MAX_ZOOM), rotation: rot }
    }
}

/// Length of the optimal zoom/pan path, in units of "screens".
pub fn path_length(from: &Camera, to: &Camera, viewport: Vec2) -> f64 {
    let vpw = if viewport.x.is_finite() && viewport.x > 0.0 { viewport.x } else { 1.0 };
    let w0 = vpw / sane_zoom(from.zoom);
    let w1 = vpw / sane_zoom(to.zoom);
    let u1 = (to.center() - from.center()).len();
    if u1 < 1e-9 * w0.max(w1) {
        return (w1 / w0).ln().abs() / RHO;
    }
    let b = |w: f64, sign: f64| (w1 * w1 - w0 * w0 + sign * RHO.powi(4) * u1 * u1) / (2.0 * w * RHO * RHO * u1);
    let r = |b: f64| ((b * b + 1.0).sqrt() - b).ln();
    ((r(b(w1, -1.0)) - r(b(w0, 1.0))) / RHO).abs()
}

/// Quintic "smootherstep": zero velocity *and* zero acceleration at both ends,
/// so the camera eases out of rest and settles without a visible jolt.
pub fn ease_in_out(t: f64) -> f64 {
    if t.is_nan() {
        return 1.0;
    }
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (6.0 * t - 15.0) + 10.0)
}

/// Clamp a zoom into the valid range, mapping NaN/inf/<=0 to something usable.
fn sane_zoom(z: f64) -> f64 {
    if z.is_finite() && z > 0.0 { z.clamp(MIN_ZOOM, MAX_ZOOM) } else if z == f64::INFINITY { MAX_ZOOM } else { 1.0 }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Shortest-arc angle interpolation in degrees.
pub fn lerp_angle(a: f64, b: f64, t: f64) -> f64 {
    let mut d = (b - a) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    a + d * t
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_screen_world() {
        let c = Camera { cx: 10.0, cy: -5.0, zoom: 3.0, rotation: 30.0 };
        let vp = Vec2::new(800.0, 600.0);
        let w = Vec2::new(12.0, 7.0);
        let s = c.world_to_screen(w, vp);
        let back = c.screen_to_world(s, vp);
        assert!((back - w).len() < 1e-9);
    }
    #[test]
    fn flight_endpoints() {
        let vp = Vec2::new(800.0, 600.0);
        let a = Camera { cx: 0.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
        let b = Camera { cx: 5000.0, cy: 100.0, zoom: 8.0, rotation: 90.0 };
        let f = CameraFlight::new(a, b, 0.0, vp, false);
        let s0 = f.sample(0.0, vp);
        assert!((s0.cx - a.cx).abs() < 1e-6 && (s0.zoom - a.zoom).abs() < 1e-6);
        assert_eq!(f.sample(f.duration + 0.01, vp), b);
        // zooms out mid-flight
        let mid = f.sample(f.duration / 2.0, vp);
        assert!(mid.zoom < a.zoom);
    }
    #[test]
    fn easing_is_monotonic_and_symmetric() {
        assert_eq!(ease_in_out(0.0), 0.0);
        assert_eq!(ease_in_out(1.0), 1.0);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-12);
        assert_eq!(ease_in_out(-3.0), 0.0);
        assert_eq!(ease_in_out(7.0), 1.0);
        let mut prev = 0.0;
        for i in 1..=100 {
            let t = i as f64 / 100.0;
            let e = ease_in_out(t);
            assert!(e >= prev);
            assert!((e + ease_in_out(1.0 - t) - 1.0).abs() < 1e-12);
            prev = e;
        }
    }
    #[test]
    fn duration_scales_and_clamps() {
        let vp = Vec2::new(800.0, 600.0);
        let a = Camera::default();
        let near = CameraFlight::new(a, Camera { cx: 100.0, ..a }, 0.0, vp, false);
        let far = CameraFlight::new(a, Camera { cx: 1e7, ..a }, 0.0, vp, false);
        let same = CameraFlight::new(a, a, 0.0, vp, false);
        assert!(same.duration >= 0.5 && same.duration <= near.duration);
        assert!(near.duration < far.duration);
        assert!(far.duration <= 2.4);
        assert_eq!(CameraFlight::new(a, far.to, 0.0, vp, true).duration, 0.35);
    }
    #[test]
    fn degenerate_cameras_stay_finite() {
        let vp = Vec2::new(800.0, 600.0);
        let a = Camera { zoom: 0.0, ..Camera::default() };
        let b = Camera { cx: 50.0, cy: 20.0, zoom: f64::NAN, rotation: 0.0 };
        let f = CameraFlight::new(a, b, 0.0, vp, false);
        assert!(f.duration.is_finite());
        for i in 0..=10 {
            let c = f.sample(f.duration * i as f64 / 10.0, vp);
            assert!(c.cx.is_finite() && c.cy.is_finite() && c.zoom.is_finite() && c.zoom > 0.0);
        }
        let z = CameraFlight::new(Camera::default(), Camera::default(), 0.0, vp, false).with_duration(f64::NAN);
        assert_eq!(z.duration, 0.0);
        assert_eq!(z.sample(0.0, vp), Camera::default());
    }
}
