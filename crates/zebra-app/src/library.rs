//! The person's own themes and templates. The files live in
//! [`zebra_api::library`] (agents reach them too); this adds the person's
//! language for a file with no name.
use std::path::{Path, PathBuf};

use zebra_core::{Document, Theme, ZResult};

pub use zebra_api::library::{list_templates, list_themes, templates_dir, themes_dir};

/// Save a theme (with its pictures and fonts) as a theme file.
pub fn save_theme(from: &Document, theme: &Theme, dir: &Path) -> ZResult<PathBuf> {
    zebra_api::library::save_theme(from, theme, dir, &crate::t!("library-file-untitled"))
}

/// Save the current presentation as a template.
pub fn save_template(doc: &Document, name: &str, dir: &Path) -> ZResult<PathBuf> {
    zebra_api::library::save_template(doc, name, dir, &crate::t!("library-file-untitled"))
}
