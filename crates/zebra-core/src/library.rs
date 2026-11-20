//! The person's own themes and templates (My Themes, My Templates) and the
//! sample presentation, as the host keeps them. Core only asks: the host
//! installs a [`Library`] (zebra-api's file library; the app adds the
//! sample, whose words are in its language files). Without one, only the
//! built-ins and the document's own themes exist.
use std::sync::{Arc, RwLock};

use crate::{Command, Document, Theme};

/// What `apply_template` takes for a saved template: `saved:<name>`.
pub const SAVED_PREFIX: &str = "saved:";
/// What `apply_template` takes for File > New from Sample.
pub const SAMPLE: &str = "sample";

pub trait Library: Send + Sync {
    /// Saved themes: the file's document (it holds the theme's pictures and
    /// fonts) and the theme.
    fn themes(&self) -> Vec<(Document, Theme)>;
    /// Saved templates' names.
    fn templates(&self) -> Vec<String>;
    /// A saved template by name.
    fn template(&self, name: &str) -> Option<Document>;
    /// The sample presentation's commands, in the person's language.
    fn sample(&self) -> Option<Vec<Command>> {
        None
    }
}

static LIBRARY: RwLock<Option<Arc<dyn Library>>> = RwLock::new(None);

/// Install the host's library (replaces any earlier one).
pub fn set(l: Arc<dyn Library>) {
    *LIBRARY.write().unwrap_or_else(|e| e.into_inner()) = Some(l);
}

/// The host's library, if it installed one.
pub fn get() -> Option<Arc<dyn Library>> {
    LIBRARY.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A saved theme by id.
pub fn theme(id: &str) -> Option<(Document, Theme)> {
    get()?.themes().into_iter().find(|(_, t)| t.id == id)
}
