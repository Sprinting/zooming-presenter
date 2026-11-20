//! Text that built-in diagrams, layouts and templates put on the canvas.
//!
//! Core never types that text in English: a [`Piece`](crate::templates::Piece)
//! carries a key (`content-add-a-title`, `content-frame-title`), and the
//! editor fills it in when the piece is placed, through
//! [`Editor::labeler`](crate::Editor::labeler) ([`Phrase::Content`](crate::Phrase::Content)),
//! so the app can translate it. Without a labeler (tests, tools) the English
//! comes from the app's own English Fluent file, so there is one source for
//! the words. Once placed, the text belongs to the person and is never
//! translated again.
use std::collections::HashMap;
use std::sync::LazyLock;

/// The app's English strings (the source language).
const ENGLISH_FTL: &str = include_str!("../../zebra-app/i18n/en-GB/zebra-app.ftl");

/// Every key built-in content may use starts with this.
pub const PREFIX: &str = "content-";

static ENGLISH: LazyLock<HashMap<String, String>> = LazyLock::new(|| parse(ENGLISH_FTL));

/// The `content-*` messages of a Fluent file. Only the plain form the
/// content strings use: `key = text` on one line, or `key =` followed by
/// indented lines, which Fluent joins with newlines.
fn parse(ftl: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut cur: Option<(String, Vec<String>)> = None;
    let mut flush = |cur: &mut Option<(String, Vec<String>)>| {
        if let Some((k, lines)) = cur.take() {
            out.insert(k, lines.join("\n"));
        }
    };
    for line in ftl.lines() {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented && !line.trim().is_empty() {
            if let Some((_, lines)) = &mut cur {
                lines.push(line.trim().to_string());
            }
            continue;
        }
        flush(&mut cur);
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            if k.starts_with(PREFIX) {
                let v = v.trim();
                cur = Some((k.to_string(), if v.is_empty() { vec![] } else { vec![v.to_string()] }));
            }
        }
    }
    flush(&mut cur);
    out
}

/// The English for a content key, or None if there is no such key.
pub fn english(key: &str) -> Option<&'static str> {
    ENGLISH.get(key).map(String::as_str)
}

/// Every content key, sorted.
pub fn keys() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = ENGLISH.keys().map(String::as_str).collect();
    v.sort_unstable();
    v
}

/// Whether built-in content may carry `s` as it is: no letters to
/// translate (a number), or nothing at all.
pub fn is_neutral(s: &str) -> bool {
    !s.chars().any(char::is_alphabetic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_one_line_and_indented_messages() {
        let m = parse("# c\ncontent-a = Add a title\nother = x\ncontent-b =\n    One\n    Two\n\ncontent-c = Once upon a time…\n");
        assert_eq!(m["content-a"], "Add a title");
        assert_eq!(m["content-b"], "One\nTwo");
        assert_eq!(m["content-c"], "Once upon a time…");
        assert!(!m.contains_key("other"));
    }

    #[test]
    fn the_english_file_has_the_content_keys() {
        assert_eq!(english("content-add-a-title"), Some("Add a title"));
        assert_eq!(english("content-add-your-points"), Some("Add your points\nOne point per line\nKeep each point short"));
        assert_eq!(english("content-frame-title"), Some("Title"));
    }
}
