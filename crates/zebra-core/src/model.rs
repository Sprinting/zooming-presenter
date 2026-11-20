//! The document model. Deliberately small.
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
    /// Custom themes saved in this document (made with the theme editor).
    /// `theme` may name one of these as well as a built-in theme.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<crate::theme::Theme>,
    /// How the presentation plays: autoplay and loop.
    #[serde(default, skip_serializing_if = "PlaySettings::is_default")]
    pub play: PlaySettings,
    /// Background music for the whole presentation (plays while presenting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soundtrack: Option<Soundtrack>,
}

/// Playback options for the whole presentation (the autoplay menu).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct PlaySettings {
    /// Advance automatically after this many seconds on each step and
    /// build. `None` = advance by hand. A step's own `dwell` overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoplay: Option<f64>,
    /// After the last step, go back to the first one.
    #[serde(default, rename = "loop")]
    pub looping: bool,
}

impl PlaySettings {
    pub fn is_default(&self) -> bool {
        *self == PlaySettings::default()
    }
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
    /// (video), mp3, m4a, aac, wav (audio), pdf (an imported PDF, drawn
    /// through the pictures that show its pages).
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
    /// A PDF that imported pages are drawn from.
    Document,
}

impl MediaKind {
    pub fn from_ext(ext: &str) -> MediaKind {
        match ext {
            "mp4" | "mov" | "m4v" => MediaKind::Video,
            "mp3" | "m4a" | "aac" | "wav" => MediaKind::Audio,
            "pdf" => MediaKind::Document,
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
    Mapped { map: Arc<crate::media::MappedFile>, start: usize, len: usize },
}

impl AssetBytes {
    pub fn is_mapped(&self) -> bool {
        matches!(self, AssetBytes::Mapped { .. })
    }
    /// The mapped file these bytes come from, if they are mapped.
    pub fn mapped_file(&self) -> Option<&Arc<crate::media::MappedFile>> {
        match self {
            AssetBytes::Mapped { map, .. } => Some(map),
            AssetBytes::Memory(_) => None,
        }
    }
    /// The file holding exactly these bytes and nothing else (a spill
    /// file, a media file of an autosave), if there is one: a player can
    /// open it as it is, and an autosave can link to it.
    pub fn own_file(&self) -> Option<&std::path::Path> {
        match self {
            AssetBytes::Mapped { map, start: 0, len } if *len == map.len() => map.path(),
            _ => None,
        }
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
            // No title yet: the app names a new document in its language.
            title: String::new(),
            theme: "classic".into(),
            elements: BTreeMap::new(),
            z_order: vec![],
            path: vec![],
            assets: BTreeMap::new(),
            next_id: 1,
            themes: vec![],
            play: PlaySettings::default(),
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
    /// A layout or template placeholder ("Double-click to add a title").
    /// Drawn faintly while editing, hidden when presenting and exporting.
    /// Editing its text makes it real content.
    #[serde(default, skip_serializing_if = "is_false")]
    pub placeholder: bool,
    /// Grouped elements share an id ("g1"): selecting one selects them all,
    /// so they move, scale and delete together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub kind: ElementKind,
}

fn is_false(b: &bool) -> bool {
    !*b
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
            ElementKind::Video(_) => "video",
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
            ElementKind::Video(_) => "v",
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
    /// A slide imported from PowerPoint: its path step shows deck-style
    /// when presenting, like an imported PDF page ([`Element::is_page`]).
    #[serde(default, skip_serializing_if = "is_false")]
    pub slide: bool,
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
    /// Bold. In a style that is Semibold or heavier, `true` keeps the
    /// style's weight and `false` makes the characters Regular.
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
    /// The visible part of the picture, as fractions of its full size.
    /// None shows the whole picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
    /// Outline the picture is clipped to.
    #[serde(default, skip_serializing_if = "is_default")]
    pub mask: ImageMask,
    /// Set when the picture is a symbol from the clip-art library (Insert ›
    /// Shapes, Icons & Symbols). Its SVG is the asset, so it draws without the
    /// library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<SymbolLook>,
    /// Set when the picture is a page of an imported PDF. `asset` is then
    /// a picture of the page made at import (thumbnails, the web player
    /// and exports draw that); the app draws the page itself from the PDF,
    /// so it stays sharp at any zoom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<PageRef>,
}

/// A page of a PDF kept in the document as an asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageRef {
    /// Asset key of the PDF (extension "pdf").
    pub pdf: String,
    /// 0-based page number.
    pub index: u32,
}

/// Where a symbol came from and how it is drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct SymbolLook {
    /// Library set and name, e.g. "noto" and "rocket" (see the `symbols` query).
    pub set: String,
    pub name: String,
    /// One-colour symbols: their colour (a theme slot follows the theme).
    /// None for symbols drawn in their own colours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRef>,
    /// Two-colour symbols (IconPark): the fill colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color2: Option<ColorRef>,
    /// A white outline and a soft shadow, like a sticker.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sticker: bool,
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
    /// Autoplay: seconds to stay on this step (and each of its builds)
    /// before moving on. Overrides the presentation's `play.autoplay`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dwell: Option<f64>,
    /// Voice-over: an audio asset that plays when this step is reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voiceover: Option<String>,
    /// Overview steps only: hide later steps' fade-ins entirely instead of
    /// showing them dimmed (the default), so the overview isn't missing
    /// content.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hide_builds: bool,
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
    /// A circle frame draws the circle inscribed in its box (shorter side,
    /// centred), so switching frame style never changes the box.
    pub fn is_circle_frame(&self) -> bool {
        matches!(&self.kind, ElementKind::Frame(f) if f.style == FrameStyle::Circle)
    }
    /// Local size of what is drawn: the inscribed square for circle frames,
    /// else the box.
    pub fn visual_size(&self) -> (f64, f64) {
        if self.is_circle_frame() {
            let d = self.size.w.min(self.size.h);
            (d, d)
        } else {
            (self.size.w, self.size.h)
        }
    }
    /// World size of what is drawn (see [`Element::visual_size`]).
    pub fn visual_world_size(&self) -> (f64, f64) {
        let (w, h) = self.visual_size();
        (w * self.transform.scale, h * self.transform.scale)
    }
    /// Axis-aligned world bounds of what is drawn.
    pub fn visual_bounds(&self) -> Rect {
        let (hw, hh) = { let (w, h) = self.visual_size(); (w / 2.0, h / 2.0) };
        Rect::from_points([Vec2::new(-hw, -hh), Vec2::new(hw, -hh), Vec2::new(hw, hh), Vec2::new(-hw, hh)].map(|p| self.transform.apply(p))).unwrap()
    }
    /// Precise hit test against the rotated box (frames: only near their border
    /// unless `interior` is true).
    pub fn hit(&self, p: Vec2, tolerance_world: f64, interior: bool) -> bool {
        let l = self.transform.inverse(p);
        let tol = tolerance_world / self.transform.scale;
        let (hw, hh) = (self.size.w / 2.0, self.size.h / 2.0);
        if self.is_circle_frame() {
            let r = hw.min(hh);
            let d = l.x.hypot(l.y);
            if d > r + tol {
                return false;
            }
            let edge = tol.max(r * 0.08);
            return interior || d >= r - edge;
        }
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
    /// A page of an imported file: a PDF page (a picture with a page
    /// reference) or a PowerPoint slide (a frame marked as a slide).
    pub fn is_page(&self) -> bool {
        match &self.kind {
            ElementKind::Image(i) => i.page.is_some(),
            ElementKind::Frame(f) => f.slide,
            _ => false,
        }
    }
    /// The margin (a fraction of the screen) left around this element when
    /// the camera fits it, as a path step or by zooming to it. An imported
    /// page fills the screen like a slide; frames and pictures nearly fill
    /// it; other things get room around them.
    pub fn fit_margin(&self) -> f64 {
        if self.is_page() {
            0.0
        } else if matches!(self.kind, ElementKind::Frame(_) | ElementKind::Image(_)) {
            0.04
        } else {
            0.12
        }
    }
    /// Short human label: name, else first text line, else type.
    pub fn label(&self, doc: &Document) -> String {
        self.own_label(doc).unwrap_or_else(|| self.kind.type_name().into())
    }

    /// The label from the element's own words (name, first text line, alt
    /// text), or `None` when only its type would describe it. The app
    /// shows a translated type name in that case.
    pub fn own_label(&self, doc: &Document) -> Option<String> {
        self.own_label_in(doc, || doc.frame_contents(&self.id))
    }

    /// [`Element::own_label`], taking a frame's contents from `contents`
    /// (called only when needed), e.g. from the editor's spatial index.
    pub fn own_label_in(&self, doc: &Document, contents: impl FnOnce() -> Vec<ElementId>) -> Option<String> {
        if !self.name.is_empty() {
            return Some(self.name.clone());
        }
        match &self.kind {
            ElementKind::Text(t) => Some(truncate(t.text.lines().next().unwrap_or(""), 40)),
            ElementKind::Frame(_) => contents().iter().filter_map(|id| doc.elements.get(id)).find_map(|e| match &e.kind {
                ElementKind::Text(t) => Some(truncate(t.text.lines().next().unwrap_or(""), 40)),
                _ => None,
            }),
            ElementKind::Image(i) if !i.alt.is_empty() => Some(truncate(&i.alt, 40)),
            ElementKind::Video(v) if !v.alt.is_empty() => Some(truncate(&v.alt, 40)),
            _ => None,
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
    /// The centre nearest `at` where a `w`×`h` box (world units, axis
    /// aligned) overlaps nothing, with a small gap around it. A box wholly
    /// inside a bigger object, such as a background frame, is fine as long
    /// as it overlaps nothing else. Gives `at` back when no space turns up
    /// within a few box sizes.
    pub fn free_spot(&self, at: Vec2, w: f64, h: f64) -> Vec2 {
        if !(w > 0.0 && h > 0.0 && at.x.is_finite() && at.y.is_finite()) {
            return at;
        }
        let gap = w.min(h) * 0.1;
        let boxes: Vec<Rect> = self.ordered().map(|e| e.bounds()).collect();
        let fits = |c: Vec2| {
            let r = Rect { x0: c.x - w / 2.0, y0: c.y - h / 2.0, x1: c.x + w / 2.0, y1: c.y + h / 2.0 }.inflate(gap);
            boxes.iter().all(|o| !o.intersects(&r) || o.contains_rect(&r))
        };
        if fits(at) {
            return at;
        }
        // Rings of a grid around `at`, nearest first; stop once a ring can't
        // beat the best spot found.
        // A quarter of the short side, but never finer than 1/16 of the
        // long one: a very long, thin box would otherwise mean millions
        // of candidates (100000 x 1 froze the app for seconds).
        let step = (w.min(h) / 4.0).max(w.max(h) / 16.0);
        let rings = ((4.0 * w.max(h)) / step).ceil() as i64;
        let mut best: Option<(f64, Vec2)> = None;
        for k in 1..=rings {
            if best.is_some_and(|(d, _)| d < k as f64 * step) {
                break;
            }
            for i in -k..=k {
                for j in -k..=k {
                    if i.abs() != k && j.abs() != k {
                        continue;
                    }
                    let c = Vec2::new(at.x + i as f64 * step, at.y + j as f64 * step);
                    let d = (c - at).len();
                    if best.is_none_or(|(b, _)| d < b) && fits(c) {
                        best = Some((d, c));
                    }
                }
            }
        }
        best.map(|(_, c)| c).unwrap_or(at)
    }

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
    /// Everything a frame holds ([`frame_holds`]), nested frames and their
    /// contents included, back to front: what it carries when the person
    /// drags it, less anything locked ([`crate::Editor::carried`]).
    pub fn frame_contents(&self, frame_id: &str) -> Vec<ElementId> {
        let Some(f) = self.elements.get(frame_id) else { return vec![] };
        let groups = self.group_members();
        self.ordered().filter(|e| frame_holds_with(f, e, |g| groups.get(g).into_iter().flatten().copied())).map(|e| e.id.clone()).collect()
    }
    /// The frame an element belongs to: the smallest frame that holds it
    /// ([`frame_holds`], [`Element::drawn_area`]), then the lowest in the
    /// draw order. Dragging that frame (or any other that holds it) moves
    /// the element. Brute force; [`crate::Editor::parent_frame`] answers
    /// the same from its index.
    pub fn parent_frame(&self, id: &str) -> Option<ElementId> {
        let e = self.elements.get(id)?;
        self.ordered()
            .filter(|f| frame_holds(self, f, e))
            .min_by(|a, b| a.drawn_area().total_cmp(&b.drawn_area()))
            .map(|f| f.id.clone())
    }
    /// Topmost element under a world point.
    pub fn hit_test(&self, p: Vec2, tolerance_world: f64) -> Option<&Element> {
        self.ordered().rev().find(|e| e.hit(p, tolerance_world, false))
    }
    /// A group id no element uses yet ("g7").
    pub fn alloc_group_id(&mut self) -> String {
        loop {
            let id = self.alloc_id("g");
            if !self.elements.values().any(|e| e.group.as_deref() == Some(id.as_str())) {
                return id;
            }
        }
    }
    /// `ids` plus every other member of their groups, in the given order
    /// followed by the added members in z-order.
    pub fn expand_groups(&self, ids: &[ElementId]) -> Vec<ElementId> {
        let groups: std::collections::BTreeSet<&str> =
            ids.iter().filter_map(|i| self.elements.get(i)?.group.as_deref()).collect();
        let mut out = ids.to_vec();
        if groups.is_empty() {
            return out;
        }
        // A set: `out.contains` for every element was O(N x k).
        let mut seen: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        for z in &self.z_order {
            if self.elements[z].group.as_deref().is_some_and(|g| groups.contains(g)) && seen.insert(z.as_str()) {
                out.push(z.clone());
            }
        }
        out
    }
    pub fn alloc_id(&mut self, prefix: &str) -> ElementId {
        loop {
            let id = format!("{prefix}{}", self.next_id);
            self.next_id = self.next_id.wrapping_add(1);
            if !self.elements.contains_key(&id) {
                return id;
            }
        }
    }
    /// Every asset key the document refers to (elements, posters,
    /// soundtrack, voice-overs, and the backgrounds, 3D layers and embedded
    /// fonts of custom themes).
    pub fn used_assets(&self) -> std::collections::BTreeSet<&str> {
        let mut out = std::collections::BTreeSet::new();
        for e in self.elements.values() {
            match &e.kind {
                ElementKind::Image(i) => {
                    out.insert(i.asset.as_str());
                    if let Some(p) = &i.page {
                        out.insert(p.pdf.as_str());
                    }
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
        for t in &self.themes {
            out.extend(t.asset_keys());
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


/// How far (as a fraction of the frame's width plus height) an element may
/// poke past a frame's outline and still be held by it: rounding when an
/// element is snapped to the edge of a rotated or scaled frame.
pub const FRAME_EDGE_TOLERANCE: f64 = 1e-9;

impl Element {
    /// The points [`frame_holds`] judges, in world space: a line or arrow
    /// by its end and bend points (a curve's bend is its middle); a frame
    /// by the box it draws (a circle frame: the square around its circle);
    /// anything else by the four corners of its own turned box (text: its
    /// box, as its selection outline shows it).
    pub fn held_points(&self) -> Vec<Vec2> {
        match &self.kind {
            ElementKind::Connector(c) => {
                let r = crate::connector::local_route(c);
                let local = match &r {
                    crate::connector::RoutePath::Poly(p) => p.clone(),
                    crate::connector::RoutePath::Cubic([a, .., d]) => vec![*a, r.midpoint(), *d],
                };
                local.into_iter().map(|p| self.transform.apply(p)).collect()
            }
            ElementKind::Shape(s) if s.shape.is_line() => {
                let hw = self.size.w / 2.0;
                vec![self.transform.apply(Vec2::new(-hw, 0.0)), self.transform.apply(Vec2::new(hw, 0.0))]
            }
            ElementKind::Frame(_) => {
                let (hw, hh) = { let (w, h) = self.visual_size(); (w / 2.0, h / 2.0) };
                [Vec2::new(-hw, -hh), Vec2::new(hw, -hh), Vec2::new(hw, hh), Vec2::new(-hw, hh)].map(|p| self.transform.apply(p)).to_vec()
            }
            _ => self.corners().to_vec(),
        }
    }

    /// Whether the world point `p` lies inside the outline this frame
    /// draws, give or take `slack` world units: the circle of a circle
    /// frame, the turned rectangle of any other.
    pub fn outline_contains(&self, p: Vec2, slack: f64) -> bool {
        let l = self.transform.inverse(p);
        let tol = slack / self.transform.scale;
        let (w, h) = self.visual_size();
        if self.is_circle_frame() {
            l.x.hypot(l.y) <= w / 2.0 + tol
        } else {
            l.x.abs() <= w / 2.0 + tol && l.y.abs() <= h / 2.0 + tol
        }
    }

    /// The area of what a frame draws, in world units: its circle, or its
    /// box. Which of two frames is smaller.
    pub fn drawn_area(&self) -> f64 {
        let (w, h) = self.visual_world_size();
        if self.is_circle_frame() {
            std::f64::consts::FRAC_PI_4 * w * h
        } else {
            w * h
        }
    }
}

impl Document {
    /// Every group's members, for [`frame_holds_with`] over many elements.
    pub fn group_members(&self) -> std::collections::HashMap<&str, Vec<&Element>> {
        let mut m: std::collections::HashMap<&str, Vec<&Element>> = std::collections::HashMap::new();
        for e in self.elements.values() {
            if let Some(g) = e.group.as_deref() {
                m.entry(g).or_default().push(e);
            }
        }
        m
    }
}

/// THE frame-ownership rule, the one every part of Zebra uses (what a
/// dragged or turned frame carries, the path, describe, the elements
/// query, import, the spatial index): `frame` holds `e` when every point
/// of `e` ([`Element::held_points`]: the corners of its own turned box, a
/// line's end and bend points) lies inside the outline `frame` draws (its
/// turned rectangle, or its circle) and, for a frame, it is smaller (two
/// frames the same size don't hold each other). Anything else exactly the
/// size of its frame, such as a page picture or a slide's background, is
/// held. A point exactly on the outline is inside ([`FRAME_EDGE_TOLERANCE`]).
/// A group counts as a whole: one member outside and the frame holds none
/// of it. Something that crosses the outline is not held and doesn't move
/// with the frame. Locked objects may be held, but never move
/// ([`crate::Editor::carried`]).
pub fn frame_holds(doc: &Document, frame: &Element, e: &Element) -> bool {
    frame_holds_with(frame, e, |g| doc.elements.values().filter(move |m| m.group.as_deref() == Some(g)))
}

/// [`frame_holds`], with the members of `e`'s group from `members` (called
/// only for a grouped element), e.g. from [`Document::group_members`].
pub fn frame_holds_with<'a, I: IntoIterator<Item = &'a Element>>(frame: &Element, e: &'a Element, members: impl FnOnce(&'a str) -> I) -> bool {
    if !frame.is_frame() || frame.id == e.id {
        return false;
    }
    match e.group.as_deref() {
        Some(g) => {
            let mut any = false;
            for m in members(g) {
                if m.id == frame.id || !holds_alone(frame, m) {
                    return false;
                }
                any = true;
            }
            // `members` left `e` out: judge it alone.
            any || holds_alone(frame, e)
        }
        None => holds_alone(frame, e),
    }
}

/// [`frame_holds`] for one element, its group aside.
fn holds_alone(frame: &Element, e: &Element) -> bool {
    if e.is_frame() && e.drawn_area() >= frame.drawn_area() {
        return false;
    }
    let (fw, fh) = frame.visual_world_size();
    let slack = FRAME_EDGE_TOLERANCE * (fw + fh);
    e.held_points().into_iter().all(|p| frame.outline_contains(p, slack))
}
