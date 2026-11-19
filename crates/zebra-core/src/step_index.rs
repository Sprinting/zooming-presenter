//! Which path step is nearest a camera, without measuring every step.
//!
//! The path sidebar and Play from Current ask for the step whose view is
//! closest to where the person is looking, and they ask again on every
//! camera change while panning. Measuring every step each time was linear
//! in the path (3.6 ms at 4,900 steps). This index holds each step's camera
//! as a point (centre x, centre y, ln zoom) in a k-d tree, built once per
//! document revision and viewport, and answers with a branch-and-bound
//! search.
//!
//! "Nearest" is the measure the sidebar always used:
//! `|centre - camera centre| * zoom / viewport width + |ln(step zoom / zoom)|`.
//! Its weight on the centre distance depends on the camera's zoom, so the
//! tree can't be pre-scaled; instead each box is bounded below by the same
//! measure taken to its closest point, which is exact for pruning.
//! Answers equal the brute-force ones (ties go to the lower step number).
use crate::camera::Camera;
use crate::editor::Editor;
use crate::model::StepTarget;

/// Points per leaf: small enough to prune well, big enough that the tree
/// is a few hundred nodes at most for a long path.
const LEAF: usize = 8;

#[derive(Clone, Copy, Debug)]
struct Point {
    step: usize,
    p: [f64; 3],
}

#[derive(Debug)]
struct Node {
    lo: [f64; 3],
    hi: [f64; 3],
    /// Leaf: a range of `points`. Inner: two children.
    kind: NodeKind,
}

#[derive(Debug)]
enum NodeKind {
    Leaf(usize, usize),
    Inner(usize, usize),
}

/// Step cameras of one document revision and viewport.
#[derive(Debug, Default)]
pub struct StepIndex {
    points: Vec<Point>,
    nodes: Vec<Node>,
    /// Steps whose camera isn't finite: always measured.
    odd: Vec<Point>,
}

/// The measure between a camera and a step's point.
fn measure(cam: &[f64; 3], w: f64, p: &[f64; 3]) -> f64 {
    let (dx, dy) = (p[0] - cam[0], p[1] - cam[1]);
    (dx * dx + dy * dy).sqrt() * w + (p[2] - cam[2]).abs()
}

/// The least the measure can be to any point in the box.
fn bound(cam: &[f64; 3], w: f64, lo: &[f64; 3], hi: &[f64; 3]) -> f64 {
    let d = |k: usize| (lo[k] - cam[k]).max(0.0).max(cam[k] - hi[k]);
    let (dx, dy) = (d(0), d(1));
    (dx * dx + dy * dy).sqrt() * w + d(2)
}

fn point_of(step: usize, c: &Camera) -> Point {
    Point { step, p: [c.cx, c.cy, c.zoom.ln()] }
}

impl StepIndex {
    /// Each step's camera once (the whole-deck framing at most once).
    pub fn build(ed: &Editor) -> StepIndex {
        let all = std::cell::OnceCell::new();
        let fit_all = || *all.get_or_init(|| ed.fit_all_camera());
        let cams = ed.doc.path.iter().map(|st| match &st.target {
            StepTarget::Overview => fit_all(),
            StepTarget::Element { id } if ed.doc.get(id).is_none() => fit_all(),
            t => ed.camera_for_target(t),
        });
        Self::from_cameras(cams)
    }

    /// From step cameras in path order.
    pub fn from_cameras(cams: impl IntoIterator<Item = Camera>) -> StepIndex {
        let (mut points, mut odd) = (vec![], vec![]);
        for (i, c) in cams.into_iter().enumerate() {
            let pt = point_of(i, &c);
            if pt.p.iter().all(|v| v.is_finite()) {
                points.push(pt);
            } else {
                odd.push(pt);
            }
        }
        let mut ix = StepIndex { points, nodes: vec![], odd };
        if !ix.points.is_empty() {
            let n = ix.points.len();
            ix.split(0, n);
        }
        ix
    }

