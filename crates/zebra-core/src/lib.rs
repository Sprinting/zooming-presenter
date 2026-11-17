//! zebra-core: the single source of truth for Zebra.
//!
//! Everything a human or an agent can do to a presentation goes through
//! [`Editor::apply`] (document edits, undoable) or [`Editor::view`] (session
//! state: camera, selection, mode). The GUI has no private mutation path.
//! Read-only access goes through [`Editor::query`].

pub mod camera;
pub mod command;
pub mod delta;
pub mod describe;
pub mod editor;
pub mod error;
pub mod geom;
pub mod io;
pub mod model;
pub mod theme;
pub mod view;

pub use camera::{Camera, CameraFlight};
pub use command::{Command, ElementPatch, NewElement, Query, QueryResult, ViewCommand, ZOrderOp};
pub use editor::{Editor, HistoryEntry, Origin, Outcome, TextMeasure};
pub use error::{ZError, ZResult};
pub use geom::{Rect, Transform, Vec2};
pub use model::*;
pub use theme::{Theme, TextStyleDef};
pub use view::{Mode, Tool, ViewState};

/// Bumped whenever the on-disk document format changes.
pub const FORMAT_VERSION: u32 = 1;
/// Bumped whenever the command/query/API schema changes incompatibly.
pub const API_VERSION: u32 = 1;
