//! The document model. Deliberately small, like classic Prezi.
use std::collections::BTreeMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geom::{Rect, Transform, Vec2};

pub type ElementId = String;

/// A colour: `#rrggbb`, `#rrggbbaa`, or a theme slot like `theme:accent`,
/// `theme:title`, `theme:frame`, `theme:shape1`..`theme:shape4`.
pub type ColorRef = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Document {
    pub format_version: u32,
    pub title: String,
    /// Built-in theme id (`classic`, `chalkboard`, `blueprint`, `paper`, `night`).
    pub theme: String,
    /// Element store. Draw order is `z_order`, not map order.
    pub elements: BTreeMap<ElementId, Element>,
    /// Back to front.
    pub z_order: Vec<ElementId>,
    /// The presentation path: ordered camera stops.
    pub path: Vec<PathStep>,
    /// Image assets, content addressed. Not serialised in document.json; stored
    /// as separate files inside the `.zebra` bundle.
    #[serde(skip)]
    #[schemars(skip)]
    pub assets: BTreeMap<String, Asset>,
    /// Counter for readable ids (`t1`, `f2`, ...).
    pub next_id: u64,
    /// Background music for the whole presentation (plays while presenting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soundtrack: Option<Soundtrack>,
}

/// Background music: an audio asset that plays while presenting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Soundtrack {
    /// Audio asset key.
    pub asset: String,
    /// Start again when it ends.
    #[serde(default = "yes", rename = "loop")]
    pub looping: bool,
    /// 0..1.
    #[serde(default = "d_volume")]
    pub volume: f64,
    /// Human label (usually the file name).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

fn yes() -> bool {
    true
}
fn d_volume() -> f64 {
    0.6
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    /// Lower-case extension: png, jpg, webp, gif (images), mp4, mov, m4v
    /// (video), mp3, m4a, aac, wav (audio).
    pub ext: String,
    /// The encoded file (PNG, JPEG...). Assets from a saved `.zebra` are
    /// memory-mapped from it, not copied into RAM.
    pub bytes: AssetBytes,
    /// Pixels for images and video; 0 for audio.
    pub width: u32,
    pub height: u32,
}

impl Asset {
    pub fn media_kind(&self) -> MediaKind {
        MediaKind::from_ext(&self.ext)
    }
}

/// What an asset holds, from its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

impl MediaKind {
    pub fn from_ext(ext: &str) -> MediaKind {
        match ext {
            "mp4" | "mov" | "m4v" => MediaKind::Video,
            "mp3" | "m4a" | "aac" | "wav" => MediaKind::Audio,
            _ => MediaKind::Image,
        }
    }
}

/// Bytes of an asset: either owned (just inserted, not saved yet) or a
/// slice of a memory-mapped `.zebra` file. Mapped bytes cost no heap; the
/// OS pages them in when read and can drop them again under pressure.
///
/// Saving writes to a temporary file and renames it over the old one, so a
/// mapping of the old file stays valid (the old inode lives until unmapped).
/// Another program truncating the file in place would make reads fault;
/// that's the usual trade-off of memory-mapping documents.
#[derive(Clone)]
pub enum AssetBytes {
    Memory(Arc<Vec<u8>>),
    Mapped { map: Arc<memmap2::Mmap>, start: usize, len: usize },
}

impl AssetBytes {
    pub fn is_mapped(&self) -> bool {
        matches!(self, AssetBytes::Mapped { .. })
    }
}

impl std::ops::Deref for AssetBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            AssetBytes::Memory(v) => v,
            AssetBytes::Mapped { map, start, len } => &map[*start..*start + *len],
        }
    }
}

impl AsRef<[u8]> for AssetBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl From<Vec<u8>> for AssetBytes {
    fn from(v: Vec<u8>) -> Self {
        AssetBytes::Memory(Arc::new(v))
    }
}

impl PartialEq for AssetBytes {
    fn eq(&self, other: &Self) -> bool {
        let (a, b): (&[u8], &[u8]) = (self, other);
        std::ptr::eq(a, b) || a == b
    }
}

