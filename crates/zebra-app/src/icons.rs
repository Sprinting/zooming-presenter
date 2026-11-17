//! Chrome icons, painted as vector strokes at SF Symbols proportions
//! (regular weight, ~1.5 pt strokes in a 20 pt box). SF Symbols themselves
//! are not bundled or rendered (licence, and the egui-only rule); these are
//! the app's own drawings in the same optical style.
//!
//! Every icon is drawn in a unit box of −10..10 on each axis, scaled to the
//! rect it is given, so one drawing serves the 13 pt inline size, the 18 pt
//! toolbar size and the 32 pt library size.
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use zebra_core::FrameStyle;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    // Tools
    Select,
    Hand,
    Text,
    Frame(FrameStyle),
    Rect,
    Ellipse,
    Line,
    Arrow,
    Pen,
    Image,
    // Edit
    Undo,
    Redo,
    Plus,
    Minus,
    Home,
    // Toolbar
    Zoom,
    AddStep,
    Play,
    Shapes,
    Media,
    Theme,
    Share,
    Sparkles,
    Format,
    SidebarLeft,
    // Small glyphs
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    UpDown,
    Check,
    Close,
    Gear,
    Lock,
    Unlock,
    Trash,
    Note,
    Video,
    Path,
    AlignLeft,
    AlignCenter,
    AlignRight,
    List,
    /// Text toolbar: decrease / increase the list indent.
    Outdent,
    Indent,
    Bold,
    Italic,
    Underline,
    Group,
    Activity,
    Pause,
    Warning,
    Keyboard,
}

/// Every icon, for review (`zebra --demo icons`).
pub const ALL: &[Icon] = &[
    Icon::Select, Icon::Hand, Icon::Text, Icon::Frame(FrameStyle::Bracket), Icon::Frame(FrameStyle::Circle), Icon::Frame(FrameStyle::Rectangle),
    Icon::Frame(FrameStyle::Invisible), Icon::Rect, Icon::Ellipse, Icon::Line, Icon::Arrow, Icon::Pen, Icon::Image, Icon::Undo, Icon::Redo,
    Icon::Plus, Icon::Minus, Icon::Home, Icon::Zoom, Icon::AddStep, Icon::Play, Icon::Shapes, Icon::Media, Icon::Theme, Icon::Share,
    Icon::Sparkles, Icon::Format, Icon::SidebarLeft, Icon::ChevronDown, Icon::ChevronRight, Icon::ChevronLeft, Icon::UpDown, Icon::Check, Icon::Close,
    Icon::Gear, Icon::Lock, Icon::Unlock, Icon::Trash, Icon::Note, Icon::Video, Icon::Path, Icon::AlignLeft, Icon::AlignCenter,
    Icon::AlignRight, Icon::List, Icon::Bold, Icon::Italic, Icon::Underline, Icon::Group, Icon::Activity, Icon::Pause, Icon::Warning, Icon::Keyboard,
];

pub fn debug_sheet(ui: &mut egui::Ui) {
    let c = ui.visuals().text_color();
    for row in ALL.chunks(10) {
        ui.horizontal(|ui| {
            for icon in row {
                let (r, _) = ui.allocate_exact_size(Vec2::new(64.0, 64.0), egui::Sense::hover());
                ui.painter().rect_stroke(r, 2.0, Stroke::new(0.5, Color32::from_gray(200)), egui::StrokeKind::Inside);
                paint_icon(ui.painter(), Rect::from_min_size(r.min + Vec2::new(4.0, 4.0), Vec2::splat(40.0)), *icon, c);
                paint_icon(ui.painter(), Rect::from_min_size(r.min + Vec2::new(44.0, 44.0), Vec2::splat(18.0)), *icon, c);
            }
        });
    }
}

/// Stroke weight for the regular symbol weight, in unit-box units.
const W: f32 = 1.45;

