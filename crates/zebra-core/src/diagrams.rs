//! Insert › Diagram: ready-made arrangements of shapes, arrows, frames and
//! placeholder text, in the theme's colours (Prezi Classic's diagrams,
//! the common SmartArt families and a few zooming-canvas patterns).
//!
//! Each diagram is built in unit coordinates like a layout (1 unit wide,
//! centred on (0, 0), about 0.62 tall); the editor scales it. Diagrams
//! without frames are inserted as one group; the zooming ones are frames,
//! which carry their contents, and add those frames to the path.
use std::f64::consts::PI;

use crate::model::{Align, Dash, FrameStyle, ShapeKind as K, TextStyle};
use crate::templates::Piece;

pub struct Diagram {
    /// Name in documents and the API.
    pub key: &'static str,
    /// Catalogue id (D1..D32).
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Frames for zooming: they go on the path.
    pub path: bool,
    pub build: fn() -> Vec<Piece>,
}

macro_rules! d {
    ($key:literal, $id:literal, $name:literal, $desc:literal, $path:literal, $f:ident) => {
        Diagram { key: $key, id: $id, name: $name, description: $desc, path: $path, build: $f }
    };
}

/// In the order the gallery shows them.
pub static ALL: &[Diagram] = &[
    d!("process", "D1", "Process", "Steps in a row joined by arrows.", false, process),
    d!("cycle", "D2", "Circular process", "Steps round a loop, the last feeding back to the first.", false, cycle),
    d!("timeline", "D3", "Timeline", "Dated points along one line, labels above and below.", false, timeline),
    d!("perspective", "D4", "Perspective", "Frames shrinking toward a vanishing point; the path zooms deeper each step.", true, perspective),
    d!("funnel", "D5", "Funnel", "Many inputs narrowing to one outcome.", false, funnel),
    d!("iceberg", "D7", "Iceberg", "A small tip above the waterline, the larger part below.", false, iceberg),
    d!("org_chart", "D9", "Org chart", "A top-down tree.", false, org_chart),
    d!("block_list", "D10", "Block list", "Side-by-side blocks, each with a heading.", false, block_list),
    d!("numbered_list", "D11", "Numbered list", "Rows with a numbered marker.", false, numbered_list),
    d!("chevrons", "D12", "Chevron process", "Interlocking arrows.", false, chevrons),
    d!("step_up", "D13", "Step-up", "Rising steps toward a goal.", false, step_up),
    d!("segmented_ring", "D14", "Segmented ring", "A ring in four parts around a centre idea.", false, segmented_ring),
    d!("tree", "D15", "Tree, left to right", "Branches out sideways, for breakdowns and decision trees.", false, tree),
    d!("venn2", "D16", "Venn, two circles", "Two overlapping sets.", false, venn2),
    d!("venn3", "D16", "Venn, three circles", "Three overlapping sets.", false, venn3),
    d!("target", "D17", "Target", "Rings from the outer context to the inner focus.", false, target),
    d!("inverted_pyramid", "D21", "Inverted pyramid", "Wide at the top, narrowing to one point.", false, inverted_pyramid),
    d!("mind_map", "D24", "Mind map", "Branches from a central topic.", false, mind_map),
    d!("fishbone", "D27", "Fishbone", "Causes branching into one effect.", false, fishbone),
    d!("columns", "D28", "Columns", "Cards sorted into to do, doing and done.", false, columns),
    d!("zoom_nest", "D30", "Zoom-in nest", "Frames inside frames; the path zooms in level by level.", true, zoom_nest),
    d!("orbit", "D31", "Orbit", "Frames round a centre; the path travels round them.", true, orbit),
    d!("big_picture", "D32", "Big picture with details", "An overview frame with detail frames inside it.", true, big_picture),
];

pub fn get(key: &str) -> Option<&'static Diagram> {
    ALL.iter().find(|d| d.key == key)
}

const FILL: [&str; 5] = ["theme:shape1", "theme:shape2", "theme:shape3", "theme:shape4", "theme:accent"];
const LINE: &str = "theme:frame";

