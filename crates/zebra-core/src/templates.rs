//! Templates (whole starting presentations) and layouts (frame
//! arrangements with placeholder text), in the spirit of Prezi Classic's
//! template chooser and "Insert › Layout".
//!
//! Both are data: one JSON file each in `zebra-core/templates/layouts` and
//! `zebra-core/templates/decks`, built into the app. They are described in
//! unit coordinates: the arrangement is 1 unit wide and centred on (0, 0).
//! The editor scales them to world units and turns each [`Piece`] into an
//! element. A deck's `path` lists indexes into its pieces (frames), so a
//! frame can appear twice (an overview at the start and the end).
use serde::{Deserialize, Serialize};

use std::sync::LazyLock;

use crate::model::{Align, FrameStyle, ShapeKind, TextStyle};

/// One element of a layout or template, in unit coordinates (centre, size).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Piece {
    Frame {
        style: FrameStyle,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(default, skip_serializing_if = "is_zero")]
        rot: f64,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        name: String,
        /// A path step (layouts); templates list their path separately.
        #[serde(default, skip_serializing_if = "is_false")]
        path: bool,
        /// Frame colour (default: the theme's).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    /// Text scaled to fit inside `w`×`h`. `ph`: a placeholder, not content.
    Text {
        style: TextStyle,
        text: String,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(default, skip_serializing_if = "is_false")]
        bullets: bool,
        #[serde(default)]
        align: Align,
        #[serde(default, skip_serializing_if = "is_false")]
        ph: bool,
        /// The fill the text sits on: white text on a dark fill.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<String>,
    },
    Shape {
        shape: ShapeKind,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(default, skip_serializing_if = "is_zero")]
        rot: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<String>,
        /// Stroke width in units (default 0.004).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sw: Option<f64>,
        #[serde(default, skip_serializing_if = "crate::model::Dash::is_solid")]
        dash: crate::model::Dash,
    },
    /// A drawn line through points (unit coordinates), e.g. an arc.
    Ink {
        points: Vec<[f64; 2]>,
        /// Line width in units.
        width: f64,
        color: String,
    },
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn is_false(v: &bool) -> bool {
    !*v
}

/// A layout or a deck, as its file describes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Decks only: the theme they start with.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub theme: String,
    pub pieces: Vec<Piece>,
    /// Decks only: the path, as indexes into `pieces`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<usize>,
}

/// What the `templates` query lists and the choosers show.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub theme: String,
}

macro_rules! files {
    ($dir:literal: $($id:literal),* $(,)?) => {
        &[$(include_str!(concat!("../templates/", $dir, "/", $id, ".json"))),*]
    };
}

/// In the order the choosers show them.
const LAYOUT_FILES: &[&str] = files!("layouts": "title", "title_bullets", "two_columns", "three_circles", "grid", "zoom_detail", "timeline", "hub", "comparison", "picture");
const DECK_FILES: &[&str] = files!("decks": "blank", "big_picture", "timeline", "journey", "mind_map", "pitch", "lesson", "story");

fn parse(files: &[&str]) -> Vec<Template> {
    files.iter().map(|f| serde_json::from_str(f).unwrap_or_else(|e| panic!("built-in template file: {e}"))).collect()
}

static LAYOUT_DATA: LazyLock<Vec<Template>> = LazyLock::new(|| parse(LAYOUT_FILES));
static DECK_DATA: LazyLock<Vec<Template>> = LazyLock::new(|| parse(DECK_FILES));

fn info(t: &Template) -> Info {
    Info { id: t.id.clone(), name: t.name.clone(), description: t.description.clone(), theme: t.theme.clone() }
}

pub static LAYOUTS: LazyLock<Vec<Info>> = LazyLock::new(|| LAYOUT_DATA.iter().map(info).collect());
pub static TEMPLATES: LazyLock<Vec<Info>> = LazyLock::new(|| DECK_DATA.iter().map(info).collect());

/// Pieces of a layout, or None for an unknown id. 1 unit wide, 0.625 tall.
pub fn layout(id: &str) -> Option<Vec<Piece>> {
    LAYOUT_DATA.iter().find(|t| t.id == id).map(|t| t.pieces.clone())
}

/// A deck (whole starting presentation), or None for an unknown id.
pub fn template(id: &str) -> Option<Template> {
    DECK_DATA.iter().find(|t| t.id == id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_layout_and_template_exists() {
        for l in LAYOUTS.iter() {
            assert!(layout(&l.id).is_some_and(|p| !p.is_empty()), "{}", l.id);
        }
        for t in TEMPLATES.iter() {
            let tp = template(&t.id).unwrap();
            for &i in &tp.path {
                assert!(matches!(tp.pieces[i], Piece::Frame { .. }), "{}: path entry {i} is not a frame", t.id);
            }
            assert!(crate::theme::Theme::by_id(&tp.theme).is_some(), "{}: theme {}", t.id, tp.theme);
        }
        assert_eq!(TEMPLATES.len(), 8);
        assert_eq!(LAYOUTS.len(), 10);
    }
}