    pub fn len(&self) -> usize {
        self.points.len() + self.odd.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Builds the subtree over `points[a..b]`; returns its node.
    fn split(&mut self, a: usize, b: usize) -> usize {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for pt in &self.points[a..b] {
            for k in 0..3 {
                lo[k] = lo[k].min(pt.p[k]);
                hi[k] = hi[k].max(pt.p[k]);
            }
        }
        let id = self.nodes.len();
        self.nodes.push(Node { lo, hi, kind: NodeKind::Leaf(a, b) });
        if b - a <= LEAF {
            return id;
        }
        // Split the widest side at the median.
        let axis = (0..3).max_by(|&i, &j| (hi[i] - lo[i]).total_cmp(&(hi[j] - lo[j]))).unwrap_or(0);
        let mid = a + (b - a) / 2;
        self.points[a..b].select_nth_unstable_by(mid - a, |x, y| x.p[axis].total_cmp(&y.p[axis]));
        let l = self.split(a, mid);
        let r = self.split(mid, b);
        self.nodes[id].kind = NodeKind::Inner(l, r);
        id
    }

    /// The step nearest `cam` seen in a viewport `viewport_w` wide.
    pub fn nearest(&self, cam: &Camera, viewport_w: f64) -> Option<usize> {
        let q = [cam.cx, cam.cy, cam.zoom.ln()];
        let w = cam.zoom / viewport_w;
        // (measure, step): the lower step wins a tie, as a scan in order would.
        let mut best: Option<(f64, usize)> = None;
        let better = |m: f64, s: usize, best: &Option<(f64, usize)>| match best {
            None => true,
            Some((bm, bs)) => m.total_cmp(bm).then(s.cmp(bs)).is_lt(),
        };
        if !self.nodes.is_empty() {
            let mut stack = vec![0usize];
            while let Some(id) = stack.pop() {
                let node = &self.nodes[id];
                if let Some((bm, _)) = best {
                    // Equal bounds can still hold a lower step number.
                    if bound(&q, w, &node.lo, &node.hi) > bm {
                        continue;
                    }
                }
                match node.kind {
                    NodeKind::Leaf(a, b) => {
                        for pt in &self.points[a..b] {
                            let m = measure(&q, w, &pt.p);
                            if better(m, pt.step, &best) {
                                best = Some((m, pt.step));
                            }
                        }
                    }
                    NodeKind::Inner(l, r) => {
                        // Nearer child last, so it is searched first.
                        let (bl, br) = (bound(&q, w, &self.nodes[l].lo, &self.nodes[l].hi), bound(&q, w, &self.nodes[r].lo, &self.nodes[r].hi));
                        if bl <= br {
                            stack.extend([r, l]);
                        } else {
                            stack.extend([l, r]);
                        }
                    }
                }
            }
        }
        for pt in &self.odd {
            let m = measure(&q, w, &pt.p);
            if better(m, pt.step, &best) {
                best = Some((m, pt.step));
            }
        }
        best.map(|(_, s)| s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute(cams: &[Camera], cam: &Camera, vw: f64) -> Option<usize> {
        let q = [cam.cx, cam.cy, cam.zoom.ln()];
        cams.iter().map(|c| measure(&q, cam.zoom / vw, &point_of(0, c).p)).enumerate().min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    }

    /// A small deterministic generator (no rand dependency).
    struct Lcg(u64);
    impl Lcg {
        fn f(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    #[test]
    fn agrees_with_measuring_every_step() {
        let mut r = Lcg(7);
        for n in [0usize, 1, 2, 9, 40, 500, 3000] {
            let cams: Vec<Camera> = (0..n)
                .map(|i| {
                    // Some steps share a camera (a frame used twice).
                    let k = if i % 7 == 3 { i - 1 } else { i };
                    let mut g = Lcg(k as u64 * 31 + 5);
                    Camera { cx: g.f() * 1e5 - 5e4, cy: g.f() * 1e5 - 5e4, zoom: (g.f() * 12.0 - 6.0).exp(), rotation: 0.0 }
                })
                .collect();
            let ix = StepIndex::from_cameras(cams.clone());
            assert_eq!(ix.len(), n);
            for _ in 0..200 {
                let cam = Camera { cx: r.f() * 1.2e5 - 6e4, cy: r.f() * 1.2e5 - 6e4, zoom: (r.f() * 14.0 - 7.0).exp(), rotation: 0.0 };
                let vw = 400.0 + r.f() * 1600.0;
                assert_eq!(ix.nearest(&cam, vw), brute(&cams, &cam, vw), "n={n} cam={cam:?}");
            }
            // Exactly on a step, including a shared one: the first of them.
            if n > 4 {
                assert_eq!(ix.nearest(&cams[3], 1000.0), brute(&cams, &cams[3], 1000.0));
                assert_eq!(ix.nearest(&cams[3], 1000.0), Some(2));
            }
        }
    }

    #[test]
    fn odd_cameras_are_still_found() {
        let cams = vec![
            Camera { cx: 0.0, cy: 0.0, zoom: 0.0, rotation: 0.0 },
            Camera { cx: 10.0, cy: 0.0, zoom: 1.0, rotation: 0.0 },
            Camera { cx: f64::NAN, cy: 0.0, zoom: 1.0, rotation: 0.0 },
        ];
        let ix = StepIndex::from_cameras(cams.clone());
        let cam = Camera { cx: 9.0, cy: 0.0, zoom: 1.0, rotation: 0.0 };
        assert_eq!(ix.nearest(&cam, 800.0), brute(&cams, &cam, 800.0));
        assert_eq!(ix.nearest(&cam, 800.0), Some(1));
    }
}
