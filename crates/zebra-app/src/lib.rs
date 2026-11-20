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
pub mod import_ui;
pub mod inspector;
pub mod keep_awake;
pub mod keymap;
pub mod keys_pane;
pub mod inspector_s3;
pub mod shape_library;
pub mod symbols_ui;
pub mod credits;
pub mod mac;
pub mod media_ui;
pub mod overlay;
pub mod menus;
pub mod open_events;
pub mod opening;
pub mod panels;
pub mod pickers;
pub mod present;
pub mod presenting;
pub mod sample;
pub mod settings;
pub mod shell;
pub mod sheets;
pub mod sidebar;
pub mod spell;
pub mod style;
pub mod text_drag;
pub mod theme_editor;
pub mod thumbs;
pub mod widgets;
pub mod words;
pub mod welcome;
pub mod window_frames;
#[cfg(target_os = "macos")]
pub mod menu_mac;

pub use app::{AppOptions, ZebraApp};

/// How a document window looks: unified title bar on the Mac (the content
/// runs under a transparent title bar, so the traffic lights and title sit
/// in our 28 pt title row above the toolbar, panels::TITLE_ROW).
pub fn window_builder(title: &str) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id(crate::app::APP_ID)
        .with_min_inner_size([760.0, 480.0])
        .with_drag_and_drop(true)
        .with_fullsize_content_view(cfg!(target_os = "macos"))
        .with_titlebar_shown(!cfg!(target_os = "macos"))
}
