//! Built-in themes. A theme is background + three text styles + frame and
//! shape colours, as in Prezi's 2012 theme wizard.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{ColorRef, TextStyle};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextStyleDef {
    /// Font family name. Renderers bundle these families.
    pub font: String,
    pub color: String,
    /// Font size in local units at scale 1.
    pub size: f64,
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
    pub shapes: [String; 4],
}

fn ts(font: &str, color: &str, size: f64) -> TextStyleDef {
    TextStyleDef { font: font.into(), color: color.into(), size }
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
                shapes: ["#3d7ab8".into(), "#e8792b".into(), "#6aa84f".into(), "#9e9e9e".into()],
            },
            Theme {
                id: "chalkboard".into(),
                name: "Chalkboard".into(),
                background: "#2f4a3a".into(),
                title: ts("Hand", "#f4f1e8", 72.0),
                subtitle: ts("Hand", "#f6d66b", 40.0),
                body: ts("Hand", "#e8e6dc", 24.0),
                frame: "#f4f1e8".into(),
                frame_fill: "#3a5847".into(),
                accent: "#f6d66b".into(),
                shapes: ["#f4f1e8".into(), "#f6d66b".into(), "#9fd3e6".into(), "#f2a3a3".into()],
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
                shapes: ["#ffffff".into(), "#ffd166".into(), "#a9d1ff".into(), "#7fdbca".into()],
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
                shapes: ["#9c4a2f".into(), "#5b7553".into(), "#c99a3b".into(), "#7a6a5f".into()],
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
                shapes: ["#39d0ff".into(), "#ff4fa3".into(), "#b4ff39".into(), "#8a8f99".into()],
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
                _ => "#ff00ff",
            },
            None => c.as_str(),
        };
        parse_hex(hex).unwrap_or([255, 0, 255, 255])
    }

    pub const COLOR_SLOTS: [&'static str; 11] = [
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
    ];
}

pub fn parse_hex(s: &str) -> Option<[u8; 4]> {
    let h = s.strip_prefix('#')?;
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
