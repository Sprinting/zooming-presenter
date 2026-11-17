//! Templates (whole starting presentations) and layouts (frame
//! arrangements with placeholder text), in the spirit of Prezi Classic's
//! template chooser and "Insert › Layout".
//!
//! Both are described in unit coordinates: the arrangement is 1 unit wide
//! and centred on (0, 0). The editor scales them to world units and turns
//! each [`Piece`] into an element.
use serde::Serialize;

use crate::model::{Align, FrameStyle, ShapeKind, TextStyle};

/// One element of a layout or template, in unit coordinates (centre, size).
#[derive(Debug, Clone)]
pub enum Piece {
    Frame { style: FrameStyle, x: f64, y: f64, w: f64, h: f64, rot: f64, name: &'static str, path: bool },
    /// Text scaled to fit inside `w`×`h`. `ph`: a placeholder, not content.
    Text { style: TextStyle, text: &'static str, x: f64, y: f64, w: f64, h: f64, bullets: bool, align: Align, ph: bool },
    Shape { shape: ShapeKind, x: f64, y: f64, w: f64, h: f64, rot: f64, fill: Option<&'static str>, stroke: Option<&'static str> },
}

#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Templates only: the theme they start with.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub theme: &'static str,
}

// ---- small constructors ----------------------------------------------

fn frame(style: FrameStyle, x: f64, y: f64, w: f64, h: f64, name: &'static str) -> Piece {
    Piece::Frame { style, x, y, w, h, rot: 0.0, name, path: true }
}
fn frame_r(style: FrameStyle, x: f64, y: f64, w: f64, h: f64, rot: f64, name: &'static str) -> Piece {
    Piece::Frame { style, x, y, w, h, rot, name, path: true }
}
/// A frame that isn't a path step (decoration or grouping).
fn frame_np(style: FrameStyle, x: f64, y: f64, w: f64, h: f64) -> Piece {
    Piece::Frame { style, x, y, w, h, rot: 0.0, name: "", path: false }
}
fn title_ph(x: f64, y: f64, w: f64, h: f64) -> Piece {
    Piece::Text { style: TextStyle::Title, text: "Add a title", x, y, w, h, bullets: false, align: Align::Center, ph: true }
}
fn sub(text: &'static str, x: f64, y: f64, w: f64, h: f64) -> Piece {
    Piece::Text { style: TextStyle::Subtitle, text, x, y, w, h, bullets: false, align: Align::Center, ph: false }
}
fn sub_ph(text: &'static str, x: f64, y: f64, w: f64, h: f64) -> Piece {
    Piece::Text { style: TextStyle::Subtitle, text, x, y, w, h, bullets: false, align: Align::Center, ph: true }
}
fn body_ph(x: f64, y: f64, w: f64, h: f64) -> Piece {
    Piece::Text { style: TextStyle::Body, text: "Add your points here\nOne idea per line\nKeep it short", x, y, w, h, bullets: true, align: Align::Left, ph: true }
}
fn note_ph(text: &'static str, x: f64, y: f64, w: f64, h: f64) -> Piece {
    Piece::Text { style: TextStyle::Body, text, x, y, w, h, bullets: false, align: Align::Center, ph: true }
}
fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Piece {
    let (dx, dy) = (x1 - x0, y1 - y0);
    Piece::Shape {
        shape: ShapeKind::Line,
        x: (x0 + x1) / 2.0,
        y: (y0 + y1) / 2.0,
        w: (dx * dx + dy * dy).sqrt(),
        h: 0.01,
        rot: dy.atan2(dx).to_degrees(),
        fill: None,
        stroke: Some("theme:frame"),
    }
}

use FrameStyle::{Bracket, Circle, Invisible, Rectangle};

// ---- layouts ----------------------------------------------------------

pub const LAYOUTS: &[Info] = &[
    Info { id: "title", name: "Title", description: "A frame with a title and a subtitle.", theme: "" },
    Info { id: "title_bullets", name: "Title and bullets", description: "A heading over a bulleted list.", theme: "" },
    Info { id: "two_columns", name: "Two columns", description: "A heading over two framed columns.", theme: "" },
    Info { id: "three_circles", name: "Three circles", description: "Three circle frames in a row.", theme: "" },
    Info { id: "grid", name: "Grid of four", description: "Four frames in a two-by-two grid.", theme: "" },
    Info { id: "zoom_detail", name: "Zoom into detail", description: "A big frame with three small frames to zoom into.", theme: "" },
    Info { id: "timeline", name: "Timeline", description: "Five stops along an arrow.", theme: "" },
    Info { id: "hub", name: "Hub and spokes", description: "A central idea with six around it.", theme: "" },
    Info { id: "comparison", name: "Comparison", description: "Two frames side by side.", theme: "" },
    Info { id: "picture", name: "Picture and caption", description: "Room for a picture next to a heading and text.", theme: "" },
];

/// Pieces of a layout, or None for an unknown id. 1 unit wide, 0.625 tall.
pub fn layout(id: &str) -> Option<Vec<Piece>> {
    let v = match id {
        "title" => vec![frame(Bracket, 0.0, 0.0, 1.0, 0.625, "Title"), title_ph(0.0, -0.05, 0.8, 0.14), sub_ph("Add a subtitle", 0.0, 0.1, 0.6, 0.06)],
        "title_bullets" => vec![frame(Bracket, 0.0, 0.0, 1.0, 0.625, ""), title_ph(0.0, -0.21, 0.85, 0.09), body_ph(0.0, 0.06, 0.8, 0.34)],
        "two_columns" => vec![
            frame(Bracket, 0.0, 0.0, 1.0, 0.625, ""),
            title_ph(0.0, -0.23, 0.85, 0.08),
            frame(Rectangle, -0.235, 0.06, 0.44, 0.42, "Left"),
            sub_ph("Heading", -0.235, -0.08, 0.36, 0.05),
            body_ph(-0.235, 0.09, 0.36, 0.22),
            frame(Rectangle, 0.235, 0.06, 0.44, 0.42, "Right"),
            sub_ph("Heading", 0.235, -0.08, 0.36, 0.05),
            body_ph(0.235, 0.09, 0.36, 0.22),
        ],
        "three_circles" => {
            let mut v = vec![];
            for x in [-0.33, 0.0, 0.33] {
                v.push(frame(Circle, x, 0.0, 0.29, 0.29, ""));
                v.push(sub_ph("Topic", x, -0.05, 0.2, 0.05));
                v.push(note_ph("A line or two\nabout this topic", x, 0.04, 0.2, 0.07));
            }
            v
        }
        "grid" => {
            let mut v = vec![];
            for (x, y) in [(-0.25, -0.155), (0.25, -0.155), (-0.25, 0.155), (0.25, 0.155)] {
                v.push(frame(Rectangle, x, y, 0.48, 0.29, ""));
                v.push(sub_ph("Heading", x, y - 0.07, 0.36, 0.05));
                v.push(note_ph("Add text", x, y + 0.04, 0.36, 0.08));
            }
            v
        }
        "zoom_detail" => {
            let mut v = vec![frame(Bracket, 0.0, 0.0, 1.0, 0.625, ""), title_ph(-0.17, -0.12, 0.55, 0.11), body_ph(-0.17, 0.08, 0.55, 0.22)];
            for y in [-0.18, 0.0, 0.18] {
                v.push(frame(Rectangle, 0.36, y, 0.2, 0.125, "Detail"));
                v.push(note_ph("Detail to zoom into", 0.36, y, 0.16, 0.05));
            }
            v
        }
        "timeline" => {
            let mut v = vec![Piece::Shape { shape: ShapeKind::Arrow, x: 0.0, y: 0.0, w: 0.98, h: 0.04, rot: 0.0, fill: None, stroke: Some("theme:frame") }];
            for (i, x) in [-0.38, -0.19, 0.0, 0.19, 0.38].into_iter().enumerate() {
                let y = if i % 2 == 0 { -0.13 } else { 0.13 };
                v.push(frame(Circle, x, y, 0.15, 0.15, ""));
                v.push(sub_ph("Date", x, y - 0.025, 0.11, 0.04));
                v.push(note_ph("What happened", x, y + 0.025, 0.11, 0.035));
            }
            v
        }
        "hub" => {
            let mut v = vec![];
            let n = 6;
            for i in 0..n {
                let a = (i as f64 / n as f64) * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
                let (x, y) = (0.33 * a.cos(), 0.21 * a.sin());
                v.push(line(0.13 * a.cos(), 0.13 * a.sin(), x - 0.08 * a.cos(), y - 0.08 * a.sin()));
            }
            v.push(frame(Circle, 0.0, 0.0, 0.26, 0.26, "Centre"));
            v.push(title_ph(0.0, 0.0, 0.2, 0.07));
            for i in 0..n {
                let a = (i as f64 / n as f64) * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
                let (x, y) = (0.33 * a.cos(), 0.21 * a.sin());
                v.push(frame(Circle, x, y, 0.16, 0.16, ""));
                v.push(sub_ph("Idea", x, y, 0.11, 0.045));
            }
            v
        }
        "comparison" => vec![
            frame_np(Invisible, 0.0, 0.0, 1.0, 0.625),
            title_ph(0.0, -0.25, 0.7, 0.08),
            frame(Rectangle, -0.25, 0.04, 0.46, 0.48, "Before"),
            sub_ph("Before", -0.25, -0.12, 0.3, 0.05),
            body_ph(-0.25, 0.07, 0.38, 0.24),
            frame(Rectangle, 0.25, 0.04, 0.46, 0.48, "After"),
            sub_ph("After", 0.25, -0.12, 0.3, 0.05),
            body_ph(0.25, 0.07, 0.38, 0.24),
        ],
        "picture" => vec![
            frame(Bracket, 0.0, 0.0, 1.0, 0.625, ""),
            Piece::Shape { shape: ShapeKind::Rectangle, x: -0.2, y: 0.0, w: 0.5, h: 0.46, rot: 0.0, fill: Some("theme:frame_fill"), stroke: None },
            note_ph("Drop a picture here", -0.2, 0.0, 0.3, 0.04),
            title_ph(0.27, -0.1, 0.4, 0.09),
            note_ph("Say what the picture shows", 0.27, 0.07, 0.4, 0.06),
        ],
        _ => return None,
    };
    Some(v)
}

// ---- templates ---------------------------------------------------------

pub const TEMPLATES: &[Info] = &[
    Info { id: "blank", name: "Blank", description: "An empty canvas.", theme: "classic" },
    Info { id: "big_picture", name: "Big picture", description: "A central idea and four topics around it, with an overview at the start and end.", theme: "classic" },
    Info { id: "timeline", name: "Timeline", description: "Six moments along an arrow, from left to right.", theme: "paper" },
    Info { id: "journey", name: "Journey", description: "A winding road of stops, each a little deeper than the last.", theme: "night" },
    Info { id: "mind_map", name: "Mind map", description: "A central topic with six branches.", theme: "chalkboard" },
    Info { id: "pitch", name: "Pitch", description: "Problem, solution, market, model, team and the ask.", theme: "classic" },
    Info { id: "lesson", name: "Lesson", description: "Objectives, three chapters with details to zoom into, and a recap.", theme: "paper" },
    Info { id: "story", name: "Story", description: "Beginning, middle and end, each zooming deeper into the last.", theme: "blueprint" },
];

/// A template: (theme, pieces, path order). The path is a list of indexes
/// into `pieces` (frames), so a frame can appear twice (overview at start
/// and end). Unit width; the editor scales to world units.
pub struct Template {
    pub theme: &'static str,
    pub pieces: Vec<Piece>,
    pub path: Vec<usize>,
}

pub fn template(id: &str) -> Option<Template> {
    let theme = TEMPLATES.iter().find(|t| t.id == id)?.theme;
    let mut pieces = vec![];
    let mut path = vec![];
    // Push a frame that is a path step.
    macro_rules! step {
        ($p:expr) => {{
            path.push(pieces.len());
            pieces.push($p);
        }};
    }
    match id {
        "blank" => {}
        "big_picture" => {
            let ov = pieces.len();
            step!(frame(Invisible, 0.0, 0.0, 1.0, 0.62, "Overview"));
            step!(frame(Circle, 0.0, 0.0, 0.3, 0.3, "Main idea"));
            pieces.push(title_ph(0.0, -0.02, 0.24, 0.07));
            pieces.push(sub_ph("Add a subtitle", 0.0, 0.06, 0.2, 0.03));
            for (i, (x, y)) in [(-0.33, -0.19), (0.33, -0.19), (0.33, 0.19), (-0.33, 0.19)].into_iter().enumerate() {
                step!(frame(Circle, x, y, 0.2, 0.2, ["Topic 1", "Topic 2", "Topic 3", "Topic 4"][i]));
                pieces.push(sub(["Topic 1", "Topic 2", "Topic 3", "Topic 4"][i], x, y - 0.04, 0.13, 0.035));
                pieces.push(body_ph(x, y + 0.025, 0.13, 0.06));
            }
            path.push(ov);
        }
        "timeline" => {
            step!(frame(Bracket, -0.36, -0.3, 0.26, 0.16, "Title"));
            pieces.push(title_ph(-0.36, -0.31, 0.22, 0.06));
            pieces.push(sub_ph("Add a subtitle", -0.36, -0.26, 0.2, 0.025));
            pieces.push(Piece::Shape { shape: ShapeKind::Arrow, x: 0.0, y: 0.0, w: 1.0, h: 0.03, rot: 0.0, fill: None, stroke: Some("theme:frame") });
            for i in 0..6 {
                let x = -0.4 + i as f64 * 0.16;
                let y = if i % 2 == 0 { -0.11 } else { 0.11 };
                step!(frame(Rectangle, x, y, 0.14, 0.1, ""));
                pieces.push(sub_ph("Year", x, y - 0.025, 0.1, 0.03));
                pieces.push(note_ph("What happened", x, y + 0.02, 0.11, 0.03));
            }
        }
        "journey" => {
            // A winding road: stops shrink so each step zooms in a little.
            let stops = [(-0.36, 0.14, 0.24, -8.0), (-0.13, -0.07, 0.2, 6.0), (0.07, 0.12, 0.17, -5.0), (0.23, -0.08, 0.14, 9.0), (0.35, 0.09, 0.11, -6.0), (0.44, -0.07, 0.08, 0.0)];
            step!(frame(Invisible, 0.0, 0.02, 1.0, 0.5, "The road ahead"));
            for w in stops.windows(2) {
                pieces.push(line(w[0].0, w[0].1, w[1].0, w[1].1));
            }
            for (i, (x, y, s, r)) in stops.into_iter().enumerate() {
                step!(frame_r(Bracket, x, y, s, s * 0.625, r, ["Start", "", "", "", "", "Destination"][i]));
                let t = ["Where we start", "First stop", "Second stop", "Third stop", "Almost there", "Where we arrive"][i];
                pieces.push(Piece::Text { style: TextStyle::Subtitle, text: t, x, y: y - s * 0.08, w: s * 0.8, h: s * 0.12, bullets: false, align: Align::Center, ph: i != 0 && i != 5 });
                pieces.push(note_ph("Add a note", x, y + s * 0.1, s * 0.7, s * 0.1));
            }
        }
        "mind_map" => {
            let n = 6;
            let pos = |i: usize| {
                let a = (i as f64 / n as f64) * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
                (0.36 * a.cos(), 0.22 * a.sin(), a)
            };
            for i in 0..n {
                let (x, y, a) = pos(i);
                pieces.push(line(0.12 * a.cos(), 0.12 * a.sin(), x - 0.09 * a.cos(), y - 0.06 * a.sin()));
            }
            step!(frame(Circle, 0.0, 0.0, 0.24, 0.24, "Central topic"));
            pieces.push(title_ph(0.0, 0.0, 0.18, 0.07));
            for i in 0..n {
                let (x, y, _) = pos(i);
                step!(frame(Rectangle, x, y, 0.18, 0.11, ""));
                pieces.push(sub_ph("Branch", x, y - 0.02, 0.13, 0.035));
                pieces.push(note_ph("Add a thought", x, y + 0.025, 0.13, 0.03));
            }
        }
        "pitch" => {
            step!(frame(Bracket, 0.0, -0.21, 0.5, 0.16, "Title"));
            pieces.push(title_ph(0.0, -0.225, 0.42, 0.08));
            pieces.push(sub_ph("One line on what you do", 0.0, -0.165, 0.36, 0.03));
            let heads = ["Problem", "Solution", "Market", "Business model", "Team", "The ask"];
            for (i, h) in heads.into_iter().enumerate() {
                let (x, y) = (-0.33 + (i % 3) as f64 * 0.33, 0.02 + (i / 3) as f64 * 0.2);
                step!(frame(Rectangle, x, y, 0.3, 0.18, h));
                pieces.push(sub(h, x, y - 0.05, 0.22, 0.04));
                pieces.push(body_ph(x, y + 0.025, 0.24, 0.09));
            }
        }
        "lesson" => {
            step!(frame(Bracket, -0.33, -0.17, 0.3, 0.19, "Title"));
            pieces.push(title_ph(-0.33, -0.18, 0.25, 0.07));
            pieces.push(sub_ph("Course or class", -0.33, -0.12, 0.2, 0.03));
            step!(frame(Rectangle, -0.33, 0.12, 0.3, 0.19, "Objectives"));
            pieces.push(sub("What you'll learn", -0.33, 0.06, 0.22, 0.035));
            pieces.push(body_ph(-0.33, 0.14, 0.24, 0.09));
            for (i, y) in [-0.2, 0.0, 0.2].into_iter().enumerate() {
                let name = ["Chapter 1", "Chapter 2", "Chapter 3"][i];
                step!(frame(Bracket, 0.18, y, 0.42, 0.18, name));
                pieces.push(sub(name, 0.07, y - 0.04, 0.16, 0.035));
                pieces.push(body_ph(0.07, y + 0.025, 0.17, 0.08));
                step!(frame(Circle, 0.32, y, 0.12, 0.12, "Detail"));
                pieces.push(note_ph("A detail to zoom into", 0.32, y, 0.08, 0.03));
            }
            step!(frame(Rectangle, -0.33, 0.29, 0.3, 0.1, "Recap"));
            pieces.push(sub("Recap", -0.33, 0.27, 0.12, 0.03));
            pieces.push(note_ph("The three things to remember", -0.33, 0.31, 0.24, 0.025));
        }
        "story" => {
            step!(frame(Bracket, 0.0, 0.0, 1.0, 0.62, "Beginning"));
            pieces.push(title_ph(-0.22, -0.2, 0.5, 0.1));
            pieces.push(sub("Once upon a time…", -0.22, -0.1, 0.4, 0.04));
            pieces.push(body_ph(-0.22, 0.08, 0.45, 0.2));
            step!(frame(Bracket, 0.26, 0.05, 0.4, 0.25, "Middle"));
            pieces.push(sub("Then something changed", 0.2, 0.0, 0.22, 0.03));
            pieces.push(note_ph("What happened next", 0.2, 0.05, 0.2, 0.05));
            step!(frame(Bracket, 0.39, 0.1, 0.1, 0.0625, "End"));
            pieces.push(sub("And in the end", 0.39, 0.095, 0.08, 0.012));
            pieces.push(note_ph("How it turned out", 0.39, 0.11, 0.07, 0.01));
        }
        _ => return None,
    }
    Some(Template { theme, pieces, path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_layout_and_template_exists() {
        for l in LAYOUTS {
            assert!(layout(l.id).is_some_and(|p| !p.is_empty()), "{}", l.id);
        }
        for t in TEMPLATES {
            let tp = template(t.id).unwrap();
            for &i in &tp.path {
                assert!(matches!(tp.pieces[i], Piece::Frame { .. }), "{}: path entry {i} is not a frame", t.id);
            }
            assert!(crate::theme::Theme::by_id(tp.theme).is_some());
        }
        assert_eq!(TEMPLATES.len(), 8);
    }
}