impl std::fmt::Debug for AssetBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = if self.is_mapped() { "mapped" } else { "memory" };
        write!(f, "AssetBytes({kind}, {} bytes)", self.len())
    }
}

impl Default for Document {
    fn default() -> Self {
        Document {
            format_version: crate::FORMAT_VERSION,
            title: "Untitled".into(),
            theme: "classic".into(),
            elements: BTreeMap::new(),
            z_order: vec![],
            path: vec![],
            assets: BTreeMap::new(),
            next_id: 1,
            soundtrack: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Element {
    pub id: ElementId,
    pub transform: Transform,
    /// Local size before scale (world size = size * transform.scale).
    /// For text, `w` is the wrap width and `h` is computed from layout.
    pub size: Size,
    #[serde(default)]
    pub locked: bool,
    /// Optional human name ("Intro", "Chapter 2"), shown in the path sidebar.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub kind: ElementKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ElementKind {
    Frame(FrameData),
    Text(TextData),
    Shape(ShapeData),
    Image(ImageData),
    /// A video clip (mp4/mov). Shows its poster until played.
    Video(VideoData),
    /// Freehand pen / highlighter stroke. Points are local coordinates.
    Ink(InkData),
}

impl ElementKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            ElementKind::Frame(_) => "frame",
            ElementKind::Text(_) => "text",
            ElementKind::Shape(_) => "shape",
            ElementKind::Image(_) => "image",
            ElementKind::Video(_) => "video",
            ElementKind::Ink(_) => "ink",
        }
    }
    pub fn id_prefix(&self) -> &'static str {
        match self {
            ElementKind::Frame(_) => "f",
            ElementKind::Text(_) => "t",
            ElementKind::Shape(_) => "s",
            ElementKind::Image(_) => "i",
            ElementKind::Video(_) => "v",
            ElementKind::Ink(_) => "k",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum FrameStyle {
    /// The iconic `[ ]` brackets.
    #[default]
    Bracket,
    Circle,
    Rectangle,
    /// Dashed in edit mode, invisible when presenting.
    Invisible,
}

impl FrameStyle {
    pub const ALL: [FrameStyle; 4] =
        [FrameStyle::Bracket, FrameStyle::Circle, FrameStyle::Rectangle, FrameStyle::Invisible];
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % 4]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct FrameData {
    pub style: FrameStyle,
    /// Override the theme frame colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextStyle {
    Title,
    Subtitle,
    #[default]
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct TextData {
    /// Plain text; `\n` separates paragraphs.
    pub text: String,
    #[serde(default)]
    pub style: TextStyle,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub align: Align,
    /// Each paragraph gets a bullet.
    #[serde(default)]
    pub bullets: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRef>,
    /// False: the box grows with the text (auto width). True: wraps at `size.w`.
    #[serde(default)]
    pub fixed_width: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    #[default]
    Rectangle,
    Ellipse,
    Triangle,
    /// Horizontal line across the local box (rotate to angle it).
    Line,
    /// Arrow pointing right across the local box.
    Arrow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct ShapeData {
    pub shape: ShapeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<ColorRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<ColorRef>,
    /// Local units.
    #[serde(default = "default_stroke_width")]
    pub stroke_width: f64,
}

fn default_stroke_width() -> f64 {
    4.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImageData {
    /// Asset key in the bundle (content hash).
    pub asset: String,
    #[serde(default)]
    pub alt: String,
    /// The visible part of the picture, as fractions of its full size.
    /// None shows the whole picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
    /// Outline the picture is clipped to.
    #[serde(default, skip_serializing_if = "is_default")]
    pub mask: ImageMask,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// A crop rectangle in fractions (0..1) of the source picture.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Crop {
    pub const FULL: Crop = Crop { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    /// Clamped into the picture, at least 1% wide and high.
    pub fn clamped(self) -> Crop {
        let w = self.w.clamp(0.01, 1.0);
        let h = self.h.clamp(0.01, 1.0);
        let x = self.x.clamp(0.0, 1.0 - w);
        let y = self.y.clamp(0.0, 1.0 - h);
        Crop { x, y, w, h }
    }
    pub fn is_full(&self) -> bool {
        (self.x.abs() + self.y.abs() + (1.0 - self.w).abs() + (1.0 - self.h).abs()) < 1e-9
    }
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageMask {
    #[default]
    Rectangle,
    Rounded,
    Circle,
}

/// A video clip. The bytes live in an asset like images do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VideoData {
    /// Video asset key (mp4/mov).
    pub asset: String,
    /// Image asset shown while the video isn't playing. None: a frame
    /// from the video where the platform can make one, else a placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
    /// Plays by itself when its path step is reached while presenting.
    #[serde(default = "yes")]
    pub autoplay: bool,
    #[serde(default, rename = "loop")]
    pub looping: bool,
    #[serde(default)]
    pub muted: bool,
    /// Trim: start time, seconds.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f64,
    /// Trim: end time, seconds. None plays to the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    /// Length in seconds, read from the file (0 when unknown).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub duration: f64,
    /// Description for people who can't see it, and a label.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub alt: String,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InkData {
    /// Local coordinates relative to the element centre.
    pub points: Vec<Vec2>,
    pub color: ColorRef,
    pub width: f64,
    /// Highlighter: wide and translucent.
    #[serde(default)]
    pub highlighter: bool,
}

/// One stop on the presentation path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PathStep {
    pub target: StepTarget,
    /// Elements that fade in one by one at this step (each is a sub-step).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub builds: Vec<ElementId>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// Transition duration override, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Voice-over: an audio asset that plays when this step is reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voiceover: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepTarget {
    /// Fit this element (usually a frame) to the screen, matching its rotation.
    Element { id: ElementId },
    /// A free camera view.
    View { view: ViewRect },
    /// Fit everything.
    Overview,
}

/// What the screen shows: a rotated rectangle in world space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewRect {
    pub cx: f64,
    pub cy: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub rotation: f64,
}

impl Element {
    /// Corners of the rotated box in world space.
    pub fn corners(&self) -> [Vec2; 4] {
        let (hw, hh) = (self.size.w / 2.0, self.size.h / 2.0);
        [Vec2::new(-hw, -hh), Vec2::new(hw, -hh), Vec2::new(hw, hh), Vec2::new(-hw, hh)]
            .map(|p| self.transform.apply(p))
    }
    /// Axis-aligned world bounds.
    pub fn bounds(&self) -> Rect {
        Rect::from_points(self.corners()).unwrap()
    }
    pub fn world_size(&self) -> (f64, f64) {
        (self.size.w * self.transform.scale, self.size.h * self.transform.scale)
    }
    /// Precise hit test against the rotated box (frames: only near their border
    /// unless `interior` is true).
    pub fn hit(&self, p: Vec2, tolerance_world: f64, interior: bool) -> bool {
        let l = self.transform.inverse(p);
        let tol = tolerance_world / self.transform.scale;
        let (hw, hh) = (self.size.w / 2.0, self.size.h / 2.0);
        let inside = l.x.abs() <= hw + tol && l.y.abs() <= hh + tol;
        if !inside {
            return false;
        }
        match &self.kind {
            ElementKind::Frame(_) if !interior => {
                let edge = tol.max(self.size.w.min(self.size.h) * 0.04);
                l.x.abs() >= hw - edge || l.y.abs() >= hh - edge
            }
            _ => true,
        }
    }
    pub fn is_frame(&self) -> bool {
        matches!(self.kind, ElementKind::Frame(_))
    }
    /// Short human label: name, else first text line, else type.
    pub fn label(&self, doc: &Document) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        match &self.kind {
            ElementKind::Text(t) => truncate(t.text.lines().next().unwrap_or(""), 40),
            ElementKind::Frame(_) => doc
                .frame_contents(&self.id)
                .iter()
                .filter_map(|id| doc.elements.get(id))
                .find_map(|e| match &e.kind {
                    ElementKind::Text(t) => Some(truncate(t.text.lines().next().unwrap_or(""), 40)),
                    _ => None,
                })
                .unwrap_or_else(|| "frame".into()),
            ElementKind::Image(i) if !i.alt.is_empty() => truncate(&i.alt, 40),
            ElementKind::Video(v) if !v.alt.is_empty() => truncate(&v.alt, 40),
            k => k.type_name().into(),
        }
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n - 1).collect();
        t.push('…');
        t
    }
}

impl Document {
    pub fn get(&self, id: &str) -> Option<&Element> {
        self.elements.get(id)
    }
    /// Elements in draw order.
    pub fn ordered(&self) -> impl DoubleEndedIterator<Item = &Element> {
        self.z_order.iter().filter_map(|id| self.elements.get(id))
    }
    /// Bounds of everything (None when empty).
    pub fn bounds(&self) -> Option<Rect> {
        self.ordered().map(|e| e.bounds()).reduce(|a, b| a.union(&b))
    }
    /// Classic Prezi semantics: a frame "contains" every element whose bounds lie
    /// inside the frame's bounds and that is smaller than the frame. Computed
    /// geometrically, so dragging something into a frame makes it a child.
    pub fn frame_contents(&self, frame_id: &str) -> Vec<ElementId> {
        let Some(f) = self.elements.get(frame_id) else { return vec![] };
        let fb = f.bounds();
        let fa = fb.area();
        self.ordered()
            .filter(|e| e.id != f.id && e.bounds().area() < fa && fb.contains_rect(&e.bounds()))
            .map(|e| e.id.clone())
            .collect()
    }
    /// Smallest frame that contains this element.
    pub fn parent_frame(&self, id: &str) -> Option<ElementId> {
        let e = self.elements.get(id)?;
        let b = e.bounds();
        self.ordered()
            .filter(|f| f.is_frame() && f.id != id)
            .filter(|f| f.bounds().contains_rect(&b) && f.bounds().area() > b.area())
            .min_by(|a, b| a.bounds().area().total_cmp(&b.bounds().area()))
            .map(|f| f.id.clone())
    }
    /// Topmost element under a world point.
    pub fn hit_test(&self, p: Vec2, tolerance_world: f64) -> Option<&Element> {
        self.ordered().rev().find(|e| e.hit(p, tolerance_world, false))
    }
    pub fn alloc_id(&mut self, prefix: &str) -> ElementId {
        loop {
            let id = format!("{prefix}{}", self.next_id);
            self.next_id += 1;
            if !self.elements.contains_key(&id) {
                return id;
            }
        }
    }
    /// Every asset key the document refers to (elements, posters,
    /// soundtrack, voice-overs).
    pub fn used_assets(&self) -> std::collections::BTreeSet<&str> {
        let mut out = std::collections::BTreeSet::new();
        for e in self.elements.values() {
            match &e.kind {
                ElementKind::Image(i) => {
                    out.insert(i.asset.as_str());
                }
                ElementKind::Video(v) => {
                    out.insert(v.asset.as_str());
                    if let Some(p) = &v.poster {
                        out.insert(p.as_str());
                    }
                }
                _ => {}
            }
        }
        if let Some(s) = &self.soundtrack {
            out.insert(s.asset.as_str());
        }
        for s in &self.path {
            if let Some(v) = &s.voiceover {
                out.insert(v.as_str());
            }
        }
        out
    }
    /// Steps where an element is used (as target or build).
    pub fn steps_using(&self, id: &str) -> Vec<usize> {
        self.path
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                matches!(&s.target, StepTarget::Element { id: t } if t == id) || s.builds.iter().any(|b| b == id)
            })
            .map(|(i, _)| i)
            .collect()
    }
}
