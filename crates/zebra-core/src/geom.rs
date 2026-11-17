//! Small f64 geometry kit. World coordinates are f64; renderers rebase to f32.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn len(self) -> f64 {
        self.x.hypot(self.y)
    }
    pub fn rotate_deg(self, deg: f64) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Vec2::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }
}
impl std::ops::Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}
impl std::ops::Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl std::ops::Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f64) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}

/// Axis-aligned rectangle in world units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn from_center(c: Vec2, w: f64, h: f64) -> Self {
        Rect { x0: c.x - w / 2.0, y0: c.y - h / 2.0, x1: c.x + w / 2.0, y1: c.y + h / 2.0 }
    }
    pub fn from_points(pts: impl IntoIterator<Item = Vec2>) -> Option<Self> {
        let mut it = pts.into_iter();
        let f = it.next()?;
        let mut r = Rect { x0: f.x, y0: f.y, x1: f.x, y1: f.y };
        for p in it {
            r.x0 = r.x0.min(p.x);
            r.y0 = r.y0.min(p.y);
            r.x1 = r.x1.max(p.x);
            r.y1 = r.y1.max(p.y);
        }
        Some(r)
    }
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }
    pub fn center(&self) -> Vec2 {
        Vec2::new((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }
    pub fn area(&self) -> f64 {
        self.width().max(0.0) * self.height().max(0.0)
    }
    pub fn union(&self, o: &Rect) -> Rect {
        Rect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x0 <= o.x1 && o.x0 <= self.x1 && self.y0 <= o.y1 && o.y0 <= self.y1
    }
    pub fn contains_rect(&self, o: &Rect) -> bool {
        o.x0 >= self.x0 && o.x1 <= self.x1 && o.y0 >= self.y0 && o.y1 <= self.y1
    }
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }
    pub fn inflate(&self, d: f64) -> Rect {
        Rect { x0: self.x0 - d, y0: self.y0 - d, x1: self.x1 + d, y1: self.y1 + d }
    }
}

/// Placement of an element: centre position, uniform scale, rotation in degrees
/// (clockwise, since y points down). No skew, exactly like classic Prezi.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Transform {
    pub x: f64,
    pub y: f64,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub rotation: f64,
}

fn one() -> f64 {
    1.0
}

impl Default for Transform {
    fn default() -> Self {
        Transform { x: 0.0, y: 0.0, scale: 1.0, rotation: 0.0 }
    }
}

impl Transform {
    pub fn at(x: f64, y: f64) -> Self {
        Transform { x, y, ..Default::default() }
    }
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }
    /// Local (centre-origin, unscaled) point to world.
    pub fn apply(&self, p: Vec2) -> Vec2 {
        (p * self.scale).rotate_deg(self.rotation) + self.center()
    }
    /// World point to local.
    pub fn inverse(&self, p: Vec2) -> Vec2 {
        (p - self.center()).rotate_deg(-self.rotation) * (1.0 / self.scale)
    }
}

/// Width and height of the axis-aligned box around a `w`×`h` box turned by
/// `rotation` degrees.
pub fn rotated_extent(w: f64, h: f64, rotation: f64) -> (f64, f64) {
    let (s, c) = rotation.to_radians().sin_cos();
    (w * c.abs() + h * s.abs(), w * s.abs() + h * c.abs())
}