fn fill(i: usize) -> &'static str {
    FILL[i % FILL.len()]
}

fn shape(k: K, x: f64, y: f64, w: f64, h: f64, fill: Option<&str>, stroke: Option<&str>) -> Piece {
    Piece::Shape { shape: k, x, y, w, h, rot: 0.0, fill: fill.map(String::from), stroke: stroke.map(String::from), sw: None, dash: Dash::Solid }
}

fn turned(mut p: Piece, deg: f64) -> Piece {
    if let Piece::Shape { rot, .. } | Piece::Frame { rot, .. } = &mut p {
        *rot = deg;
    }
    p
}

fn stroked(mut p: Piece, width: f64, d: Dash) -> Piece {
    if let Piece::Shape { sw, dash, .. } = &mut p {
        *sw = Some(width);
        *dash = d;
    }
    p
}

/// A straight line or arrow from (x1, y1) to (x2, y2).
fn line(x1: f64, y1: f64, x2: f64, y2: f64, arrow: bool) -> Piece {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let k = if arrow { K::Arrow } else { K::Line };
    let p = shape(k, (x1 + x2) / 2.0, (y1 + y2) / 2.0, dx.hypot(dy), if arrow { 0.03 } else { 0.01 }, None, Some(LINE));
    turned(p, dy.atan2(dx).to_degrees())
}

/// Placeholder text fitted into a w x h box centred on (x, y).
fn ph(style: TextStyle, text: &str, x: f64, y: f64, w: f64, h: f64, on: Option<&str>) -> Piece {
    Piece::Text { style, text: text.into(), x, y, w, h, bullets: false, align: Align::Center, ph: true, on: on.map(String::from) }
}

/// Text that shows when presenting (numbers, column headings).
fn real(style: TextStyle, text: &str, x: f64, y: f64, w: f64, h: f64, on: Option<&str>) -> Piece {
    Piece::Text { style, text: text.into(), x, y, w, h, bullets: false, align: Align::Center, ph: false, on: on.map(String::from) }
}

fn frame(style: FrameStyle, x: f64, y: f64, w: f64, h: f64) -> Piece {
    Piece::Frame { style, x, y, w, h, rot: 0.0, name: String::new(), path: true, color: None }
}

