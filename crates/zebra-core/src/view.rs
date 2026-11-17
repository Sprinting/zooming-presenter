//! Session (non-document) state: camera, selection, tool, mode, presenting.
//! Not undoable, not saved in the document.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::camera::{Camera, CameraFlight};
use crate::geom::Vec2;
use crate::model::{ElementId, FrameStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Edit,
    /// Path editing overlay: numbered badges; clicking objects appends steps.
    PathEdit,
    /// Full-screen presenting.
    Present,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    #[default]
    Select,
    Hand,
    Text,
    Frame,
    Rectangle,
    Ellipse,
    Triangle,
    Line,
    Arrow,
    Pen,
    Highlighter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Read state and render images only.
    Observe,
    /// Undoable document edits; the agent has its own camera and selection.
    Edit,
    /// Also drive the visible camera, selection, present mode and files.
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct PresentState {
    /// Index into the path. `None` = overview / before the first step.
    pub step: Option<usize>,
    /// How many of the step's builds are visible.
    pub build: usize,
    /// True after the presenter wandered off-path (zoomed/clicked freely).
    pub off_path: bool,
    pub blank: Option<Blank>,
    /// Autoplay is paused by the presenter (it runs only when the
    /// document's `play.autoplay` or a step's `dwell` is set).
    #[serde(default)]
    pub autoplay_paused: bool,
    /// When the current step or build was reached (camera at rest), for
    /// autoplay timing.
    #[serde(default)]
    pub arrived_at: f64,
    #[serde(skip)]
    #[schemars(skip)]
    pub seen: Option<(Option<usize>, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Blank {
    Black,
    White,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ViewState {
    pub camera: Camera,
    /// Logical size of the canvas area in points.
    pub viewport: Vec2,
    #[serde(skip)]
    #[schemars(skip)]
    pub flight: Option<CameraFlight>,
    /// Human selection (accent colour).
    pub selection: Vec<ElementId>,
    /// Agent selection (violet, dashed). Never touches the human's.
    pub agent_selection: Vec<ElementId>,
    /// The agent's private camera, used for its captures unless it has Full.
    pub agent_camera: Camera,
    pub mode: Mode,
    pub tool: Tool,
    pub frame_style: FrameStyle,
    pub present: PresentState,
    pub reduce_motion: bool,
    /// Human camera follows the agent's camera.
    pub follow_agent: bool,
    /// Agents' mutations are refused while paused (Esc / stop button).
    pub agent_paused: bool,
    pub agent_permission: Permission,
    /// Seconds; injected by the host each frame.
    pub now: f64,
    /// Each agent's own one-line status ("Rewriting the intro"), set with
    /// `set_status`. Shown in the status bar; the person can dismiss it.
    #[serde(skip)]
    #[schemars(skip)]
    pub agent_status: std::collections::BTreeMap<String, AgentStatus>,
}

/// An agent's one-line status. Read-only for the person, who can hide it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentStatus {
    pub text: String,
    /// `now` when it was set.
    pub time: f64,
    /// The person hid it; a new status from the agent shows again.
    pub dismissed: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            camera: Camera::default(),
            viewport: Vec2::new(1280.0, 800.0),
            flight: None,
            selection: vec![],
            agent_selection: vec![],
            agent_camera: Camera::default(),
            mode: Mode::Edit,
            tool: Tool::Select,
            frame_style: FrameStyle::Bracket,
            present: PresentState::default(),
            reduce_motion: false,
            follow_agent: false,
            agent_paused: false,
            agent_permission: Permission::Edit,
            now: 0.0,
            agent_status: Default::default(),
        }
    }
}

impl ViewState {
    /// Advance animations to `now`. Returns true while something is animating
    /// (the host should keep repainting).
    pub fn tick(&mut self, now: f64) -> bool {
        self.now = now;
        if let Some(f) = self.flight {
            self.camera = f.sample(now, self.viewport);
            if f.done(now) {
                self.flight = None;
            }
        }
        self.flight.is_some()
    }
    /// Animate (or jump) the visible camera.
    pub fn fly_to(&mut self, to: Camera, animate: bool) {
        if animate {
            self.flight = Some(CameraFlight::new(self.camera, to, self.now, self.viewport, self.reduce_motion));
        } else {
            self.flight = None;
            self.camera = to;
        }
    }
    /// Destination of the current flight, or the camera itself.
    pub fn target_camera(&self) -> Camera {
        self.flight.map(|f| f.to).unwrap_or(self.camera)
    }
}
