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
    },
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
    /// Freehand pen / highlighter stroke in world coordinates.
    AddInk {
        points: Vec<[f64; 2]>,
        #[serde(default = "d_ink_color")]
        color: ColorRef,
        #[serde(default = "d_ink_width")]
        width: f64,
        #[serde(default)]
        highlighter: bool,
    },
    /// Change properties of one element. Only given fields change.
    Update { id: ElementId, patch: ElementPatch },
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
    },
    ClearPath,
    /// Replace the path with every frame in reading order (top-left to
    /// bottom-right, larger frames first).
    AutoPath,

    // ---- document ----
    SetTheme { theme: String },
    SetTitle { title: String },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f64>,
    // frame
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_style: Option<FrameStyle>,
    // image
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
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
    Themes,
    History,
    /// Layout lint: overlaps, text unreadable at its step, empty steps...
    Lint,
}

/// Result of a [`Query`].
pub type QueryResult = serde_json::Value;
