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
    })
}

/// The Fluent key fragment for a serde snake_case name: `rounded_rect` → `rounded-rect`.
fn key_part<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(|s| s.replace('_', "-"))).unwrap_or_default()
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

fn command(c: &Command) -> String {
    match c {
        Command::AddElement { element } => t!("history-add-element", kind = type_name(&element.kind)),
        Command::AddText { text, .. } => t!("history-add-text", text = zebra_core::model::truncate(text.lines().next().unwrap_or(""), 24)),
        Command::AddFrame { .. } => t!("history-add-frame"),
        Command::AddShape { shape, .. } => t!("history-add-shape", shape = shape_name(*shape)),
        Command::AddImage { .. } => t!("history-insert-image"),
        Command::AddIcon { name, .. } => t!("history-add-icon", name = name.as_str()),
        Command::AddConnector { .. } => t!("history-add-connector"),
        Command::AddVideo { .. } => t!("history-insert-video"),
        Command::ReplaceImage { .. } => t!("history-replace-image"),
        Command::SetPoster { .. } => t!("history-set-poster"),
        Command::SetSoundtrack { data_base64, path, .. } if !data_base64.is_empty() || path.is_some() => t!("history-add-soundtrack"),
        Command::SetSoundtrack { .. } => t!("history-change-soundtrack"),
        Command::RemoveSoundtrack => t!("history-remove-soundtrack"),
        Command::SetVoiceover { data_base64, path, .. } if data_base64.is_empty() && path.is_none() => t!("history-remove-voiceover"),
        Command::SetVoiceover { .. } => t!("history-add-voiceover"),
        Command::Update { patch, .. } if patch.crop.is_some() => t!("history-crop"),
        Command::AddInk { highlighter: true, .. } => t!("history-highlight"),
        Command::AddInk { .. } => t!("history-draw"),
        Command::Update { patch, .. } if patch.text.is_some() => t!("history-edit-text"),
        Command::FormatText { .. } => t!("history-format-text"),
        Command::FormatParagraphs { .. } => t!("history-format-paragraphs"),
        Command::Update { .. } => t!("history-change-properties"),
        Command::Move { .. } => t!("history-move"),
        Command::Scale { .. } => t!("history-resize"),
        Command::Rotate { .. } => t!("history-rotate"),
        Command::Delete { ids } => t!("history-delete", count = (ids.len() as u64)),
        Command::Duplicate { .. } => t!("history-duplicate"),
        Command::ZOrder { .. } => t!("history-change-order"),
        Command::WrapInFrame { .. } => t!("history-frame-selection"),
        Command::Arrange { .. } => t!("history-arrange"),
        Command::Align { .. } => t!("history-align"),
        Command::Group { .. } => t!("history-group"),
        Command::Ungroup { .. } => t!("history-ungroup"),
        Command::AddStep { .. } => t!("history-add-step"),
        Command::RemoveStep { .. } => t!("history-remove-step"),
        Command::MoveStep { .. } => t!("history-reorder-path"),
        Command::UpdateStep { .. } => t!("history-edit-step"),
        Command::ClearPath => t!("history-clear-path"),
        Command::AutoPath => t!("history-path-from-frames"),
        Command::SetTheme { theme } => t!("history-theme", theme = theme.as_str()),
        Command::SetTitle { .. } => t!("history-rename"),
        Command::SetPlayback { .. } => t!("history-change-autoplay"),
        Command::SaveTheme { .. } => t!("history-edit-theme"),
        Command::DeleteTheme { .. } => t!("history-delete-theme"),
        Command::AddAsset { .. } => t!("history-add-picture"),
        Command::ApplyTemplate { template, .. } => t!("history-template", template = template.as_str()),
        Command::AddLayout { layout, .. } => t!("history-insert-layout", layout = layout.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_and_type_has_a_name() {
        for k in ShapeKind::ALL {
            let key = format!("shape-{}", key_part(k));
            assert!(crate::i18n::LOADER.has(&key), "missing {key} in zebra-app.ftl");
        }
        for t in ["frame", "text", "shape", "image", "video", "ink", "icon", "connector"] {
            assert!(crate::i18n::LOADER.has(&format!("element-{t}")), "missing element-{t}");
        }
    }

    #[test]
    fn delete_counts_its_objects() {
        let one = command(&Command::Delete { ids: vec!["a".into()] });
        let two = command(&Command::Delete { ids: vec!["a".into(), "b".into()] });
        assert_eq!(one, "Delete 1 object");
        assert_eq!(two, "Delete 2 objects");
    }
}
