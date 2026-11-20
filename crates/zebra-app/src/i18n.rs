//! User-facing text, from Fluent files (`i18n/<lang>/zebra-app.ftl`).
//!
//! Use [`t!`](crate::t) for every string a person sees:
//! `t!("toolbar-play")`, `t!("steps-count", count = n)`. Keys are checked
//! at compile time against the English file, so a typo or a missing
//! string fails the build. Shortcut hints are never written into the text:
//! use [`with_shortcut`] so they always show the live key binding.
use std::sync::LazyLock;

use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::unic_langid::LanguageIdentifier;
use i18n_embed::{DesktopLanguageRequester, LanguageLoader};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "i18n"]
struct Localizations;

pub static LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    load(&loader, chosen_language().as_deref());
    // No Unicode isolation marks around arguments: egui would draw them.
    loader.set_use_isolating(false);
    loader
});

/// The language picked in Settings first, then the Mac's preferred
/// languages in order; English when none of them is available.
fn load(loader: &FluentLanguageLoader, chosen: Option<&str>) {
    let mut requested: Vec<LanguageIdentifier> = chosen.and_then(|c| c.parse().ok()).into_iter().collect();
    requested.extend(DesktopLanguageRequester::requested_languages());
    if i18n_embed::select(loader, &Localizations, &requested).is_err() {
        let _ = loader.load_fallback_language(&Localizations);
    }
}

fn language_file() -> std::path::PathBuf {
    zebra_api::discovery::app_dir().join("language")
}

/// The language picked in Settings › General; `None` follows the Mac.
pub fn chosen_language() -> Option<String> {
    let s = std::fs::read_to_string(language_file()).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Switch every string to `id` now and remember it; `None` goes back to
/// following the Mac.
pub fn set_language(id: Option<&str>) {
    let path = language_file();
    let r = match id {
        Some(id) => path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, id)),
        None => std::fs::remove_file(&path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }),
    };
    if let Err(e) = r {
        eprintln!("zebra: could not save {}: {e}", path.display());
    }
    load(&LOADER, id);
}

/// Languages Zebra ships, as (id, the language's name in itself).
pub fn available() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> =
        LOADER.available_languages(&Localizations).unwrap_or_default().into_iter().map(|l| l.to_string()).map(|id| (language_name(&id), id)).map(|(n, id)| (id, n)).collect();
    v.sort_by(|a, b| a.1.cmp(&b.1));
    v
}

/// A language's name in itself, from `language-name` in its own file.
pub fn language_name(id: &str) -> String {
    Localizations::get(&format!("{id}/zebra-app.ftl"))
        .and_then(|f| {
            let text = String::from_utf8_lossy(&f.data).into_owned();
            text.lines().find_map(|l| l.strip_prefix("language-name").map(|r| r.trim_start().trim_start_matches('=').trim().to_string()))
        })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| id.to_string())
}

/// The shipped language Zebra uses when following the Mac.
pub fn system_language() -> String {
    let have: Vec<LanguageIdentifier> = LOADER.available_languages(&Localizations).unwrap_or_default();
    DesktopLanguageRequester::requested_languages()
        .iter()
        .find_map(|want| have.iter().find(|h| *h == want).or_else(|| have.iter().find(|h| h.language == want.language)))
        .unwrap_or(LOADER.fallback_language())
        .to_string()
}

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

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn the_picker_lists_shipped_languages_by_their_own_names() {
        let v = available();
        // Run with --nocapture on a Mac to see its preferred languages.
        println!("system languages: {:?}", DesktopLanguageRequester::requested_languages());
        assert!(v.iter().any(|(id, name)| id == "en-GB" && name == "English"), "{v:?}");
        assert_eq!(language_name("xx-YY"), "xx-YY");
        assert!(v.iter().any(|(id, _)| *id == system_language()));
    }
}
