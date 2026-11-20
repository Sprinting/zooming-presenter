//! Built-in themes. A theme is background + three text styles + frame and
//! shape colours.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::model::{ColorRef, Document, TextStyle};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextStyleDef {
    /// Font family name. Renderers bundle these families.
    pub font: String,
    pub color: String,
    /// Font size in local units at scale 1.
    pub size: f64,
    /// Font weight, 1 to 1000 as in CSS and OpenType: 400 Regular, 600
    /// Semibold, 700 Bold. Older files have none: Regular. A font without
    /// this weight is drawn in its nearest face, never a faked bold.
    #[serde(default = "d_weight")]
    pub weight: u16,
}

/// Four to six shape colours; a missing fifth or sixth is left empty.
fn four_to_six<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[String; 6], D::Error> {
    let v: Vec<String> = Vec::deserialize(d)?;
    if !(4..=6).contains(&v.len()) {
        return Err(serde::de::Error::custom(format!("shapes has {} colours; give 4 to 6", v.len())));
    }
    let mut out: [String; 6] = Default::default();
    for (i, c) in v.into_iter().enumerate() {
        out[i] = c;
    }
    Ok(out)
}

fn d_weight() -> u16 {
    REGULAR
}

/// Regular weight (CSS 400).
pub const REGULAR: u16 = 400;
/// Bold weight (CSS 700).
pub const BOLD: u16 = 700;
/// Styles at this weight or heavier count as bold: the B button shows on
/// and Bold on their words turns them Regular (Keynote, Pages).
pub const BOLD_FROM: u16 = 600;

/// The usual weights, lightest first, with the names the macOS Fonts
/// window gives them (agent-facing English; the app translates).
pub const WEIGHT_NAMES: [(u16, &str); 9] = [
    (100, "Ultralight"),
    (200, "Thin"),
    (300, "Light"),
    (400, "Regular"),
    (500, "Medium"),
    (600, "Semibold"),
    (700, "Bold"),
    (800, "Heavy"),
    (900, "Black"),
];

/// The macOS name of a usual weight ("Semibold"), or None for an odd one.
pub fn weight_name(w: u16) -> Option<&'static str> {
    WEIGHT_NAMES.iter().find(|(v, _)| *v == w).map(|(_, n)| *n)
}

/// The weight in `have` nearest to `w` (a lighter one on a tie, below
/// Medium; a heavier one from Medium up). `w` itself when `have` is empty.
pub fn nearest_weight(have: &[u16], w: u16) -> u16 {
    have.iter()
        .copied()
        .min_by_key(|&h| {
            let d = (h as i32 - w as i32).unsigned_abs();
            let tie = if w >= 500 { h < w } else { h > w };
            (d, tie)
        })
        .unwrap_or(w)
}

