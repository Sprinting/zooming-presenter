//! Translated names for things zebra-core knows about: history entries,
//! shapes and object types. Core keeps plain English for the agent API;
//! everything a person reads goes through here.

use zebra_core::{Command, Document, Element, ElementKind, Phrase, ShapeKind};

use crate::i18n::tr;
use crate::t;

/// Installed as [`zebra_core::Editor::labeler`].
pub fn phrase(p: Phrase) -> Option<String> {
    Some(match p {
        Phrase::Command(c) => command(c),
        Phrase::Changes(n) => t!("history-changes", count = (n as u64)),
        // Built-in content: filled in once, when it is placed.
        Phrase::Content(key) if crate::i18n::LOADER.has(key) => tr(key),
        Phrase::Content(_) => return None,
    })
}

/// Who made a change, as the Activity panel and Edit menu name them.
pub fn origin_name(o: &zebra_core::Origin) -> String {
    match o {
        zebra_core::Origin::Human => t!("origin-you"),
        zebra_core::Origin::Agent { name } => name.clone(),
    }
}

/// The Fluent key fragment for a serde snake_case name: `rounded_rect` → `rounded-rect`.
fn key_part<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(|s| s.replace('_', "-"))).unwrap_or_default()
}

/// A font weight by its macOS Fonts-window name ("Semibold"), or its
/// number when it isn't one of the usual nine.
pub fn weight_name(w: u16) -> String {
    if zebra_core::theme::weight_name(w).is_some() { tr(&format!("font-weight-{w}")) } else { w.to_string() }
}

/// A face as the Weight pop-up lists it: an installed font's own name for
/// it ("Demi Bold"), else the usual name of its weight.
pub fn face_name(f: &zebra_core::FontFace) -> String {
    f.name.clone().unwrap_or_else(|| weight_name(f.weight))
}

/// The face a style of `weight` is drawn in, by name: the face of that
/// weight if the font has one, else the nearest.
pub fn style_face_name(faces: &[zebra_core::FontFace], weight: u16) -> String {
    let have: Vec<u16> = faces.iter().map(|f| f.weight).collect();
    let w = if have.is_empty() { weight } else { zebra_core::theme::nearest_weight(&have, weight) };
    match faces.iter().find(|f| f.weight == w) {
        Some(f) if f.weight == weight || f.name.is_some() => face_name(f),
        _ => weight_name(weight),
    }
}

pub fn shape_name(k: ShapeKind) -> String {
    tr(&format!("shape-{}", key_part(&k)))
}

pub fn type_name(k: &ElementKind) -> String {
    tr(&format!("element-{}", k.type_name()))
}

/// Short label for an object: its own words, else its translated type.
pub fn element_label(e: &Element, doc: &Document) -> String {
    e.own_label(doc).unwrap_or_else(|| type_name(&e.kind))
}

/// `prefix-<id>` from the ftl file when it has one, else `fallback`.
fn named(prefix: &str, id: &str, fallback: &str) -> String {
    let key = format!("{prefix}-{}", id.replace('_', "-"));
    if crate::i18n::LOADER.has(&key) { tr(&key) } else { fallback.to_string() }
}

/// A theme's name: translated for the built-in themes, a person's own
/// themes keep the name they gave.
pub fn theme_name(t: &zebra_core::Theme) -> String {
    if zebra_core::Theme::builtins().iter().any(|b| b.id == t.id && b.name == t.name) {
        named("theme-name", &t.id, &t.name)
    } else {
        t.name.clone()
    }
}

pub fn template_name(i: &zebra_core::templates::Info) -> String {
    named("template-name", &i.id, &i.name)
}
pub fn template_description(i: &zebra_core::templates::Info) -> String {
    named("template-description", &i.id, &i.description)
}
pub fn layout_name(i: &zebra_core::templates::Info) -> String {
    named("layout-name", &i.id, &i.name)
}
pub fn layout_description(i: &zebra_core::templates::Info) -> String {
    named("layout-description", &i.id, &i.description)
}
pub fn diagram_name(d: &zebra_core::diagrams::Diagram) -> String {
    named("diagram-name", d.key, d.name)
}
pub fn diagram_description(d: &zebra_core::diagrams::Diagram) -> String {
    named("diagram-description", d.key, d.description)
}

/// What a person reads when a command they gave fails: by what the error
/// is about where the code alone would say the wrong thing (`too_big` is
/// a table, a picture or a document), else by its code.
pub fn error(e: &zebra_core::ZError) -> String {
    use zebra_core::error::About;
    match e.about {
        Some(About::Table) => return t!("table-too-big"),
        Some(About::Picture) => return t!("app-error-picture-too-big"),
        Some(About::Poster) => return t!("app-error-poster-too-big"),
        Some(About::Document) => return t!("app-error-document-too-big"),
        Some(About::Saving) => return t!("app-error-saving"),
        None => {}
    }
    match e.code {
        "locked" => t!("app-error-locked"),
        "busy" => t!("app-error-busy"),
        "io" => t!("app-error-io"),
        "not_empty" => t!("app-error-not-empty"),
        "not_found" => t!("app-error-not-found"),
        "too_big" | "too_large" => t!("app-error-too-big"),
        "linked" => t!("app-error-linked"),
        zebra_core::beta::CODE => t!("app-error-beta-off"),
        _ => t!("app-error-other"),
    }
}