fn arc(cx: f64, cy: f64, r: f64, from_deg: f64, to_deg: f64, width: f64, color: &str) -> Piece {
    let n = (((to_deg - from_deg).abs() / 4.0).ceil() as usize).max(2);
    let points = (0..=n)
        .map(|i| {
            let a = (from_deg + (to_deg - from_deg) * i as f64 / n as f64) * PI / 180.0;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    Piece::Ink { points, width, color: color.into() }
}

/// A quadratic curve from a to b bending through c.
fn curve(a: (f64, f64), c: (f64, f64), b: (f64, f64), width: f64, color: &str) -> Piece {
    let points = (0..=16)
        .map(|i| {
            let t = i as f64 / 16.0;
            let u = 1.0 - t;
            [u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0, u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1]
        })
        .collect();
    Piece::Ink { points, width, color: color.into() }
}

use TextStyle::{Body, Subtitle, Title};

fn process() -> Vec<Piece> {
    let mut v = vec![];
    let (w, h, step) = (0.19, 0.13, 0.25);
    for i in 0..4 {
        let x = -0.375 + i as f64 * step;
        v.push(shape(K::RoundedRect, x, -0.03, w, h, Some(fill(i)), None));
        v.push(ph(Subtitle, "Step", x, -0.03, w * 0.75, 0.045, Some(fill(i))));
        v.push(ph(Body, "Add text", x, 0.1, w, 0.035, None));
        if i < 3 {
            v.push(line(x + w / 2.0 + 0.012, -0.03, x + step - w / 2.0 - 0.012, -0.03, true));
        }
    }
    v
}

fn cycle() -> Vec<Piece> {
    let mut v = vec![];
    let (n, r, d) = (5, 0.21, 0.13);
    let at = |i: usize| {
        let a = (-90.0 + 360.0 * i as f64 / n as f64) * PI / 180.0;
        (r * a.cos(), r * a.sin() + 0.01)
    };
    for i in 0..n {
        let (x, y) = at(i);
        let (x2, y2) = at((i + 1) % n);
        let (dx, dy) = (x2 - x, y2 - y);
        let len = dx.hypot(dy);
        let k = (d / 2.0 + 0.015) / len;
        v.push(line(x + dx * k, y + dy * k, x2 - dx * k, y2 - dy * k, true));
    }
    for i in 0..n {
        let (x, y) = at(i);
        v.push(shape(K::Ellipse, x, y, d, d, Some(fill(i)), None));
        v.push(ph(Body, "Step", x, y, d * 0.7, 0.035, Some(fill(i))));
    }
    v.push(ph(Subtitle, "Idea", 0.0, 0.01, 0.16, 0.05, None));
    v
}

fn timeline() -> Vec<Piece> {
    let mut v = vec![stroked(line(-0.48, 0.0, 0.48, 0.0, true), 0.006, Dash::Solid)];
    for i in 0..5 {
        let x = -0.36 + i as f64 * 0.18;
        let s = if i % 2 == 0 { -1.0 } else { 1.0 };
        v.push(line(x, 0.0, x, s * 0.07, false));
        v.push(shape(K::Ellipse, x, 0.0, 0.028, 0.028, Some("theme:accent"), None));
        v.push(ph(Subtitle, "Date", x, s * 0.105, 0.14, 0.035, None));
        v.push(ph(Body, "What happened", x, s * 0.155, 0.17, 0.03, None));
    }
    v
}

fn perspective() -> Vec<Piece> {
    // Frames shrink along a line toward a vanishing point at the right,
    // side by side so none hides another.
    // Each frame is the one before it scaled about the vanishing point,
    // so sizes, gaps and the guide lines all agree. Three quarters each
    // time: the fifth frame is still a third of the first (at 0.62 it was
    // a seventh, too small to work in).
    let (vx, vy) = (0.49, -0.2);
    let (ratio, gap) = (0.75, 0.05);
    let (left, y0) = (-0.49, 0.1);
    let w0 = (vx - left) * (1.0 - ratio) / (1.0 + gap);
    let h0 = w0 * 0.62;
    let mut v = vec![
        stroked(line(left, y0 - h0 / 2.0, vx, vy, false), 0.002, Dash::Dashed),
        stroked(line(left, y0 + h0 / 2.0, vx, vy, false), 0.002, Dash::Dashed),
    ];
    let mut k = 1.0;
    for _ in 0..5 {
        let w = w0 * k;
        let h = w * 0.62;
        let x = vx - (vx - left) * k + w / 2.0;
        let y = vy + (y0 - vy) * k;
        v.push(frame(FrameStyle::Rectangle, x, y, w, h));
        v.push(ph(Title, "Add a title", x, y - h * 0.2, w * 0.7, h * 0.16, None));
        v.push(ph(Body, "Add text", x, y + h * 0.12, w * 0.6, h * 0.1, None));
        k *= ratio;
    }
    v
}

fn funnel() -> Vec<Piece> {
    let mut v = vec![];
    for i in 0..4 {
        let x = -0.18 + i as f64 * 0.12;
        v.push(shape(K::Ellipse, x, -0.27, 0.07, 0.07, Some(fill(i)), None));
        v.push(line(x * 0.9, -0.225, x * 0.6, -0.185, true));
    }
    let (mut w, h, mut y) = (0.52, 0.1, -0.12);
    for i in 0..3 {
        let d = (w * 0.2_f64).min(h);
        v.push(turned(shape(K::Trapezoid, 0.0, y, w, h, Some(fill(i)), None), 180.0));
        v.push(ph(Body, "Add text", 0.0, y, (w - 2.0 * d) * 0.85, 0.035, Some(fill(i))));
        w -= 2.0 * d;
        y += h + 0.008;
    }
    v.push(line(0.0, y - h / 2.0 + 0.004, 0.0, y + 0.03, true));
    v.push(shape(K::RoundedRect, 0.0, y + 0.08, 0.22, 0.08, Some("theme:accent"), None));
    v.push(ph(Subtitle, "Outcome", 0.0, y + 0.08, 0.16, 0.04, Some("theme:accent")));
    v
}

fn iceberg() -> Vec<Piece> {
    vec![
        turned(shape(K::Triangle, 0.0, 0.1, 0.5, 0.36, Some("theme:shape1"), None), 180.0),
        shape(K::Triangle, 0.0, -0.14, 0.2, 0.12, Some("theme:frame_fill"), Some(LINE)),
        stroked(line(-0.48, -0.08, 0.48, -0.08, false), 0.005, Dash::Solid),
        ph(Subtitle, "What people see", -0.3, -0.15, 0.24, 0.04, None),
        ph(Subtitle, "What lies beneath", 0.0, 0.0, 0.26, 0.04, Some("theme:shape1")),
        ph(Body, "Add text", 0.34, 0.0, 0.2, 0.035, None),
        ph(Body, "Add text", 0.34, 0.08, 0.2, 0.035, None),
        ph(Body, "Add text", 0.34, 0.16, 0.2, 0.035, None),
    ]
}

fn org_chart() -> Vec<Piece> {
    let mut v = vec![];
    let mid = [-0.3, 0.0, 0.3];
    v.push(line(0.0, -0.155, 0.0, -0.095, false));
    v.push(line(-0.3, -0.095, 0.3, -0.095, false));
    for &x in &mid {
        v.push(line(x, -0.095, x, -0.06, false));
        v.push(line(x, 0.02, x, 0.07, false));
        v.push(line(x - 0.075, 0.07, x + 0.075, 0.07, false));
        for dx in [-0.075, 0.075] {
            v.push(line(x + dx, 0.07, x + dx, 0.125, false));
            v.push(shape(K::RoundedRect, x + dx, 0.16, 0.13, 0.07, Some(fill(2)), None));
            v.push(ph(Body, "Name", x + dx, 0.16, 0.1, 0.03, Some(fill(2))));
        }
        v.push(shape(K::RoundedRect, x, -0.02, 0.17, 0.08, Some(fill(1)), None));
        v.push(ph(Body, "Name", x, -0.02, 0.13, 0.034, Some(fill(1))));
    }
    v.push(shape(K::RoundedRect, 0.0, -0.2, 0.19, 0.09, Some(fill(0)), None));
    v.push(ph(Subtitle, "Name", 0.0, -0.2, 0.14, 0.04, Some(fill(0))));
    v
}

fn block_list() -> Vec<Piece> {
    let mut v = vec![];
    for i in 0..4 {
        let x = -0.36 + i as f64 * 0.24;
        v.push(shape(K::RoundedRect, x, 0.0, 0.22, 0.32, Some(fill(i)), None));
        v.push(ph(Subtitle, "Heading", x, -0.1, 0.17, 0.045, Some(fill(i))));
        v.push(ph(Body, "Add your points here\nOne idea per line", x, 0.03, 0.18, 0.08, Some(fill(i))));
    }
    v
}

fn numbered_list() -> Vec<Piece> {
    let mut v = vec![];
    for i in 0..5 {
        let y = -0.22 + i as f64 * 0.11;
        v.push(shape(K::RoundedRect, 0.07, y, 0.62, 0.08, Some("theme:frame_fill"), None));
        v.push(shape(K::Ellipse, -0.27, y, 0.09, 0.09, Some(fill(i)), None));
        v.push(real(Subtitle, &(i + 1).to_string(), -0.27, y, 0.04, 0.045, Some(fill(i))));
        v.push(ph(Body, "Add text", 0.08, y, 0.5, 0.034, Some("theme:frame_fill")));
    }
    v
}

fn chevrons() -> Vec<Piece> {
    let mut v = vec![];
    let (w, h) = (0.22, 0.13);
    let d = (w * 0.3_f64).min(h / 2.0);
    let step = w - d + 0.012;
    let x0 = -(step * 4.0) / 2.0;
    for i in 0..5 {
        let x = x0 + i as f64 * step;
        v.push(shape(K::Chevron, x, -0.03, w, h, Some(fill(i)), None));
        v.push(ph(Body, "Step", x + d * 0.3, -0.03, (w - 2.0 * d) * 0.9, 0.035, Some(fill(i))));
        v.push(ph(Body, "Add text", x, 0.08, step * 0.95, 0.03, None));
    }
    v
}

fn step_up() -> Vec<Piece> {
    let mut v = vec![];
    let base = 0.22;
    for i in 0..4 {
        let x = -0.33 + i as f64 * 0.22;
        let h = 0.1 + i as f64 * 0.09;
        v.push(shape(K::Rectangle, x, base - h / 2.0, 0.2, h, Some(fill(i)), None));
        v.push(ph(Subtitle, "Step", x, base - h + 0.035, 0.14, 0.04, Some(fill(i))));
        v.push(ph(Body, "Add text", x, base - h - 0.04, 0.19, 0.03, None));
    }
    v
}

fn segmented_ring() -> Vec<Piece> {
    let mut v = vec![];
    let r = 0.17;
    for i in 0..4 {
        let a0 = -90.0 + i as f64 * 90.0;
        // Round line ends reach about 12 degrees past each end.
        v.push(arc(0.0, 0.0, r, a0 + 14.0, a0 + 76.0, 0.07, fill(i)));
        let m = (a0 + 45.0) * PI / 180.0;
        v.push(ph(Body, "Add text", 0.3 * m.cos() * 1.25, 0.27 * m.sin(), 0.16, 0.032, None));
    }
    v.push(ph(Subtitle, "Idea", 0.0, 0.0, 0.14, 0.045, None));
    v
}

fn tree() -> Vec<Piece> {
    let mut v = vec![];
    let kids = [-0.18, 0.0, 0.18];
    v.push(line(-0.28, 0.0, -0.2, 0.0, false));
    v.push(line(-0.2, -0.18, -0.2, 0.18, false));
    for &y in &kids {
        v.push(line(-0.2, y, -0.135, y, false));
        v.push(line(0.015, y, 0.1, y, false));
        v.push(line(0.1, y - 0.045, 0.1, y + 0.045, false));
        for dy in [-0.045, 0.045] {
            v.push(line(0.1, y + dy, 0.185, y + dy, false));
            v.push(shape(K::RoundedRect, 0.28, y + dy, 0.19, 0.07, Some(fill(2)), None));
            v.push(ph(Body, "Add text", 0.28, y + dy, 0.15, 0.03, Some(fill(2))));
        }
        v.push(shape(K::RoundedRect, -0.06, y, 0.15, 0.08, Some(fill(1)), None));
        v.push(ph(Body, "Idea", -0.06, y, 0.11, 0.034, Some(fill(1))));
    }
    v.push(shape(K::RoundedRect, -0.36, 0.0, 0.16, 0.1, Some(fill(0)), None));
    v.push(ph(Subtitle, "Topic", -0.36, 0.0, 0.12, 0.04, Some(fill(0))));
    v
}

fn venn_circle(x: f64, y: f64, r: f64, i: usize) -> Piece {
    stroked(shape(K::Ellipse, x, y, 2.0 * r, 2.0 * r, None, Some(fill(i))), 0.007, Dash::Solid)
}

fn venn2() -> Vec<Piece> {
    vec![
        venn_circle(-0.11, 0.0, 0.21, 0),
        venn_circle(0.11, 0.0, 0.21, 1),
        ph(Subtitle, "Idea", -0.2, 0.0, 0.13, 0.04, None),
        ph(Subtitle, "Idea", 0.2, 0.0, 0.13, 0.04, None),
        ph(Body, "Both", 0.0, 0.0, 0.08, 0.03, None),
    ]
}

fn venn3() -> Vec<Piece> {
    let r = 0.17;
    let c = [(0.0, -0.085), (-0.1, 0.085), (0.1, 0.085)];
    let mut v: Vec<Piece> = c.iter().enumerate().map(|(i, &(x, y))| venn_circle(x, y, r, i)).collect();
    v.push(ph(Subtitle, "Idea", 0.0, -0.17, 0.12, 0.04, None));
    v.push(ph(Subtitle, "Idea", -0.17, 0.13, 0.12, 0.04, None));
    v.push(ph(Subtitle, "Idea", 0.17, 0.13, 0.12, 0.04, None));
    v.push(ph(Body, "All", 0.0, 0.03, 0.05, 0.028, None));
    v
}

fn target() -> Vec<Piece> {
    let radii = [0.29, 0.22, 0.15, 0.08];
    let mut v = vec![];
    for (i, &r) in radii.iter().enumerate() {
        v.push(shape(K::Ellipse, 0.0, 0.0, 2.0 * r, 2.0 * r, Some(fill(3 - i)), None));
    }
    for i in 0..3 {
        let y = -(radii[i] + radii[i + 1]) / 2.0;
        v.push(ph(Body, "Add text", 0.0, y, 0.16, 0.03, Some(fill(3 - i))));
    }
    v.push(ph(Subtitle, "Focus", 0.0, 0.0, 0.1, 0.04, Some(fill(0))));
    v
}

fn inverted_pyramid() -> Vec<Piece> {
    let mut v = vec![];
    let (mut w, h) = (0.62, 0.11);
    let mut y = -0.21;
    for i in 0..3 {
        let d = (w * 0.2_f64).min(h);
        v.push(turned(shape(K::Trapezoid, 0.0, y, w, h, Some(fill(i)), None), 180.0));
        v.push(ph(Body, "Add text", 0.0, y, (w - 2.0 * d) * 0.8, 0.035, Some(fill(i))));
        w -= 2.0 * d;
        y += h + 0.008;
    }
    let th = 0.15;
    v.push(turned(shape(K::Triangle, 0.0, y + (th - h) / 2.0, w, th, Some(fill(3)), None), 180.0));
    v.push(ph(Body, "Point", 0.0, y - 0.01, w * 0.45, 0.025, Some(fill(3))));
    v
}

fn mind_map() -> Vec<Piece> {
    let mut v = vec![];
    let nodes: [(f64, f64); 6] = [(0.33, -0.2), (0.38, 0.0), (0.33, 0.2), (-0.33, -0.2), (-0.38, 0.0), (-0.33, 0.2)];
    for (i, &(x, y)) in nodes.iter().enumerate() {
        let sx = x.signum();
        let start = (sx * 0.09, y * 0.15);
        let end = (x - sx * 0.08, y);
        let ctrl = (sx * 0.2, y * 0.95);
        v.push(curve(start, ctrl, end, 0.008, fill(i)));
    }
    for (i, &(x, y)) in nodes.iter().enumerate() {
        v.push(shape(K::Pill, x, y, 0.17, 0.065, Some(fill(i)), None));
        v.push(ph(Body, "Idea", x, y, 0.12, 0.032, Some(fill(i))));
    }
    v.push(shape(K::Ellipse, 0.0, 0.0, 0.22, 0.13, Some("theme:accent"), None));
    v.push(ph(Subtitle, "Topic", 0.0, 0.0, 0.15, 0.045, Some("theme:accent")));
    v
}

fn fishbone() -> Vec<Piece> {
    let mut v = vec![stroked(line(-0.44, 0.0, 0.25, 0.0, true), 0.007, Dash::Solid)];
    for x in [-0.3, -0.1, 0.1] {
        for s in [-1.0, 1.0] {
            v.push(stroked(line(x - 0.09, s * 0.2, x + 0.03, 0.0, false), 0.004, Dash::Solid));
            v.push(ph(Subtitle, "Cause", x - 0.09, s * 0.235, 0.12, 0.035, None));
            v.push(ph(Body, "Add text", x - 0.11, s * 0.1, 0.1, 0.026, None));
        }
    }
    v.push(shape(K::RoundedRect, 0.36, 0.0, 0.2, 0.12, Some("theme:accent"), None));
    v.push(ph(Subtitle, "Effect", 0.36, 0.0, 0.14, 0.045, Some("theme:accent")));
    v
}

fn columns() -> Vec<Piece> {
    let mut v = vec![];
    let heads = ["To do", "Doing", "Done"];
    for (c, head) in heads.iter().enumerate() {
        let x = -0.32 + c as f64 * 0.32;
        v.push(shape(K::RoundedRect, x, 0.02, 0.3, 0.52, Some("theme:frame_fill"), None));
        v.push(real(Subtitle, head, x, -0.2, 0.14, 0.045, Some("theme:frame_fill")));
        for k in 0..(3 - c) {
            let y = -0.11 + k as f64 * 0.11;
            v.push(shape(K::RoundedRect, x, y, 0.25, 0.085, Some(fill(c)), None));
            v.push(ph(Body, "Add text", x, y, 0.19, 0.032, Some(fill(c))));
        }
    }
    v
}

fn zoom_nest() -> Vec<Piece> {
    let mut v = vec![];
    let (mut x, mut y, mut w) = (0.0, 0.0, 1.0);
    for _ in 0..4 {
        let h = w * 0.62;
        v.push(frame(FrameStyle::Rectangle, x, y, w, h));
        v.push(ph(Title, "Add a title", x - 0.22 * w, y - 0.2 * w, 0.42 * w, 0.07 * w, None));
        v.push(ph(Body, "Add text", x - 0.22 * w, y - 0.08 * w, 0.36 * w, 0.045 * w, None));
        x += 0.22 * w;
        y += 0.15 * w;
        w *= 0.4;
    }
    v
}

fn orbit() -> Vec<Piece> {
    let r = 0.25;
    let mut v = vec![stroked(shape(K::Ellipse, 0.0, 0.0, 2.0 * r, 2.0 * r, None, Some(LINE)), 0.003, Dash::Dashed)];
    v.push(frame(FrameStyle::Circle, 0.0, 0.0, 0.22, 0.22));
    v.push(ph(Subtitle, "Topic", 0.0, 0.0, 0.15, 0.045, None));
    for i in 0..6 {
        let a = (-90.0 + i as f64 * 60.0) * PI / 180.0;
        let (x, y) = (r * a.cos() * 1.3, r * a.sin());
        v.push(frame(FrameStyle::Circle, x, y, 0.14, 0.14));
        v.push(ph(Body, "Idea", x, y, 0.1, 0.032, None));
    }
    v
}

fn big_picture() -> Vec<Piece> {
    let mut v = vec![frame(FrameStyle::Bracket, 0.0, 0.0, 1.0, 0.62)];
    v.push(ph(Title, "Add a title", 0.0, -0.2, 0.6, 0.08, None));
    for i in 0..4 {
        let x = -0.33 + i as f64 * 0.22;
        let y = 0.06 + if i % 2 == 0 { -0.03 } else { 0.05 };
        v.push(frame(FrameStyle::Rectangle, x, y, 0.19, 0.12));
        v.push(ph(Body, "Detail to zoom into", x, y, 0.15, 0.03, None));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_diagram_builds_inside_its_box() {
        for d in ALL {
            let pieces = (d.build)();
            assert!(!pieces.is_empty(), "{}", d.key);
            for p in &pieces {
                let (x, y) = match p {
                    Piece::Frame { x, y, .. } | Piece::Text { x, y, .. } | Piece::Shape { x, y, .. } => (*x, *y),
                    Piece::Ink { points, .. } => (points[0][0], points[0][1]),
                };
                assert!(x.abs() <= 0.5 && y.abs() <= 0.35, "{} has a piece at {x}, {y}", d.key);
            }
            let frames = pieces.iter().filter(|p| matches!(p, Piece::Frame { .. })).count();
            assert_eq!(frames > 0, d.path, "{}: only zooming diagrams use frames", d.key);
        }
    }
}