/// The weight characters are drawn at in a style of weight `style`, with
/// their bold setting (None: as the style). Bold in a style that is
/// already bold or heavier keeps the style's weight, and "not bold" there
/// is Regular; in a lighter style bold is Bold and "not bold" is the
/// style's own weight. So Bold toggles between the two, as in Keynote.
pub fn bold_weight(style: u16, bold: Option<bool>) -> u16 {
    let heavy = style >= BOLD_FROM;
    match bold {
        None => style,
        Some(true) if heavy => style,
        Some(true) => BOLD,
        Some(false) if heavy => REGULAR,
        Some(false) => style,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub background: String,
    pub title: TextStyleDef,
    pub subtitle: TextStyleDef,
    pub body: TextStyleDef,
    pub frame: String,
    /// Fill for rectangle/circle frames.
    pub frame_fill: String,
    pub accent: String,
    /// Shape and series colours, `theme:shape1` to `theme:shape6`. Files
    /// from before shape5 and shape6 have four: the missing two are empty
    /// here and resolve to the base theme's (or Classic's), see
    /// [`Theme::shape`].
    #[serde(deserialize_with = "four_to_six")]
    #[schemars(with = "Vec<String>", length(min = 4, max = 6))]
    pub shapes: [String; 6],
    /// A picture behind everything, fixed to the screen (asset key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_image: Option<String>,
    /// 3D background: up to three picture layers, back to front, that move
    /// slower than the canvas as the camera flies (a 3D background).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<BackgroundLayer>,
    /// Fonts embedded in the document, so the file looks the same on a Mac
    /// without them: family name -> font file asset keys.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fonts: BTreeMap<String, Vec<String>>,
    /// A custom theme: the theme it was made from. Deleting the custom
    /// theme goes back to this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

/// One layer of a 3D (parallax) background.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BackgroundLayer {
    /// Image asset key.
    pub asset: String,
    /// How much the layer follows the camera, 0..1. 0 stays still like a
    /// backdrop; 1 moves with the canvas. Back layers use small values.
    #[serde(default = "d_depth")]
    pub depth: f64,
    /// 0..1.
    #[serde(default = "d_opacity")]
    pub opacity: f64,
}

fn d_depth() -> f64 {
    0.3
}
fn d_opacity() -> f64 {
    1.0
}

/// Most layers a 3D background can have.
pub const MAX_LAYERS: usize = 3;

fn ts(font: &str, color: &str, size: f64) -> TextStyleDef {
    TextStyleDef { font: font.into(), color: color.into(), size, weight: REGULAR }
}

impl Theme {
    pub fn builtins() -> Vec<Theme> {
        vec![
            Theme {
                id: "classic".into(),
                name: "Classic".into(),
                background: "#ffffff".into(),
                title: ts("Sans", "#2b2b2b", 72.0),
                subtitle: ts("Sans", "#3d7ab8", 40.0),
                body: ts("Sans", "#4a4a4a", 24.0),
                frame: "#3d7ab8".into(),
                frame_fill: "#e8f0f8".into(),
                accent: "#e8792b".into(),
                shapes: ["#3d7ab8".into(), "#e8792b".into(), "#6aa84f".into(), "#9e9e9e".into(), "#7b5ea7".into(), "#c0392b".into()],
                background_image: None,
                layers: vec![],
                fonts: BTreeMap::new(),
                base: None,
            },
            Theme {
                id: "chalkboard".into(),
                name: "Chalkboard".into(),
                background: "#2f4a3a".into(),
                title: ts("Hand", "#f4f1e8", 72.0),
                subtitle: ts("Hand", "#f6d66b", 40.0),
                // Caveat only for headings: at body size it is hard to read
                // from the back of a room.
                body: ts("Sans", "#e8e6dc", 24.0),
                frame: "#f4f1e8".into(),
                frame_fill: "#3a5847".into(),
                accent: "#f6d66b".into(),
                shapes: ["#f4f1e8".into(), "#f6d66b".into(), "#9fd3e6".into(), "#f2a3a3".into(), "#c5e1a5".into(), "#ffcc99".into()],
                background_image: None,
                layers: vec![],
                fonts: BTreeMap::new(),
                base: None,
            },
            Theme {
                id: "blueprint".into(),
                name: "Blueprint".into(),
                background: "#1d4e89".into(),
                title: ts("Mono", "#ffffff", 72.0),
                subtitle: ts("Mono", "#a9d1ff", 40.0),
                body: ts("Sans", "#dbeaff", 24.0),
                frame: "#a9d1ff".into(),
                frame_fill: "#245a9b".into(),
                accent: "#ffd166".into(),
                shapes: ["#ffffff".into(), "#ffd166".into(), "#a9d1ff".into(), "#7fdbca".into(), "#ffb3c6".into(), "#c3f584".into()],
                background_image: None,
                layers: vec![],
                fonts: BTreeMap::new(),
                base: None,
            },
            Theme {
                id: "paper".into(),
                name: "Paper".into(),
                background: "#f6f0e4".into(),
                title: ts("Serif", "#3b2f2a", 72.0),
                subtitle: ts("Serif", "#9c4a2f", 40.0),
                body: ts("Serif", "#4d413a", 24.0),
                frame: "#9c4a2f".into(),
                frame_fill: "#ece2cf".into(),
                accent: "#9c4a2f".into(),
                shapes: ["#9c4a2f".into(), "#5b7553".into(), "#c99a3b".into(), "#7a6a5f".into(), "#3f6e8c".into(), "#8c5a7a".into()],
                background_image: None,
                layers: vec![],
                fonts: BTreeMap::new(),
                base: None,
            },
            Theme {
                id: "night".into(),
                name: "Night".into(),
                background: "#111318".into(),
                title: ts("Sans", "#f2f2f2", 72.0),
                subtitle: ts("Sans", "#39d0ff", 40.0),
                body: ts("Sans", "#c9ccd3", 24.0),
                frame: "#39d0ff".into(),
                frame_fill: "#1b1f29".into(),
                accent: "#ff4fa3".into(),
                shapes: ["#39d0ff".into(), "#ff4fa3".into(), "#b4ff39".into(), "#8a8f99".into(), "#ffb547".into(), "#a78bfa".into()],
                background_image: None,
                layers: vec![],
                fonts: BTreeMap::new(),
                base: None,
            },
        ]
    }

    pub fn by_id(id: &str) -> Option<Theme> {
        Self::builtins().into_iter().find(|t| t.id == id)
    }

    /// Never fails: unknown ids fall back to classic.
    pub fn resolve(id: &str) -> Theme {
        Self::by_id(id).unwrap_or_else(|| Self::builtins().remove(0))
    }

    /// Look a theme up in a document: its own custom themes first, then the
    /// built-ins. Never fails: unknown ids fall back to classic.
    pub fn resolve_in(doc: &Document, id: &str) -> Theme {
        doc.themes.iter().find(|t| t.id == id).cloned().unwrap_or_else(|| Self::resolve(id))
    }

    /// Built-in and document theme ids.
    pub fn ids_in(doc: &Document) -> Vec<String> {
        let mut v = Self::ids();
        v.extend(doc.themes.iter().map(|t| t.id.clone()));
        v
    }

    pub fn is_builtin(id: &str) -> bool {
        Self::by_id(id).is_some()
    }

    /// Assets this theme refers to (background, layers, fonts).
    pub fn asset_keys(&self) -> impl Iterator<Item = &str> {
        self.background_image
            .iter()
            .map(|s| s.as_str())
            .chain(self.layers.iter().map(|l| l.asset.as_str()))
            .chain(self.fonts.values().flatten().map(|s| s.as_str()))
    }

    /// Every colour of the theme, for validation: (field, value).
    pub fn colors(&self) -> Vec<(&'static str, &str)> {
        let mut v = vec![
            ("background", self.background.as_str()),
            ("title.color", self.title.color.as_str()),
            ("subtitle.color", self.subtitle.color.as_str()),
            ("body.color", self.body.color.as_str()),
            ("frame", self.frame.as_str()),
            ("frame_fill", self.frame_fill.as_str()),
            ("accent", self.accent.as_str()),
        ];
        for (i, c) in self.shapes.iter().enumerate() {
            // An empty fifth or sixth follows the base theme (older files).
            if !c.is_empty() {
                v.push((["shapes[0]", "shapes[1]", "shapes[2]", "shapes[3]", "shapes[4]", "shapes[5]"][i], c.as_str()));
            }
        }
        v
    }

    pub fn ids() -> Vec<String> {
        Self::builtins().into_iter().map(|t| t.id).collect()
    }

    pub fn text_style(&self, s: TextStyle) -> &TextStyleDef {
        match s {
            TextStyle::Title => &self.title,
            TextStyle::Subtitle => &self.subtitle,
            TextStyle::Body => &self.body,
        }
    }

    /// Text colour for writing on a shape filled with `fill`: white on a
    /// dark fill, None (the theme's text colour) otherwise.
    pub fn text_on(&self, fill: Option<&ColorRef>) -> Option<ColorRef> {
        let lum = |[r, g, b, _]: [u8; 4]| {
            let lin = |c: u8| {
                let c = c as f64 / 255.0;
                if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
        };
        let f = self.color(fill?);
        if f[3] <= 128 {
            return None;
        }
        // WCAG contrast ratio of the theme's text and of white on the fill.
        let contrast = |a: f64, b: f64| (a.max(b) + 0.05) / (a.min(b) + 0.05);
        let lf = lum(f);
        let text = lum(self.color(&"theme:body".to_string()));
        (contrast(1.0, lf) > contrast(text, lf)).then(|| "#ffffff".to_string())
    }

    /// Resolve a [`ColorRef`] to `[r,g,b,a]` in 0..=255. Unknown -> magenta.
    pub fn color(&self, c: &ColorRef) -> [u8; 4] {
        let hex = match c.strip_prefix("theme:") {
            Some(slot) => match slot {
                "background" => self.background.as_str(),
                "title" => &self.title.color,
                "subtitle" => &self.subtitle.color,
                "body" | "text" => &self.body.color,
                "frame" => &self.frame,
                "frame_fill" => &self.frame_fill,
                "accent" => &self.accent,
                "shape1" => &self.shapes[0],
                "shape2" => &self.shapes[1],
                "shape3" => &self.shapes[2],
                "shape4" => &self.shapes[3],
                "shape5" => return self.shape(4),
                "shape6" => return self.shape(5),
                _ => "#ff00ff",
            },
            None => c.as_str(),
        };
        parse_hex(hex).unwrap_or([255, 0, 255, 255])
    }

    /// Shape colour `i` (0-based, 0..6) as RGBA. A fifth or sixth colour
    /// a theme from before they existed leaves empty comes from its base
    /// theme, else Classic.
    pub fn shape(&self, i: usize) -> [u8; 4] {
        let own = self.shapes.get(i).map(String::as_str).unwrap_or("");
        if let Some(c) = parse_hex(own) {
            return c;
        }
        let base = self.base.as_deref().and_then(Theme::by_id).filter(|b| b.id != self.id).unwrap_or_else(|| Theme::resolve("classic"));
        base.shapes.get(i).and_then(|c| parse_hex(c)).unwrap_or([255, 0, 255, 255])
    }

    pub const COLOR_SLOTS: [&'static str; 13] = [
        "theme:background",
        "theme:title",
        "theme:subtitle",
        "theme:body",
        "theme:frame",
        "theme:frame_fill",
        "theme:accent",
        "theme:shape1",
        "theme:shape2",
        "theme:shape3",
        "theme:shape4",
        "theme:shape5",
        "theme:shape6",
    ];
}

pub fn parse_hex(s: &str) -> Option<[u8; 4]> {
    let h = s.strip_prefix('#')?;
    // Hex digits only: the slices below index by byte.
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([p(0)?, p(2)?, p(4)?, 255]),
        8 => Some([p(0)?, p(2)?, p(4)?, p(6)?]),
        3 => {
            let q = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|v| v * 17);
            Some([q(0)?, q(1)?, q(2)?, 255])
        }
        _ => None,
    }
}

