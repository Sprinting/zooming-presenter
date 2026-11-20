//! The command bus vocabulary. Every human gesture and every agent call is
//! one of these values. JSON shape: `{"op": "add_text", ...}`.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::model::*;
use crate::view::{Blank, Mode, Permission, Tool};

/// Document edits. Undoable, recorded in history with their origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    /// Add any element. Most callers prefer the `add_*` helpers.
    AddElement { element: NewElement },
    /// Double-click-to-write: add a text box. `x`,`y` is the centre.
    /// If `scale` is omitted the text is sized to read well at the given
    /// `place` (or at scale 1).
    AddText {
        text: String,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default)]
        style: TextStyle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<Placement>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ElementId>,
        /// Read `text` as Markdown: `**bold**`, `*italic*`, `~~strike~~`,
        /// `[text](url)`, `- ` bullets, `1. ` numbers, nested by indenting.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        markdown: bool,
        /// Type on a shape (like double-clicking it): the text is centred
        /// on the shape, wraps inside it and is grouped with it. `x`,`y`
        /// are then ignored.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<ElementId>,
        /// Text colour: #rrggbb, #rrggbbaa or a theme slot. Default: the
        /// theme's, or on a shape a colour that reads on its fill.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<ColorRef>,
    },
    /// Add a frame. `w`,`h` are world units at scale 1 (default 800x600).
    AddFrame {
        #[serde(default)]
        style: FrameStyle,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default = "d_frame_w")]
        w: f64,
        #[serde(default = "d_frame_h")]
        h: f64,
        #[serde(default)]
        rotation: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ElementId>,
        /// Also append the new frame to the end of the path.
        #[serde(default)]
        add_to_path: bool,
        /// If the frame would overlap other objects at `x`,`y`, put it in
        /// the nearest empty space instead.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        find_space: bool,
    },
    /// Add a shape. An empty `fill` or `stroke` means none.
    AddShape {
        shape: ShapeKind,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default = "d_shape")]
        w: f64,
        #[serde(default = "d_shape")]
        h: f64,
        #[serde(default)]
        rotation: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<ColorRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<ColorRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ElementId>,
    },
    /// Insert an image from bytes (base64). The API also accepts `path`.
    AddImage {
        data_base64: String,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        /// World width; height follows the aspect ratio. Default: pixel width.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
        #[serde(default)]
        alt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ElementId>,
    },
    /// A line icon from the built-in library, centred at x, y. Find names
    /// with the `icons` query.
    AddIcon {
        name: String,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        /// Width and height of the icon's square box.
        #[serde(default = "d_icon")]
        size: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<ColorRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ElementId>,
    },
    /// A connector: a line or arrow from one place to another. An end with
    /// `element` sticks to that element's edge and follows it when it moves;
    /// otherwise it sits at x, y.
    AddConnector {
        from: EndSpec,
        to: EndSpec,
        #[serde(default)]
        route: Route,
        #[serde(default)]
        start: Arrowhead,
        #[serde(default = "d_arrowhead")]
        end: Arrowhead,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<ColorRef>,
        #[serde(default = "d_connector_width")]
        width: f64,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ElementId>,
    },
    /// Insert a video (mp4/mov, H.264 or HEVC). Give `path` (a file on
    /// this computer; it is copied, never read into memory) or `data_base64`.
    /// It shows `poster_base64` (PNG/JPEG) until played.
    AddVideo {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        data_base64: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        /// World width; height follows the aspect ratio.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
        #[serde(default)]
        alt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        poster_base64: Option<String>,
        /// Play by itself when its step is reached while presenting.
        #[serde(default = "yes")]
        autoplay: bool,
        #[serde(default, rename = "loop")]
        looping: bool,
        #[serde(default)]
        muted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ElementId>,
    },
    /// Swap the picture of an image, keeping its place and width. The
    /// height follows the new picture; crop is reset, the mask is kept.
    ReplaceImage {
        id: ElementId,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        data_base64: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// Swap a video's clip for another, keeping its place, width, rotation
    /// and settings (auto-play, loop, mute, description). The trim starts
    /// over, and the poster is the one given, or none.
    ReplaceVideo {
        id: ElementId,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        data_base64: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        poster_base64: Option<String>,
    },
    /// Set a video's poster picture. No data, path or `at` clears it.
    SetPoster {
        id: ElementId,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        data_base64: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// A frame of the video itself, this many seconds from the start of
        /// its file (the inspector's "Use current frame"). Needs a platform
        /// that decodes video (macOS).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<f64>,
    },
    /// Background music for the presentation. With data or a path the track
    /// is replaced; without, only the given settings change.
    SetSoundtrack {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        data_base64: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
        looping: Option<bool>,
        /// 0..1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        volume: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    RemoveSoundtrack,
    /// Voice-over for a path step (plays when the step is reached). No data
    /// and no path removes it.
    SetVoiceover {
        index: usize,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        data_base64: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// Freehand pen / highlighter stroke in world coordinates.
    AddInk {
        points: Vec<[f64; 2]>,
        #[serde(default = "d_ink_color")]
        color: ColorRef,
        #[serde(default = "d_ink_width")]
        width: f64,
        #[serde(default)]
        highlighter: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ElementId>,
    },
    /// Change properties of one element. Only given fields change.
    Update { id: ElementId, patch: ElementPatch },
    /// Format part of a text box: bold, italic, underline, strike, colour,
    /// link or relative size on characters `start..end` (counted in
    /// characters), or on every occurrence of `find`. With neither, the
    /// whole text. Unset fields in `format` are left as they are; `false` or
    /// `""` turns a property off.
    FormatText {
        id: ElementId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        find: Option<String>,
        format: crate::model::SpanFormat,
    },
    /// Paragraph formatting in a text box: list kind (none, bullet,
    /// number), indent level (0-8, or `indent_by` +1/-1) and alignment.
    /// `first`..=`last` are paragraph indices (0-based; default all).
    FormatParagraphs {
        id: ElementId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        first: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        list: Option<crate::model::ListKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        indent: Option<u8>,
        #[serde(default)]
        indent_by: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        align: Option<crate::model::Align>,
    },
    /// Move elements by a world delta. Frames carry their contents.
    Move {
        ids: Vec<ElementId>,
        dx: f64,
        dy: f64,
        #[serde(default = "yes")]
        with_contents: bool,
    },
    /// Scale elements about a world point (default: their common centre).
    Scale {
        ids: Vec<ElementId>,
        factor: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        about: Option<[f64; 2]>,
        #[serde(default = "yes")]
        with_contents: bool,
    },
    /// Set an element's size on the canvas (world units, scale included),
    /// keeping its centre. Pictures, videos, icons and drawings keep their
    /// shape: give `width` or `height`, or both to fit inside that box.
    /// Text takes a `width` (its wrap width); its height follows its words.
    Resize {
        id: ElementId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<f64>,
    },
    /// Rotate elements by degrees (clockwise) about a point.
    Rotate {
        ids: Vec<ElementId>,
        degrees: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        about: Option<[f64; 2]>,
        #[serde(default = "yes")]
        with_contents: bool,
    },
    Delete { ids: Vec<ElementId> },
    /// Copy elements (frames bring their contents). New ids are returned.
    Duplicate {
        ids: Vec<ElementId>,
        #[serde(default = "d_dup")]
        dx: f64,
        #[serde(default = "d_dup")]
        dy: f64,
    },
    ZOrder { ids: Vec<ElementId>, order: ZOrderOp },
    /// Draw a frame around existing elements.
    WrapInFrame {
        ids: Vec<ElementId>,
        #[serde(default)]
        style: FrameStyle,
        /// Padding as a fraction of the content size.
        #[serde(default = "d_pad")]
        padding: f64,
        #[serde(default)]
        add_to_path: bool,
    },
    /// Arrange elements in a row, column, grid or circle.
    Arrange {
        ids: Vec<ElementId>,
        layout: Layout,
        /// Gap in world units; default: 20% of the average size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gap: Option<f64>,
    },
    /// Align elements (snapping helper).
    Align { ids: Vec<ElementId>, edge: AlignEdge },
    /// Group elements (at least two): selecting one selects them all.
    /// Members of existing groups join the new one. Returns the group id.
    Group {
        ids: Vec<ElementId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Dissolve the groups these elements belong to.
    Ungroup { ids: Vec<ElementId> },

    // ---- path ----
    /// Insert a step (default: at the end).
    AddStep {
        target: StepTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        #[serde(default)]
        builds: Vec<ElementId>,
        #[serde(default)]
        notes: String,
    },
    RemoveStep { index: usize },
    MoveStep { from: usize, to: usize },
    UpdateStep {
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<StepTarget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        builds: Option<Vec<ElementId>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notes: Option<String>,
        /// Seconds; negative clears the override.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration: Option<f64>,
        /// Autoplay time on this step, seconds; negative clears it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dwell: Option<f64>,
        /// Overview steps: hide later fade-ins instead of showing them dimmed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hide_builds: Option<bool>,
    },
    /// Remove every step. Refused while a step has speaker notes, unless
    /// `discard_notes`.
    ClearPath {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        discard_notes: bool,
    },
    /// Replace the path with every frame in reading order (top-left to
    /// bottom-right, larger frames first). A frame that was already a step
    /// keeps its notes, builds and timings. Steps that aren't frames are
    /// dropped; refused while one of those has speaker notes, unless
    /// `discard_notes`.
    AutoPath {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        discard_notes: bool,
    },

    // ---- document ----
    /// A built-in theme, one of this document's, or one from the person's
    /// My Themes (see the `themes` query), copied in with its pictures and
    /// fonts.
    SetTheme { theme: String },
    SetTitle { title: String },
    /// Autoplay and loop for the whole presentation. Only given fields change.
    SetPlayback {
        /// Seconds per step and build; 0 or negative turns autoplay off.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        autoplay: Option<f64>,
        #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
        looping: Option<bool>,
    },
    /// Add or replace a custom theme in this document (the theme editor's
    /// Save). Built-in ids can't be replaced: copy one under a new id.
    /// Fonts that aren't bundled are embedded in the file automatically.
    SaveTheme {
        theme: crate::theme::Theme,
        /// Also make it the presentation's theme.
        #[serde(default)]
        apply: bool,
        /// Also save it in the person's My Themes (Save as Theme in the
        /// theme editor), with its pictures and fonts. Needs 'full'.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        to_library: bool,
    },
    /// Remove a custom theme. If it's in use, its base built-in (or classic) takes over.
    DeleteTheme { id: String },
    /// Store an image (base64) for use as a theme background or 3D layer.
    /// The new asset key is returned in `assets`.
    AddAsset { data_base64: String },
    /// Start from a template (see the `templates` query): frames, placeholder
    /// text, a path and a theme. `saved:<name>` is one the person saved (File
    /// > Save as Template); `sample` is File > New from Sample. Needs an
    /// empty canvas unless `replace`.
    ApplyTemplate {
        template: String,
        /// Clear the current canvas and path first.
        #[serde(default)]
        replace: bool,
    },
    /// Insert a layout (see the `templates` query): a frame arrangement with
    /// placeholder text, centred at `x`,`y`, `width` world units wide.
    AddLayout {
        layout: String,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        /// Default 1600.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
        /// Append the layout's frames to the path.
        #[serde(default = "yes")]
        add_to_path: bool,
    },
    /// A symbol from the clip-art library (Insert › Shapes, Icons & Symbols),
    /// centred at x, y. Find sets and names with the `symbols` query. The
    /// drawing is copied into the document.
    AddSymbol {
        set: String,
        name: String,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        /// World width; height follows the symbol's shape. Default 200.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
        /// One-colour and two-colour sets only. Default `theme:body`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<ColorRef>,
        /// Two-colour sets only: the fill colour. Default `theme:accent`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color2: Option<ColorRef>,
        /// White outline and soft shadow.
        #[serde(default, skip_serializing_if = "is_false")]
        sticker: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ElementId>,
    },
    /// Insert a diagram (see the `diagrams` query): frames, arrows and
    /// placeholder text in theme colours, grouped, centred at `x`,`y`,
    /// `width` world units wide. Diagrams made for zooming add their frames
    /// to the path.
    AddDiagram {
        diagram: String,
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        /// Default 1600.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f64>,
    },
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn yes() -> bool {
    true
}
fn d_frame_w() -> f64 {
    800.0
}
fn d_frame_h() -> f64 {
    600.0
}
fn d_shape() -> f64 {
    200.0
}
fn d_arrowhead() -> Arrowhead {
    Arrowhead::Arrow
}
fn d_connector_width() -> f64 {
    3.0
}

/// One end of a new connector: stuck to `element`, or at world x, y.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct EndSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<ElementId>,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

fn d_icon() -> f64 {
    96.0
}
fn d_icon_limit() -> usize {
    50
}
fn d_dup() -> f64 {
    40.0
}
fn d_pad() -> f64 {
    0.15
}
fn d_ink_color() -> ColorRef {
    "theme:accent".into()
}
fn d_ink_width() -> f64 {
    6.0
}

/// Place a new element relative to an existing one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Placement {
    pub relative_to: ElementId,
    pub side: Side,
    /// World units; default 10% of the reference element's size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Above,
    Below,
    Left,
    Right,
    /// Centred inside (for frames), sized to fit comfortably.
    Inside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Row,
    Column,
    Grid,
    Circle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlignEdge {
    Left,
    Right,
    Top,
    Bottom,
    CenterX,
    CenterY,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZOrderOp {
    Forward,
    Backward,
    Front,
    Back,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NewElement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ElementId>,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub rotation: f64,
    /// Local size; defaults depend on the kind (text: measured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    #[serde(default)]
    pub name: String,
    pub kind: ElementKind,
}

fn one() -> f64 {
    1.0
}

/// Partial update. Fields that don't apply to the element's kind are rejected.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ElementPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    /// Local width (text: wrap width).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    // text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Replace all character formatting (see `format_text` for edits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spans: Option<Vec<crate::model::TextSpan>>,
    /// Replace all paragraph formatting (see `format_paragraphs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paras: Option<Vec<crate::model::ParaFormat>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<TextStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bullets: Option<bool>,
    /// Text / frame / ink colour. Empty string resets to the theme default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRef>,
    // shape
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeKind>,
    /// Empty string = no fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<ColorRef>,
    /// Empty string = no stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<ColorRef>,
    /// Shapes: local units. Icons: units on the 24-unit icon grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<Dash>,
    /// Connectors: move or re-attach an end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<EndSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<EndSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<Route>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_head: Option<Arrowhead>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_head: Option<Arrowhead>,
    /// Connectors: text at the middle of the line ("" removes it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Icons: swap to another library icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    // frame
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_style: Option<FrameStyle>,
    // image / video
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    /// Visible part of an image in fractions of the picture; the visible
    /// content stays where it is. `{"x":0,"y":0,"w":1,"h":1}` uncrops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<ImageMask>,
    /// Two-colour symbols: the fill colour (`color` is the outline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color2: Option<ColorRef>,
    /// Symbols: white outline and soft shadow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticker: Option<bool>,
    // video
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoplay: Option<bool>,
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub looping: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    /// Trim start, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    /// Trim end, seconds; negative clears it (play to the end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
}

/// Session commands: camera, selection, tools, presenting. Not undoable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ViewCommand {
    SetCamera {
        camera: Camera,
        #[serde(default = "yes")]
        animate: bool,
    },
    /// Zoom by a factor around a screen point (default: viewport centre).
    Zoom {
        factor: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<[f64; 2]>,
    },
    /// Pan by a screen-space delta in points.
    Pan { dx: f64, dy: f64 },
    /// Fly to fit an element (frames: match their rotation).
    ZoomTo {
        id: ElementId,
        #[serde(default = "yes")]
        animate: bool,
    },
    FitAll {
        #[serde(default = "yes")]
        animate: bool,
    },
    ZoomToSelection,
    Select { ids: Vec<ElementId> },
    AddToSelection { ids: Vec<ElementId> },
    ClearSelection,
    SelectAll,
    SetTool { tool: Tool },
    SetFrameStyle { style: FrameStyle },
    SetMode { mode: Mode },
    /// Enter present mode at a step (default: first step).
    Present {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<usize>,
    },
    /// Next build or step.
    Next,
    Prev,
    GotoStep { step: usize },
    /// Fly to the overview while presenting.
    Overview,
    Blank { blank: Option<Blank> },
    ExitPresent,
    SetViewport { width: f64, height: f64 },
    SetReduceMotion { on: bool },
    FollowAgent { on: bool },
    PauseAgents { paused: bool },
    SetAgentPermission { permission: Permission },
    /// Pause or resume autoplay while presenting.
    PauseAutoplay { paused: bool },
    /// Play a video in place, with its sound (the inspector's Play; a click
    /// while presenting). Needs a window and a platform that plays video.
    Play { id: ElementId },
    /// Pause a playing video.
    Pause { id: ElementId },
    /// Agents: a one-line status the person sees in the status bar
    /// ("Rewriting the intro"). Up to 120 characters; "" clears it.
    SetStatus { text: String },
}

/// Read-only questions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "q", rename_all = "snake_case")]
pub enum Query {
    /// Compact text outline of the whole presentation (start here).
    Describe,
    /// Mode, selection, camera, revision, step, tool, theme...
    State,
    /// Full document JSON (without image bytes).
    Document,
    Element { id: ElementId },
    /// Filter elements. All filters are optional and combined with AND.
    Elements {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        /// Case-insensitive substring of text / name / alt.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// Only elements inside this frame.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        within: Option<ElementId>,
        /// Only elements visible in the human's current view.
        #[serde(default)]
        visible: bool,
    },
    /// Topmost element at a world point.
    HitTest { x: f64, y: f64 },
    /// Topmost element at a screen point of the human's window.
    HitTestScreen { x: f64, y: f64 },
    Path,
    /// Built-in themes, this document's own (`custom`) and the person's My
    /// Themes (`library`; `set_theme` copies one in with its pictures and
    /// fonts).
    Themes,
    /// Search the icon library by name or keyword (e.g. "idea", "arrow
    /// right"). Empty query lists names only.
    Icons {
        #[serde(default)]
        query: String,
        #[serde(default = "d_icon_limit")]
        limit: usize,
    },
    /// The shape kinds with their human names.
    Shapes,
    /// Search the clip-art library (sets, then symbols by name or keyword).
    /// With no `query` and no `set`, lists the sets.
    Symbols {
        #[serde(default)]
        query: String,
        /// Only this set (its key, e.g. "noto").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set: Option<String>,
        #[serde(default = "d_icon_limit")]
        limit: usize,
    },
    /// The diagrams `add_diagram` accepts.
    Diagrams,
    History,
    /// Templates and layouts that `apply_template` / `add_layout` accept,
    /// the person's saved templates (`saved`) and whether `sample` is there.
    Templates,
    /// Font families installed on this machine (any can be used in a theme).
    Fonts,
    /// Memory and speed: heap in use, cache sizes, frame times, repaints.
    Perf,
}

/// Result of a [`Query`].
pub type QueryResult = serde_json::Value;
