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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    /// Lower-case extension: png, jpg, webp, gif, svg.
    pub ext: String,
    /// The encoded file (PNG, JPEG...). Assets from a saved `.zebra` are
    /// memory-mapped from it, not copied into RAM.
    pub bytes: AssetBytes,
    pub width: u32,
    pub height: u32,
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
    /// Freehand pen / highlighter stroke. Points are local coordinates.
    Ink(InkData),
    /// A line icon from the built-in library (see `icons`).
    Icon(IconData),
    /// A line or arrow whose ends can stick to other elements.
    Connector(ConnectorData),
}

impl ElementKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            ElementKind::Frame(_) => "frame",
            ElementKind::Text(_) => "text",
            ElementKind::Shape(_) => "shape",
            ElementKind::Image(_) => "image",
            ElementKind::Ink(_) => "ink",
            ElementKind::Icon(_) => "icon",
            ElementKind::Connector(_) => "connector",
        }
    }
    pub fn id_prefix(&self) -> &'static str {
        match self {
            ElementKind::Frame(_) => "f",
            ElementKind::Text(_) => "t",
            ElementKind::Shape(_) => "s",
            ElementKind::Image(_) => "i",
            ElementKind::Ink(_) => "k",
            ElementKind::Icon(_) => "n",
            ElementKind::Connector(_) => "c",
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
    /// Character formatting on parts of the text (bold word, coloured
    /// phrase, link). Ranges are in characters (Unicode scalar values), not
    /// bytes. Kept sorted and non-overlapping; see [`crate::rich`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<TextSpan>,
    /// Per-paragraph formatting, one entry per `\n`-separated paragraph
    /// (missing entries use the defaults: `bullets` for the list, indent 0).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paras: Vec<ParaFormat>,
}

/// Formatting for the characters `start..end` of a text. Unset fields
/// inherit the text box's own settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
    #[serde(flatten)]
    pub format: SpanFormat,
}

/// Character formatting. `None` leaves a property as it is (when applying)
/// or inherited (when stored).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct SpanFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    /// `#rrggbb` or a theme slot. An empty string clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRef>,
    /// URL opened when the text is clicked while presenting. An empty
    /// string removes the link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Size relative to the text style (1.0 = normal, 2.0 = twice as big).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    #[default]
    None,
    Bullet,
    Number,
}

/// Formatting of one paragraph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct ParaFormat {
    /// Indent level, 0 to 8.
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub indent: u8,
    /// `None` here means "use the box's `bullets` setting".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<ListKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

macro_rules! shape_kinds {
    ($($(#[$m:meta])* $v:ident => $label:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
        #[serde(rename_all = "snake_case")]
        pub enum ShapeKind {
            #[default]
            $($(#[$m])* $v),*
        }
        impl ShapeKind {
            /// Every shape, in library order.
            pub const ALL: &'static [ShapeKind] = &[$(ShapeKind::$v),*];
            /// Human name for menus and the library.
            pub fn label(self) -> &'static str {
                match self { $(ShapeKind::$v => $label),* }
            }
        }
    };
}

shape_kinds! {
    Rectangle => "Rectangle",
    RoundedRect => "Rounded Rectangle",
    Ellipse => "Oval",
    Triangle => "Triangle",
    RightTriangle => "Right Triangle",
    Diamond => "Diamond",
    Pentagon => "Pentagon",
    Hexagon => "Hexagon",
    Octagon => "Octagon",
    Parallelogram => "Parallelogram",
    Trapezoid => "Trapezoid",
    Star => "Star",
    Sparkle => "Four-Point Star",
    Burst => "Burst",
    Heart => "Heart",
    Plus => "Plus",
    Ring => "Ring",
    Moon => "Moon",
    Lightning => "Lightning",
    Cloud => "Cloud",
    SpeechBubble => "Speech Bubble",
    Callout => "Callout",
    ThoughtBubble => "Thought Bubble",
    StickyNote => "Sticky Note",
    Document => "Document",
    Cylinder => "Cylinder",
    Cube => "Cube",
    Pill => "Pill",
    Delay => "Half Round",
    Chevron => "Chevron",
    Tag => "Tag",
    BlockArrowRight => "Block Arrow Right",
    BlockArrowLeft => "Block Arrow Left",
    BlockArrowUp => "Block Arrow Up",
    BlockArrowDown => "Block Arrow Down",
    BlockArrowBoth => "Block Arrow Both Ways",
    /// Horizontal line across the local box (rotate to angle it).
    Line => "Line",
    /// Arrow pointing right across the local box.
    Arrow => "Arrow",
    /// Line with arrowheads at both ends.
    DoubleArrow => "Double Arrow",
}

impl ShapeKind {
    /// Lines and arrows: drawn with the stroke only, never filled.
    pub fn is_line(self) -> bool {
        matches!(self, ShapeKind::Line | ShapeKind::Arrow | ShapeKind::DoubleArrow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Dash {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl Dash {
    pub fn is_solid(&self) -> bool {
        *self == Dash::Solid
    }
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
    /// Corner rounding in local units for rectangles, callouts and sticky
    /// notes. 0 means the shape's own default.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub corner_radius: f64,
    #[serde(default, skip_serializing_if = "Dash::is_solid")]
    pub dash: Dash,
}

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

/// A line icon. Icons are drawn from a 24x24 grid scaled uniformly to fit
/// the element box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IconData {
    /// Library name, e.g. "lightbulb". See the `list_icons` query.
    pub name: String,
    pub color: ColorRef,
    /// Stroke width on the 24-unit icon grid (Lucide's default is 2).
    #[serde(default = "default_icon_stroke")]
    pub stroke_width: f64,
}

fn default_icon_stroke() -> f64 {
    2.0
}

/// One end of a connector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct Endpoint {
    /// Stuck to this element: the end follows it around and sits on its edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<ElementId>,
    /// Where the end is, relative to the connector's centre (kept up to date
    /// for attached ends too).
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    /// Direction the line leaves this end (unit vector, maintained by the
    /// editor; used for curves and elbows).
    #[serde(default, skip_serializing_if = "is_zero_vec")]
    pub dir: Vec2,
}

fn is_zero_vec(v: &Vec2) -> bool {
    v.x == 0.0 && v.y == 0.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    #[default]
    Straight,
    /// Right angles only.
    Elbow,
    Curved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Arrowhead {
    #[default]
    None,
    /// Filled triangle.
    Arrow,
    /// Open V.
    Open,
    Dot,
}

impl Arrowhead {
    pub fn is_none(&self) -> bool {
        *self == Arrowhead::None
    }
}

fn default_end_head() -> Arrowhead {
    Arrowhead::Arrow
}

fn default_connector_width() -> f64 {
    3.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConnectorData {
    pub from: Endpoint,
    pub to: Endpoint,
    #[serde(default)]
    pub route: Route,
    #[serde(default, skip_serializing_if = "Arrowhead::is_none")]
    pub start: Arrowhead,
    #[serde(default = "default_end_head")]
    pub end: Arrowhead,
    pub color: ColorRef,
    #[serde(default = "default_connector_width")]
    pub width: f64,
    #[serde(default, skip_serializing_if = "Dash::is_solid")]
    pub dash: Dash,
    /// Short text drawn at the middle of the line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
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
            ElementKind::Connector(c) => crate::connector::distance(c, l) <= tol + c.width / 2.0 + 2.0,
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