fn command(c: &Command) -> String {
    match c {
        Command::AddElement { element } => t!("history-add-element", kind = type_name(&element.kind)),
        Command::AddText { text, .. } => t!("history-add-text", text = zebra_core::model::truncate(text.lines().next().unwrap_or(""), 24)),
        Command::AddFrame { .. } => t!("history-add-frame"),
        Command::AddShape { shape, .. } => t!("history-add-shape", shape = shape_name(*shape)),
        Command::AddImage { .. } => t!("history-insert-image"),
        Command::AddIcon { name, .. } => t!("history-add-icon", name = name.as_str()),
        Command::AddConnector { .. } => t!("history-add-connector"),
        Command::AddVideo { .. } | Command::AddWebVideo { .. } => t!("history-insert-video"),
        Command::DownloadVideo { .. } => t!("history-download-video"),
        Command::ReplaceImage { .. } => t!("history-replace-image"),
        Command::ReplaceVideo { .. } => t!("history-replace-video"),
        Command::SetPoster { .. } => t!("history-set-poster"),
        Command::SetSoundtrack { data_base64, path, asset, .. } if !data_base64.is_empty() || path.is_some() || asset.is_some() => t!("history-add-soundtrack"),
        Command::SetSoundtrack { .. } => t!("history-change-soundtrack"),
        Command::RemoveSoundtrack => t!("history-remove-soundtrack"),
        Command::SetVoiceover { data_base64, path, asset, .. } if data_base64.is_empty() && path.is_none() && asset.is_none() => t!("history-remove-voiceover"),
        Command::SetVoiceover { .. } => t!("history-add-voiceover"),
        Command::Update { patch, .. } if patch.crop.is_some() => t!("history-crop"),
        Command::AddInk { highlighter: true, .. } => t!("history-highlight"),
        Command::AddInk { .. } => t!("history-draw"),
        Command::Update { patch, .. } if patch.text.is_some() => t!("history-edit-text"),
        Command::FormatText { .. } => t!("history-format-text"),
        Command::FormatParagraphs { .. } => t!("history-format-paragraphs"),
        Command::SplitText { copy: true, .. } => t!("history-copy-text"),
        Command::SplitText { .. } => t!("history-move-text"),
        Command::Update { .. } => t!("history-change-properties"),
        Command::Move { .. } => t!("history-move"),
        Command::Scale { .. } | Command::Resize { .. } => t!("history-resize"),
        Command::Rotate { .. } => t!("history-rotate"),
        Command::Delete { ids } => t!("history-delete", count = (ids.len() as u64)),
        Command::Duplicate { .. } => t!("history-duplicate"),
        Command::ZOrder { .. } => t!("history-change-order"),
        Command::AddTable { .. } => t!("history-add-table"),
        Command::AddChart { .. } => t!("history-add-chart"),
        Command::SetChartData { .. } => t!("history-edit-chart-data"),
        Command::FormatChart { format, .. } => {
            // As zebra-core's label: a type or a style alone says so.
            let only = |set: bool| set && *format == zebra_core::command::ChartFormat { chart_type: format.chart_type, look: format.look, ..Default::default() };
            if only(format.chart_type.is_some() && format.look.is_none()) {
                t!("history-change-chart-type")
            } else if only(format.look.is_some() && format.chart_type.is_none()) {
                t!("history-change-chart-style")
            } else {
                t!("history-format-chart")
            }
        }
        Command::LinkChart { .. } => t!("history-link-chart"),
        Command::UnlinkChart { .. } => t!("history-unlink-chart"),
        Command::SetCells { values, .. } if values.len() * values.first().map_or(0, |r| r.len()) <= 1 => t!("history-edit-cell"),
        Command::SetCells { .. } => t!("history-edit-cells"),
        Command::InsertRows { count, .. } => {
            let count = *count;
            t!("history-add-rows", count = count)
        }
        Command::DeleteRows { count, .. } => {
            let count = *count;
            t!("history-delete-rows", count = count)
        }
        Command::InsertColumns { count, .. } => {
            let count = *count;
            t!("history-add-columns", count = count)
        }
        Command::DeleteColumns { count, .. } => {
            let count = *count;
            t!("history-delete-columns", count = count)
        }
        Command::MoveRows { .. } => t!("history-move-rows"),
        Command::MoveColumns { .. } => t!("history-move-columns"),
        Command::MergeCells { .. } => t!("history-merge-cells"),
        Command::UnmergeCells { .. } => t!("history-unmerge-cells"),
        Command::FormatCells { .. } => t!("history-format-cells"),
        Command::SetColumn { .. } => t!("history-change-column"),
        Command::SetRow { .. } => t!("history-change-row"),
        Command::SortTable { .. } => t!("history-sort-table"),
        Command::WrapInFrame { .. } => t!("history-frame-selection"),
        Command::Arrange { .. } => t!("history-arrange"),
        Command::Align { .. } => t!("history-align"),
        Command::Group { .. } => t!("history-group"),
        Command::Ungroup { .. } => t!("history-ungroup"),
        Command::AddStep { .. } => t!("history-add-step"),
        Command::RemoveStep { .. } => t!("history-remove-step"),
        Command::MoveStep { .. } => t!("history-reorder-path"),
        Command::UpdateStep { .. } => t!("history-edit-step"),
        Command::ClearPath { .. } => t!("history-clear-path"),
        Command::AutoPath { .. } => t!("history-path-from-frames"),
        Command::SetTheme { theme } => t!("history-theme", theme = theme.as_str()),
        Command::SetTitle { .. } => t!("history-rename"),
        Command::SetPlayback { .. } => t!("history-change-autoplay"),
        Command::SaveTheme { .. } => t!("history-edit-theme"),
        Command::DeleteTheme { .. } => t!("history-delete-theme"),
        Command::AddAsset { .. } => t!("history-add-picture"),
        Command::ApplyTemplate { template, .. } => {
            t!("history-template", template = zebra_core::templates::TEMPLATES.iter().find(|i| i.id == *template).map(template_name).unwrap_or_else(|| template.clone()))
        }
        Command::AddLayout { layout, .. } => {
            t!("history-insert-layout", layout = zebra_core::templates::LAYOUTS.iter().find(|i| i.id == *layout).map(layout_name).unwrap_or_else(|| layout.clone()))
        }
        Command::AddSymbol { name, .. } => t!("history-insert-symbol", name = name.as_str()),
        Command::AddDiagram { diagram, .. } => t!("history-insert-diagram", diagram = zebra_core::diagrams::get(diagram).map(diagram_name).unwrap_or_else(|| diagram.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_name_has_a_key() {
        for k in ShapeKind::ALL {
            let key = format!("shape-{}", key_part(k));
            assert!(crate::i18n::LOADER.has(&key), "missing {key} in zebra-app.ftl");
        }
        for t in zebra_core::Theme::builtins() {
            assert!(crate::i18n::LOADER.has(&format!("theme-name-{}", t.id.replace('_', "-"))), "missing theme {}", t.id);
        }
        for i in zebra_core::templates::TEMPLATES.iter() {
            assert!(crate::i18n::LOADER.has(&format!("template-name-{}", i.id.replace('_', "-"))), "missing template {}", i.id);
            assert!(crate::i18n::LOADER.has(&format!("template-description-{}", i.id.replace('_', "-"))), "missing template {}", i.id);
        }
        for i in zebra_core::templates::LAYOUTS.iter() {
            assert!(crate::i18n::LOADER.has(&format!("layout-name-{}", i.id.replace('_', "-"))), "missing layout {}", i.id);
            assert!(crate::i18n::LOADER.has(&format!("layout-description-{}", i.id.replace('_', "-"))), "missing layout {}", i.id);
        }
        for t in ["frame", "text", "shape", "image", "video", "ink", "icon", "connector"] {
            assert!(crate::i18n::LOADER.has(&format!("element-{t}")), "missing element-{t}");
        }
    }

    /// The undo name shows the layout's and template's name, not its id.
    #[test]
    fn layout_and_template_undo_names_are_names() {
        let l = command(&Command::AddLayout { layout: "title_bullets".into(), x: 0.0, y: 0.0, width: None, add_to_path: true });
        assert_eq!(l, format!("Insert Layout: {}", tr("layout-name-title-bullets")));
        assert!(!l.contains("title_bullets"), "{l}");
        let t = command(&Command::ApplyTemplate { template: "big_picture".into(), replace: false });
        assert_eq!(t, format!("Apply Template: {}", tr("template-name-big-picture")));
    }

    /// Built-in content gets its words from this .ftl, and core's English
    /// fallback reads the very same file the same way.
    #[test]
    fn content_keys_come_from_the_ftl() {
        let keys = zebra_core::content::keys();
        assert!(keys.len() > 50, "{}", keys.len());
        for k in keys {
            assert!(crate::i18n::LOADER.has(k), "{k}");
            assert_eq!(phrase(Phrase::Content(k)).as_deref(), zebra_core::content::english(k), "{k}");
        }
        assert_eq!(phrase(Phrase::Content("7")), None, "a number has no key and stays as it is");
    }

    #[test]
    fn the_person_is_named_from_the_ftl() {
        assert_eq!(origin_name(&zebra_core::Origin::Human), "You");
        assert_eq!(origin_name(&zebra_core::Origin::agent("Claude")), "Claude");
    }

    #[test]
    fn delete_counts_its_objects() {
        let one = command(&Command::Delete { ids: vec!["a".into()] });
        let two = command(&Command::Delete { ids: vec!["a".into(), "b".into()] });
        assert_eq!(one, "Delete 1 Object");
        assert_eq!(two, "Delete 2 Objects");
    }
}