pub fn paint_icon(painter: &Painter, r: Rect, icon: Icon, c: Color32) {
    let m = r.center();
    let u = r.width().min(r.height()) / 20.0;
    let p = |x: f32, y: f32| m + Vec2::new(x * u, y * u);
    let s = Stroke::new((W * u).max(1.0), c);
    let thin = Stroke::new((1.15 * u).max(0.9), c);
    // Polylines are stroked segment by segment with round joins: egui's
    // path stroker mitres sharp corners into long spikes, and SF Symbols
    // use round joins and caps anyway.
    let line = |pts: &[(f32, f32)]| round_poly(pts.iter().map(|(x, y)| p(*x, *y)).collect(), false, s);
    let closed = |pts: &[(f32, f32)]| round_poly(pts.iter().map(|(x, y)| p(*x, *y)).collect(), true, s);
    let arc = |cx: f32, cy: f32, rad: f32, a0: f32, a1: f32| -> Vec<Pos2> {
        let n = ((a1 - a0).abs() / 0.2).ceil().max(4.0) as usize;
        (0..=n).map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            p(cx + rad * a.cos(), cy + rad * a.sin())
        }).collect()
    };
    let rrect = |x0: f32, y0: f32, x1: f32, y1: f32, rad: f32| -> Vec<Pos2> {
        let mut v = vec![];
        v.extend(arc(x1 - rad, y0 + rad, rad, -FRAC_PI_2, 0.0));
        v.extend(arc(x1 - rad, y1 - rad, rad, 0.0, FRAC_PI_2));
        v.extend(arc(x0 + rad, y1 - rad, rad, FRAC_PI_2, PI));
        v.extend(arc(x0 + rad, y0 + rad, rad, PI, PI * 1.5));
        v
    };
    let rr = |x0: f32, y0: f32, x1: f32, y1: f32, rad: f32| Shape::closed_line(rrect(x0, y0, x1, y1, rad), s);
    let tri = |pts: [(f32, f32); 3], fill: Color32| Shape::convex_polygon(pts.iter().map(|(x, y)| p(*x, *y)).collect(), fill, Stroke::NONE);

    match icon {
        Icon::Select => {
            let pts = [(-4.5, -8.0), (-4.5, 6.0), (-1.0, 2.8), (1.6, 8.3), (3.9, 7.2), (1.4, 1.9), (5.8, 1.6)];
            painter.add(closed(&pts));
        }
        Icon::Hand => {
            for (x, top) in [(-4.2, -4.5), (-1.4, -7.0), (1.4, -6.5), (4.2, -4.0)] {
                painter.add(Shape::line(arc(x, top + 1.1, 1.1, PI, TAU), thin));
                painter.line_segment([p(x - 1.1, top + 1.1), p(x - 1.1, 1.5)], thin);
            }
            painter.line_segment([p(5.3, -2.9), p(5.3, 3.5)], thin);
            painter.add(Shape::line(vec![p(-5.3, -0.5), p(-7.6, -2.0), p(-8.6, -0.8), p(-5.0, 5.0), p(-2.0, 8.3), p(3.0, 8.3), p(5.3, 3.5)], thin));
        }
        Icon::Text => {
            painter.add(line(&[(-6.5, -5.5), (-6.5, -7.5), (6.5, -7.5), (6.5, -5.5)]));
            painter.add(line(&[(0.0, -7.5), (0.0, 7.5)]));
            painter.add(line(&[(-2.5, 7.5), (2.5, 7.5)]));
        }
        Icon::Frame(st) => match st {
            FrameStyle::Circle => {
                painter.circle_stroke(m, 7.5 * u, s);
            }
            FrameStyle::Rectangle => {
                painter.add(rr(-8.0, -6.0, 8.0, 6.0, 1.5));
                painter.add(Shape::convex_polygon(rrect(-8.0, -6.0, 8.0, 6.0, 1.5), c.gamma_multiply(0.18), Stroke::NONE));
            }
            FrameStyle::Invisible => {
                let pts = rrect(-8.0, -6.0, 8.0, 6.0, 1.5);
                let mut v = pts.clone();
                v.push(pts[0]);
                painter.extend(Shape::dashed_line(&v, s, 2.4 * u, 1.9 * u));
            }
            FrameStyle::Bracket => {
                painter.add(Shape::line(vec![p(-4.0, -7.0), p(-8.0, -7.0), p(-8.0, 7.0), p(-4.0, 7.0)], Stroke::new(s.width * 1.25, c)));
                painter.add(Shape::line(vec![p(4.0, -7.0), p(8.0, -7.0), p(8.0, 7.0), p(4.0, 7.0)], Stroke::new(s.width * 1.25, c)));
            }
        },
        Icon::Rect => {
            painter.add(rr(-7.5, -6.0, 7.5, 6.0, 1.8));
        }
        Icon::Ellipse => {
            painter.circle_stroke(m, 7.3 * u, s);
        }
        Icon::Line => {
            painter.add(line(&[(-7.0, 7.0), (7.0, -7.0)]));
        }
        Icon::Arrow => {
            painter.add(line(&[(-7.0, 7.0), (6.6, -6.6)]));
            painter.add(line(&[(-0.5, -7.0), (7.0, -7.0), (7.0, 0.5)]));
        }
        Icon::Pen => {
            painter.add(closed(&[(-7.5, 7.5), (-6.6, 3.4), (4.0, -7.2), (7.2, -4.0), (-3.4, 6.6)]));
            painter.add(line(&[(1.9, -5.1), (5.1, -1.9)]));
        }
        Icon::Image | Icon::Media => {
            painter.add(rr(-8.5, -6.5, 8.5, 6.5, 2.0));
            painter.add(line(&[(-8.0, 5.0), (-3.0, -0.5), (1.0, 3.2), (3.4, 1.0), (8.0, 5.0)]));
            painter.circle_filled(p(3.4, -2.6), 1.5 * u, c);
            if icon == Icon::Media {
                // a small film-strip notch marks "photo or video"
                painter.add(tri([(-1.0, -9.6), (2.4, -8.0), (-1.0, -6.4)], c));
            }
        }
        Icon::Undo | Icon::Redo => {
            let f = if icon == Icon::Redo { -1.0 } else { 1.0 };
            let q = |x: f32, y: f32| p(x * f, y);
            let mut curve: Vec<Pos2> = vec![q(-6.5, -3.0), q(2.0, -3.0)];
            curve.extend((0..=14).map(|i| {
                let a = -FRAC_PI_2 + PI * i as f32 / 14.0;
                q(2.0 + 4.6 * a.cos(), 1.6 + 4.6 * a.sin())
            }));
            curve.push(q(-1.5, 6.2));
            painter.add(Shape::line(curve, s));
            painter.add(Shape::line(vec![q(-3.0, -6.5), q(-6.5, -3.0), q(-3.0, 0.5)], s));
        }
        Icon::Plus => {
            painter.add(line(&[(-7.0, 0.0), (7.0, 0.0)]));
            painter.add(line(&[(0.0, -7.0), (0.0, 7.0)]));
        }
        Icon::Minus => {
            painter.add(line(&[(-7.0, 0.0), (7.0, 0.0)]));
        }
        Icon::Home => {
            painter.add(closed(&[(-7.0, -0.5), (0.0, -7.5), (7.0, -0.5), (5.0, -0.5), (5.0, 7.0), (-5.0, 7.0), (-5.0, -0.5)]));
        }
        Icon::Zoom => {
            painter.circle_stroke(p(-1.5, -1.5), 6.0 * u, s);
            painter.add(Shape::line(vec![p(2.9, 2.9), p(7.5, 7.5)], Stroke::new(s.width * 1.3, c)));
        }
        Icon::AddStep => {
            // rectangle.stack.badge.plus: a slide with a stack edge and a +
            painter.add(rr(-8.0, -4.0, 5.0, 7.5, 1.6));
            painter.add(line(&[(-5.5, -6.5), (7.5, -6.5), (7.5, 4.5)]));
            painter.add(line(&[(-4.5, 1.8), (1.5, 1.8)]));
            painter.add(line(&[(-1.5, -1.2), (-1.5, 4.8)]));
        }
        Icon::Play => {
            // play.circle.fill: filled disc, knocked-out triangle
            painter.circle_filled(m, 9.0 * u, c);
            let bg = if c.r() as u32 + c.g() as u32 + c.b() as u32 > 380 { Color32::BLACK } else { Color32::WHITE };
            painter.add(tri([(-2.4, -4.6), (5.0, 0.0), (-2.4, 4.6)], bg));
        }
        Icon::Shapes => {
            painter.add(rr(-8.0, -1.0, 2.0, 8.0, 1.4));
            painter.add(Shape::line(arc(2.5, -2.5, 5.3, PI * 0.5, PI * 2.5), s));
        }
        Icon::Theme => {
            // paintpalette: an open disc with a thumb notch, drawn in pieces
            // so no sharp join is stroked.
            painter.add(Shape::line(arc(0.0, 0.0, 8.0, PI * 0.42, PI * 1.98), s));
            painter.add(Shape::line(arc(5.0, 2.6, 2.4, -PI * 0.15, PI * 0.95), s));
            painter.add(round_poly(vec![p(8.0 * (PI * 1.98).cos(), 8.0 * (PI * 1.98).sin()), p(5.0 + 2.4 * (-PI * 0.15).cos(), 2.6 + 2.4 * (-PI * 0.15).sin())], false, s));
            painter.add(round_poly(vec![p(5.0 + 2.4 * (PI * 0.95).cos(), 2.6 + 2.4 * (PI * 0.95).sin()), p(8.0 * (PI * 0.42).cos(), 8.0 * (PI * 0.42).sin())], false, s));
            for (x, y) in [(-4.2, -2.0), (-1.0, -5.0), (3.4, -4.2), (-3.6, 3.6)] {
                painter.circle_filled(p(x, y), 1.35 * u, c);
            }
        }
        Icon::Share => {
            painter.add(line(&[(-3.5, -2.0), (-7.0, -2.0), (-7.0, 8.0), (7.0, 8.0), (7.0, -2.0), (3.5, -2.0)]));
            painter.add(line(&[(0.0, 3.0), (0.0, -8.5)]));
            painter.add(line(&[(-3.3, -5.3), (0.0, -8.6), (3.3, -5.3)]));
        }
        Icon::Sparkles => {
            let star = |cx: f32, cy: f32, r: f32| {
                let k = r * 0.28;
                vec![p(cx, cy - r), p(cx + k, cy - k), p(cx + r, cy), p(cx + k, cy + k), p(cx, cy + r), p(cx - k, cy + k), p(cx - r, cy), p(cx - k, cy - k)]
            };
            painter.add(Shape::closed_line(star(-2.0, 1.5, 7.0), s));
            painter.add(Shape::closed_line(star(5.5, -5.5, 3.2), thin));
        }
        Icon::Format => {
            // paintbrush: a slanted handle and a convex bristle head along
            // the same diagonal axis.
            painter.add(Shape::line_segment([p(7.8, -7.8), p(1.4, -1.4)], Stroke::new(s.width * 1.6, c)));
            let (ax, ay, tx, ty) = (1.2f32, -1.2f32, -8.4f32, 8.4f32);
            let (dx, dy) = (tx - ax, ty - ay);
            let len = (dx * dx + dy * dy).sqrt();
            let (nx, ny) = (-dy / len, dx / len);
            let prof = [(0.0f32, 1.7f32), (0.25, 2.5), (0.55, 2.3), (0.8, 1.4), (1.0, 0.0)];
            let mut pts: Vec<Pos2> = prof.iter().map(|(t, w)| p(ax + dx * t + nx * w, ay + dy * t + ny * w)).collect();
            pts.extend(prof.iter().rev().skip(1).map(|(t, w)| p(ax + dx * t - nx * w, ay + dy * t - ny * w)));
            painter.add(Shape::convex_polygon(pts, c, Stroke::NONE));
        }
        Icon::SidebarLeft => {
            painter.add(rr(-9.0, -7.0, 9.0, 7.0, 2.0));
            painter.add(line(&[(-3.0, -7.0), (-3.0, 7.0)]));
            for y in [-4.0, -1.5, 1.0] {
                painter.line_segment([p(-7.0, y), p(-5.0, y)], thin);
            }
        }
        Icon::ChevronDown => {
            painter.add(line(&[(-5.0, -2.5), (0.0, 2.5), (5.0, -2.5)]));
        }
        Icon::ChevronRight => {
            painter.add(line(&[(-2.5, -5.0), (2.5, 0.0), (-2.5, 5.0)]));
        }
        Icon::ChevronLeft => {
            painter.add(line(&[(2.5, -5.0), (-2.5, 0.0), (2.5, 5.0)]));
        }
        Icon::UpDown => {
            painter.add(line(&[(-3.5, -2.0), (0.0, -5.5), (3.5, -2.0)]));
            painter.add(line(&[(-3.5, 2.0), (0.0, 5.5), (3.5, 2.0)]));
        }
        Icon::Check => {
            painter.add(Shape::line(vec![p(-6.0, 0.5), p(-2.0, 5.0), p(6.5, -5.5)], Stroke::new(s.width * 1.3, c)));
        }
        Icon::Close => {
            painter.add(line(&[(-5.5, -5.5), (5.5, 5.5)]));
            painter.add(line(&[(5.5, -5.5), (-5.5, 5.5)]));
        }
        Icon::Gear => {
            let n = 8;
            let mut v = vec![];
            for i in 0..n * 4 {
                let a = TAU * i as f32 / (n * 4) as f32;
                let r = if (i / 2) % 2 == 0 { 8.0 } else { 6.2 };
                v.push(p(r * a.cos(), r * a.sin()));
            }
            painter.add(Shape::closed_line(v, s));
            painter.circle_stroke(m, 2.6 * u, s);
        }
        Icon::Lock | Icon::Unlock => {
            painter.add(rr(-6.5, -1.0, 6.5, 8.0, 1.6));
            let (a0, a1, cx) = if icon == Icon::Lock { (PI, TAU, 0.0) } else { (PI, TAU * 0.92, 4.0) };
            let mut v = vec![p(cx - 4.0, -1.0)];
            v.extend(arc(cx, -4.5, 4.0, a0, a1));
            if icon == Icon::Lock {
                v.push(p(4.0, -1.0));
            }
            painter.add(Shape::line(v, s));
        }
        Icon::Trash => {
            painter.add(line(&[(-8.0, -5.0), (8.0, -5.0)]));
            painter.add(line(&[(-3.0, -5.0), (-3.0, -7.5), (3.0, -7.5), (3.0, -5.0)]));
            painter.add(line(&[(-6.0, -5.0), (-5.0, 8.0), (5.0, 8.0), (6.0, -5.0)]));
        }
        Icon::Note => {
            painter.add(closed(&[(-7.0, -8.0), (7.0, -8.0), (7.0, 3.5), (2.5, 8.0), (-7.0, 8.0)]));
            painter.add(line(&[(2.5, 8.0), (2.5, 3.5), (7.0, 3.5)]));
            painter.add(line(&[(-4.0, -4.0), (4.0, -4.0)]));
            painter.add(line(&[(-4.0, -0.5), (4.0, -0.5)]));
        }
        Icon::Video => {
            painter.add(rr(-9.0, -5.5, 3.5, 5.5, 1.8));
            painter.add(closed(&[(3.5, -1.5), (9.0, -5.0), (9.0, 5.0), (3.5, 1.5)]));
        }
        Icon::Path => {
            painter.circle_stroke(p(-6.0, 5.5), 2.2 * u, s);
            painter.circle_filled(p(6.0, -5.5), 2.4 * u, c);
            let pts = vec![p(-4.0, 4.0), p(-1.0, 0.5), p(1.0, 1.5), p(4.2, -3.8)];
            painter.extend(Shape::dashed_line(&pts, s, 2.0 * u, 1.6 * u));
        }
        Icon::AlignLeft | Icon::AlignCenter | Icon::AlignRight => {
            for (i, wv) in [16.0, 11.0, 16.0, 9.0].iter().enumerate() {
                let y = -6.0 + i as f32 * 4.0;
                let (x0, x1) = match icon {
                    Icon::AlignLeft => (-8.0, -8.0 + wv),
                    Icon::AlignRight => (8.0 - wv, 8.0),
                    _ => (-wv / 2.0, wv / 2.0),
                };
                painter.line_segment([p(x0, y), p(x1, y)], s);
            }
        }
        Icon::List => {
            for i in 0..3 {
                let y = -5.5 + i as f32 * 5.5;
                painter.circle_filled(p(-6.5, y), 1.4 * u, c);
                painter.line_segment([p(-3.0, y), p(8.0, y)], s);
            }
        }
        Icon::Outdent | Icon::Indent => {
            // Lines on the right, a small arrow on the left pointing the
            // way the paragraph moves.
            painter.line_segment([p(-8.0, -7.0), p(8.0, -7.0)], s);
            painter.line_segment([p(0.0, -2.3), p(8.0, -2.3)], s);
            painter.line_segment([p(0.0, 2.3), p(8.0, 2.3)], s);
            painter.line_segment([p(-8.0, 7.0), p(8.0, 7.0)], s);
            let pts = if icon == Icon::Indent { [(-8.0, -3.5), (-3.5, 0.0), (-8.0, 3.5)] } else { [(-3.5, -3.5), (-8.0, 0.0), (-3.5, 3.5)] };
            painter.add(line(&pts));
        }
        Icon::Bold => {
            painter.text(m, egui::Align2::CENTER_CENTER, "B", crate::style::semibold(15.0 * u), c);
        }
        Icon::Italic => {
            painter.add(line(&[(-1.0, -7.0), (5.0, -7.0)]));
            painter.add(line(&[(-5.0, 7.0), (1.0, 7.0)]));
            painter.add(line(&[(2.0, -7.0), (-2.0, 7.0)]));
        }
        Icon::Underline => {
            let mut uu = vec![p(-5.0, -8.0), p(-5.0, 0.5)];
            uu.extend((0..=10).map(|i| {
                let a = PI - PI * i as f32 / 10.0;
                p(5.0 * a.cos(), 0.5 + 5.0 * a.sin())
            }));
            uu.push(p(5.0, -8.0));
            painter.add(Shape::line(uu, s));
            painter.add(line(&[(-7.0, 8.5), (7.0, 8.5)]));
        }
        Icon::Group => {
            let dash = |x0: f32, y0: f32, x1: f32, y1: f32| {
                let v = vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1), p(x0, y0)];
                Shape::dashed_line(&v, thin, 2.0 * u, 1.6 * u)
            };
            painter.extend(dash(-9.0, -8.0, 9.0, 8.0));
            painter.add(rr(-6.0, -5.0, 1.0, 1.5, 1.0));
            painter.add(rr(-1.5, -0.5, 6.0, 5.5, 1.0));
        }
        Icon::Activity => {
            painter.add(line(&[(-9.0, 0.0), (-4.5, 0.0), (-2.0, -6.5), (2.0, 6.5), (4.5, 0.0), (9.0, 0.0)]));
        }
        Icon::Pause => {
            painter.add(line(&[(-3.0, -6.5), (-3.0, 6.5)]));
            painter.add(line(&[(3.0, -6.5), (3.0, 6.5)]));
        }
        Icon::Warning => {
            painter.add(closed(&[(0.0, -8.5), (9.0, 7.5), (-9.0, 7.5)]));
            painter.add(line(&[(0.0, -2.5), (0.0, 2.5)]));
            painter.circle_filled(p(0.0, 5.0), 1.0 * u, c);
        }
        Icon::Keyboard => {
            painter.add(rr(-9.5, -6.0, 9.5, 6.0, 1.8));
            for y in [-2.5, 0.5] {
                for x in [-6.0, -3.0, 0.0, 3.0, 6.0] {
                    painter.circle_filled(p(x, y), 0.9 * u, c);
                }
            }
            painter.add(line(&[(-4.5, 3.5), (4.5, 3.5)]));
        }
    }
}

/// Stroke a polyline as separate segments with round joins and caps.
fn round_poly(pts: Vec<Pos2>, closed: bool, s: Stroke) -> Shape {
    let mut shapes = Vec::with_capacity(pts.len() * 2 + 1);
    let n = pts.len();
    let segs = if closed { n } else { n.saturating_sub(1) };
    for i in 0..segs {
        shapes.push(Shape::line_segment([pts[i], pts[(i + 1) % n]], s));
    }
    for q in &pts {
        shapes.push(Shape::circle_filled(*q, s.width / 2.0, s.color));
    }
    Shape::Vec(shapes)
}
