//! The README's shortcut table is generated from the keymap's defaults.
//! Run with ZEBRA_BLESS=1 to rewrite it after changing a default.
use zebra_app::actions::{fmt_shortcut_for, Action, DEFAULTS};

fn table() -> String {
    let mut order: Vec<Action> = zebra_app::keymap::rebindable();
    for a in DEFAULTS.iter().map(|d| &d.2).chain(&[Action::Cut, Action::Copy, Action::Paste]) {
        if !order.contains(a) {
            order.push(a.clone());
        }
    }
    let mut out = String::from("| Action | Keys |\n|---|---|\n");
    for a in order {
        // ⇧! and ⇧? are the same keys as ⇧1 and ? on some layouts: not shown.
        let alias = |d: &(egui::Modifiers, egui::Key, Action)| d.0.shift && matches!(d.1, egui::Key::Exclamationmark | egui::Key::Questionmark);
        let mut keys: Vec<String> = DEFAULTS.iter().filter(|d| d.2 == a && !alias(d)).map(|d| fmt_shortcut_for(true, d.0, d.1)).collect();
        // The menu bar's own text-editing keys, outside the keymap.
        let cmd = egui::Modifiers::COMMAND;
        match a {
            Action::Cut => keys.push(fmt_shortcut_for(true, cmd, egui::Key::X)),
            Action::Copy => keys.push(fmt_shortcut_for(true, cmd, egui::Key::C)),
            Action::Paste => keys.push(fmt_shortcut_for(true, cmd, egui::Key::V)),
            _ => {}
        }
        if keys.is_empty() {
            continue;
        }
        let label = a.label().replace('|', "\\|");
        out.push_str(&format!("| {label} | {} |\n", keys.join(" or ").replace('|', "\\|")));
    }
    out
}

#[test]
fn readme_shortcut_table_matches_the_defaults() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
    let readme = std::fs::read_to_string(&path).unwrap();
    let (begin, end) = ("<!-- BEGIN SHORTCUTS -->\n", "<!-- END SHORTCUTS -->");
    let i = readme.find(begin).expect("README has the BEGIN SHORTCUTS marker") + begin.len();
    let j = readme.find(end).expect("README has the END SHORTCUTS marker");
    let want = table();
    if std::env::var_os("ZEBRA_BLESS").is_some() {
        std::fs::write(&path, format!("{}{}{}", &readme[..i], want, &readme[j..])).unwrap();
        return;
    }
    assert_eq!(&readme[i..j], want, "README shortcut table is out of date: run with ZEBRA_BLESS=1");
}
