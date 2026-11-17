//! Every word a person reads comes from the Fluent file
//! (`i18n/en-US/zebra-app.ftl`), and every shortcut shown comes from the
//! keymap. This test reads the app's source and fails on a string literal
//! that reaches the screen any other way.
//!
//! Two checks, both on the code above each file's `#[cfg(test)]`:
//! 1. A literal with a letter or a key symbol passed straight to a widget
//!    or tooltip call (`label(`, `on_hover_text(`, `menu_item(`, ...).
//! 2. Anywhere in the file, a literal that reads like an English phrase
//!    ("Two words", capitalised), outside log lines and error plumbing.
//!
//! Files that hold sample content (the sample deck and media demo, which
//! will move to data files) are listed in `SAMPLE_CONTENT`.

use std::path::Path;

/// Calls whose string arguments are shown to people.
const UI_CALLS: &[&str] = &[
    "label",
    "caption",
    "toast",
    "button",
    "push_button",
    "push_button_ext",
    "small_button",
    "menu_item",
    "menu_check",
    "menu_row",
    "menu_icon_row",
    "toolbar_item",
    "checkbox",
    "switch",
    "radio",
    "radio_value",
    "selectable_label",
    "selectable_value",
    "heading",
    "on_hover_text",
    "on_disabled_hover_text",
    "add_filter",
    "layout_no_wrap",
    "labeled",
    "with_shortcut",
    "with_key",
    "hint_text",
];

/// Lines that are for the terminal or for code, not for people.
const NOT_SHOWN: &[&str] = &["eprintln!", "println!", "panic!", "expect(", "unreachable!", "debug_assert", "assert", "anyhow!", "bail!", "log::", "tracing::", "Error::new(", "context("];

/// Sample content, not interface text.
/// Calls that compare or search text: their literals are never shown.
const TEXT_MATCHING: &[&str] = &["contains", "starts_with", "ends_with", "strip_prefix", "strip_suffix", "find", "rfind", "split", "replace", "eq"];

const SAMPLE_CONTENT: &[&str] = &["sample.rs"];

/// Words that are names, not copy: product, font families, folders.
const NAMES: &[&str] = &["Zebra", "JetBrains Mono"];

const KEY_SYMBOLS: &[char] = &['⌘', '⇧', '⌥', '⌃', '⌫', '↵', '⇥'];

struct Lit {
    start: usize,
    text: String,
}

