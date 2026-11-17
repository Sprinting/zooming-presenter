//! Zebra desktop app (egui + vello). See `README.md` at the repo root.
pub mod actions;
pub mod app;
pub mod canvas;
pub mod cli;
pub mod fallback;
pub mod library;
pub mod files;
pub mod i18n;
pub mod icons;
pub mod inspector;
pub mod keymap;
pub mod inspector_s3;
pub mod shape_library;
pub mod mac;
pub mod media_ui;
pub mod menus;
pub mod panels;
pub mod present;
pub mod presenting;
pub mod sample;
pub mod settings;
pub mod sheets;
pub mod sidebar;
pub mod style;
pub mod theme_editor;
pub mod thumbs;
pub mod widgets;
pub mod words;
pub mod welcome;
#[cfg(target_os = "macos")]
pub mod menu_mac;

pub use app::{AppOptions, ZebraApp};
