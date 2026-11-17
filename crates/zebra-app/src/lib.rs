//! Zebra desktop app (egui + vello). See `README.md` at the repo root.
pub mod actions;
pub mod app;
pub mod canvas;
pub mod cli;
pub mod fallback;
pub mod files;
pub mod icons;
pub mod inspector;
pub mod mac;
pub mod menus;
pub mod panels;
pub mod present;
pub mod sample;
pub mod settings;
pub mod sheets;
pub mod sidebar;
pub mod style;
pub mod thumbs;
pub mod widgets;
#[cfg(target_os = "macos")]
pub mod menu_mac;

pub use app::{AppOptions, ZebraApp};