/// String literals on one line, with where each starts. Char literals are
/// skipped; a `//` outside a string ends the line.
fn literals(line: &str) -> Vec<Lit> {
    let b: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '/' if b.get(i + 1) == Some(&'/') => break,
            '\'' => {
                // 'x' or '\x' or a lifetime: skip a char literal if it closes soon.
                if b.get(i + 1) == Some(&'\\') && b.get(i + 3) == Some(&'\'') {
                    i += 4;
                } else if b.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            '"' => {
                let start = i;
                let mut s = String::new();
                i += 1;
                while i < b.len() && b[i] != '"' {
                    // Escapes (\n, \") are not words: drop them.
                    if b[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    s.push(b[i]);
                    i += 1;
                }
                i += 1;
                let start = b[..start].iter().map(|c| c.len_utf8()).sum();
                out.push(Lit { start, text: s });
            }
            _ => i += 1,
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The innermost call still open at byte `pos`: its name, if any.
fn open_call(line: &str, pos: usize) -> Option<&str> {
    let head = &line[..pos];
    let mut depth = 0i32;
    let mut lit = false;
    for (i, c) in head.char_indices().rev() {
        if c == '"' {
            lit = !lit;
            continue;
        }
        if lit {
            continue;
        }
        match c {
            ')' | ']' => depth += 1,
            '(' | '[' if depth > 0 => depth -= 1,
            // An array such as a file dialog's extensions: not an argument.
            '[' => return Some("["),
            '(' => {
                let name_end = i;
                let name_start = head[..name_end].char_indices().rev().take_while(|(_, c)| is_ident(*c) || *c == '!').last().map(|(j, _)| j).unwrap_or(name_end);
                return Some(&head[name_start..name_end]);
            }
            _ => {}
        }
    }
    None
}

/// Does `line` call one of `UI_CALLS` with `pos` inside its arguments
/// (directly or nested in `format!`, `Some(`, `if` arms and the like)?
fn in_ui_call(line: &str, pos: usize) -> bool {
    let mut p = pos;
    loop {
        match open_call(line, p) {
            None => return false,
            Some(name) if is_translation(name) || name == "[" => return false,
            Some(name) if UI_CALLS.contains(&name) => return true,
            Some(name) => {
                // Step out of this call and keep looking outwards.
                let at = line[..p].rfind(&format!("{name}(")).unwrap_or(0);
                if at == 0 {
                    return false;
                }
                p = at;
            }
        }
    }
}

/// `t!(` / `fl!(` / `tr(`: the literal is a Fluent key or argument. The
/// name is only the last path segment, so `crate::t!` arrives as `t!`.
fn is_translation(name: &str) -> bool {
    matches!(name, "t!" | "fl!" | "tr")
}

/// The literal without `format!` placeholders such as `{name}`.
fn without_placeholders(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn shown_text(s: &str) -> bool {
    without_placeholders(s).chars().any(|c| c.is_alphabetic() || KEY_SYMBOLS.contains(&c))
}

fn english_phrase(s: &str) -> bool {
    if NAMES.contains(&s) {
        return false;
    }
    let mut words = s.split_whitespace();
    let first = words.next().unwrap_or("");
    let capital = first.chars().next().is_some_and(|c| c.is_uppercase()) && first.chars().skip(1).all(|c| c.is_lowercase() || c == '\'');
    capital && first.len() > 1 && words.any(|w| w.chars().all(|c| c.is_alphabetic() || ",.?!…'".contains(c)) && w.chars().any(|c| c.is_lowercase()))
}

fn scan(file: &str, src: &str) -> Vec<String> {
    let code = src.split("#[cfg(test)]").next().unwrap_or(src);
    let mut bad = Vec::new();
    let sample = SAMPLE_CONTENT.contains(&file);
    for (n, line) in code.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("#[") || NOT_SHOWN.iter().any(|w| line.contains(w)) {
            continue;
        }
        for l in literals(line) {
            if NAMES.contains(&l.text.as_str()) {
                continue;
            }
            let ui = shown_text(&l.text) && in_ui_call(line, l.start);
            let call = open_call(line, l.start);
            if call.is_some_and(|c| TEXT_MATCHING.contains(&c)) {
                continue;
            }
            let phrase = !sample && english_phrase(&l.text) && call.is_none_or(|c| !is_translation(c));
            if ui || phrase {
                bad.push(format!("{file}:{}: {:?}", n + 1, l.text));
            }
        }
    }
    bad
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_interface_text_outside_the_fluent_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    files.sort();
    let mut bad = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        bad.extend(scan(&rel, &std::fs::read_to_string(f).unwrap()));
    }
    assert!(bad.is_empty(), "Interface text written into the code. Add a key to i18n/en-US/zebra-app.ftl and use crate::t!, or Action::shortcut_text / i18n::key_text for keys:\n{}", bad.join("\n"));
}

#[test]
fn the_check_catches_what_it_should() {
    let bad = |s: &str| !scan("x.rs", s).is_empty();
    assert!(bad(r#"ui.label("Hello");"#));
    assert!(bad(r#"x.on_hover_text("⌘J")"#));
    assert!(bad(r#"w::menu_check(ui, on, &crate::t!("a"), Some("P"))"#));
    assert!(bad(r#"x.on_hover_text(format!("Close {k}"))"#));
    assert!(bad(r#"let tip = "Show the inspector";"#));
    assert!(!bad(r#"ui.label(crate::t!("hello"));"#));
    assert!(!bad(r#"egui::Panel::top("menubar")"#));
    assert!(!bad(r#"eprintln!("zebra: Could not open {p}");"#));
    assert!(!bad(r#"x.on_hover_text(crate::t!("a", name = "b"))"#));
    assert!(!bad("// ui.label(\"Hello world\")"));
    assert!(!bad(r#"_ if e.message.contains("No such file") => x,"#));
    assert!(!bad(r#"d.add_filter(crate::t!("f"), &["zebra"])"#));
    assert!(bad(r#"d.add_filter("Video", &["mp4"])"#));
}
