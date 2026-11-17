//! Zebra desktop app (egui + vello). See `README.md` at the repo root.
pub mod actions;
pub mod app;
pub mod canvas;
pub mod cli;
pub mod fallback;
pub mod files;
pub mod library;
pub mod menus;
pub mod panels;
pub mod present;
pub mod sample;
pub mod sidebar;
pub mod style;
pub mod thumbs;
#[cfg(target_os = "macos")]
pub mod menu_mac;

pub use app::{AppOptions, ZebraApp};