pub fn valid_color(c: &str) -> bool {
    Theme::COLOR_SLOTS.contains(&c) || parse_hex(c).is_some()
}

#[cfg(test)]
mod weight_tests {
    use super::*;

    #[test]
    fn old_styles_read_as_regular_and_builtins_stay_regular() {
        let s: TextStyleDef = serde_json::from_str(r##"{"font":"Sans","color":"#000000","size":72}"##).unwrap();
        assert_eq!(s.weight, REGULAR);
        for t in Theme::builtins() {
            for s in [&t.title, &t.subtitle, &t.body] {
                assert_eq!(s.weight, REGULAR, "{}", t.id);
            }
        }
        let classic = Theme::resolve("classic");
        assert_eq!((classic.title.font.as_str(), classic.title.weight), ("Sans", 400));
    }

    #[test]
    fn nearest_weight_picks_the_closest_face() {
        assert_eq!(nearest_weight(&[400, 700], 600), 700);
        assert_eq!(nearest_weight(&[400, 700], 500), 400);
        assert_eq!(nearest_weight(&[400, 700], 300), 400);
        assert_eq!(nearest_weight(&[400, 700], 900), 700);
        assert_eq!(nearest_weight(&[300, 500], 400), 300, "a tie below Medium goes lighter");
        assert_eq!(nearest_weight(&[500, 700], 600), 700, "a tie from Medium up goes heavier");
        assert_eq!(nearest_weight(&[], 600), 600);
        assert_eq!(nearest_weight(&[100, 200, 300, 400, 500, 600, 700, 800, 900], 600), 600);
    }

    #[test]
    fn bold_toggles_between_regular_and_the_heavy_style() {
        // Light and Regular styles: Bold is 700, "not bold" the style's own.
        assert_eq!(bold_weight(400, Some(true)), 700);
        assert_eq!(bold_weight(300, Some(false)), 300);
        assert_eq!(bold_weight(500, Some(true)), 700);
        // Semibold and heavier: bold keeps the style, "not bold" is Regular.
        assert_eq!(bold_weight(600, Some(true)), 600);
        assert_eq!(bold_weight(800, Some(false)), 400);
        assert_eq!(bold_weight(800, None), 800);
        assert_eq!(weight_name(600), Some("Semibold"));
        assert_eq!(weight_name(650), None);
    }
}
