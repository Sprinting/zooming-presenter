//! User-facing text, from Fluent files (`i18n/<lang>/zebra-app.ftl`).
//!
//! Use [`t!`](crate::t) for every string a person sees:
//! `t!("toolbar-play")`, `t!("steps-count", count = n)`. Keys are checked
//! at compile time against the English file, so a typo or a missing
//! string fails the build. Shortcut hints are never written into the text:
//! use [`with_shortcut`] so they always show the live key binding.
use std::sync::LazyLock;

use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::{DesktopLanguageRequester, LanguageLoader};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "i18n"]
struct Localizations;

pub static LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    let requested = DesktopLanguageRequester::requested_languages();
    if i18n_embed::select(&loader, &Localizations, &requested).is_err() {
        let _ = loader.load_fallback_language(&Localizations);
    }
    // No Unicode isolation marks around arguments: egui would draw them.
    loader.set_use_isolating(false);
    loader
});

/// Look up a user-facing string: `t!("key")` or `t!("key", name = value)`.
#[macro_export]
macro_rules! t {
    ($($args:tt)*) => {
        i18n_embed_fl::fl!($crate::i18n::LOADER, $($args)*)
    };
}

/// `text` followed by the action's current key binding, for tooltips:
/// "Add Step  ⇧⌘A". Never write a shortcut into a string by hand.
pub fn with_shortcut(text: impl Into<String>, action: &crate::actions::Action) -> String {
    let text = text.into();
    match action.shortcut_text() {
        Some(k) => format!("{text}  {k}"),
        None => text,
    }
}

/// Look up a key known only at run time (a table of keys, say). Prefer
/// [`t!`](crate::t) wherever the key is a literal: it is checked at compile
/// time, this is not.
pub fn tr(key: &str) -> String {
    LOADER.get(key)
}

/// A single key combination as shown to people ("⇧1", "Esc", "Space"),
/// for keys that are not bound to an [`Action`](crate::actions::Action).
pub fn key_text(m: egui::Modifiers, k: egui::Key) -> String {
    crate::actions::fmt_shortcut(m, k)
}

/// A modifier on its own, as shown to people: "⌘", "⌥", "⇧" on the Mac,
/// "Ctrl", "Alt", "Shift" elsewhere. For text such as "⌘-scroll zooms".
pub fn modifier_text(m: egui::Modifiers) -> String {
    let s = crate::actions::fmt_shortcut(m, egui::Key::A);
    let s = s.strip_suffix('A').unwrap_or(&s);
    s.strip_suffix('+').unwrap_or(s).to_string()
}

/// `text` followed by a key that is not an action's binding: "Finish
/// editing the path  Esc".
pub fn with_key(text: impl Into<String>, m: egui::Modifiers, k: egui::Key) -> String {
    format!("{}  {}", text.into(), key_text(m, k))
}
