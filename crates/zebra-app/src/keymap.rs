//! The live keymap: the default shortcuts ([`DEFAULTS`]) with the
//! person's own changes on top, made in Settings › Keyboard and kept in
//! `keymap.json` in Zebra's data folder.
//!
//! Every shortcut Zebra shows (menus, tooltips, the shortcuts sheet, the
//! ⌘K palette) is read from here, so a change shows everywhere at once.
//! Text-editing keys that every Mac app shares (⌘Z, ⌘C, ⌘B…) and ⌘Q are
//! fixed: see [`rebindable`].

use std::collections::BTreeMap;
use std::sync::{LazyLock, RwLock};

use egui::{Key, Modifiers};

use crate::actions::{Action, DEFAULTS};

pub type Binding = (Modifiers, Key, Action);

/// Settings › Keyboard is waiting for the keys for `action`.
#[derive(Debug, Clone)]
pub struct Capture {
    pub action: Action,
    /// Keys pressed (set by the pane, or by the Mac menu bar for keys it owns).
    pub got: Option<(Modifiers, Key)>,
}

/// The person's changes: action id → its keys (empty = no shortcut).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Overrides {
    #[serde(default)]
    pub keys: BTreeMap<String, Vec<String>>,
}

struct State {
    overrides: Overrides,
    bindings: Vec<Binding>,
}

static STATE: LazyLock<RwLock<State>> = LazyLock::new(|| RwLock::new(State { overrides: Overrides::default(), bindings: DEFAULTS.to_vec() }));

/// Run `f` on the current bindings (most specific first).
pub fn with<R>(f: impl FnOnce(&[Binding]) -> R) -> R {
    f(&STATE.read().unwrap_or_else(|e| e.into_inner()).bindings)
}

/// The first binding for `a`, if it has one.
pub fn binding_for(a: &Action) -> Option<(Modifiers, Key)> {
    with(|b| b.iter().find(|x| &x.2 == a).map(|x| (x.0, x.1)))
}

/// Every binding for `a` (an action can have several, e.g. ⌫ and Del).
pub fn bindings_for(a: &Action) -> Vec<(Modifiers, Key)> {
    with(|b| b.iter().filter(|x| &x.2 == a).map(|x| (x.0, x.1)).collect())
}

/// The action a key press triggers.
pub fn action_for(m: Modifiers, k: Key) -> Option<Action> {
    with(|b| b.iter().find(|x| x.1 == k && crate::actions::mods_match(m, x.0)).map(|x| x.2.clone()))
}

/// A stable name for an action in `keymap.json`.
pub fn id(a: &Action) -> String {
    format!("{a:?}")
}

/// Actions a person can rebind, in the order Settings lists them.
pub fn rebindable() -> Vec<Action> {
    use Action::*;
    // Keys every Mac text field shares, and Quit: fixed.
    let fixed = |a: &Action| matches!(a, Undo | Redo | Cut | Copy | Paste | SelectAll | Bold | Italic | Underline | Quit | SetTheme(_));
    let mut v: Vec<Action> = Action::palette().into_iter().filter(|a| !fixed(a)).collect();
    for (_, _, a) in DEFAULTS {
        if !fixed(a) && !v.contains(a) {
            v.push(a.clone());
        }
    }
    v
}

fn compute(o: &Overrides) -> Vec<Binding> {
    let changed: Vec<Action> = rebindable().into_iter().filter(|a| o.keys.contains_key(&id(a))).collect();
    let mut out: Vec<Binding> = vec![];
    for a in &changed {
        for s in &o.keys[&id(a)] {
            if let Some((m, k)) = parse(s) {
                out.push((m, k, a.clone()));
            }
        }
    }
    // Defaults the person didn't change, minus keys they gave to something else.
    for d in DEFAULTS {
        if changed.contains(&d.2) || out.iter().any(|b| b.1 == d.1 && same_mods(b.0, d.0)) {
            continue;
        }
        out.push(d.clone());
    }
    // Most specific first (more modifiers win when matching).
    out.sort_by_key(|b| std::cmp::Reverse(count(b.0)));
    out
}

fn count(m: Modifiers) -> u8 {
    m.shift as u8 + m.alt as u8 + m.ctrl as u8 + (m.command || m.mac_cmd) as u8
}

fn same_mods(a: Modifiers, b: Modifiers) -> bool {
    a.shift == b.shift && a.alt == b.alt && a.ctrl == b.ctrl && (a.command || a.mac_cmd) == (b.command || b.mac_cmd)
}

/// "cmd+shift+A" (egui key names; `cmd` is ⌘ on the Mac, Ctrl elsewhere).
pub fn format(m: Modifiers, k: Key) -> String {
    let mut s = String::new();
    if m.command || m.mac_cmd {
        s.push_str("cmd+");
    }
    if m.ctrl {
        s.push_str("ctrl+");
    }
    if m.alt {
        s.push_str("alt+");
    }
    if m.shift {
        s.push_str("shift+");
    }
    s.push_str(k.name());
    s
}

pub fn parse(s: &str) -> Option<(Modifiers, Key)> {
    let mut m = Modifiers::NONE;
    let mut parts: Vec<&str> = s.split('+').collect();
    // "cmd++" would split into an empty last part: the key is "+".
    let key = if s.ends_with("++") {
        parts.truncate(parts.len() - 2);
        "+"
    } else {
        parts.pop()?
    };
    for p in parts {
        match p {
            "cmd" => m.command = true,
            "ctrl" => m.ctrl = true,
            "alt" => m.alt = true,
            "shift" => m.shift = true,
            _ => return None,
        }
    }
    if cfg!(target_os = "macos") && m.command {
        m.mac_cmd = true;
    }
    let k = Key::from_name(key)?;
    // ⌘ or ⇧ on its own is never a shortcut (an older build could save one).
    if is_modifier(k) {
        return None;
    }
    Some((m, k))
}

/// ⇧, ⌃, ⌥ or ⌘ itself, which egui also reports as a key press.
pub fn is_modifier(k: Key) -> bool {
    matches!(k, Key::ShiftLeft | Key::ShiftRight | Key::ControlLeft | Key::ControlRight | Key::AltLeft | Key::AltRight | Key::SuperLeft | Key::SuperRight)
}

fn file() -> std::path::PathBuf {
    zebra_api::discovery::app_dir().join("keymap.json")
}

/// Read the person's keymap at startup. A missing or broken file means
/// the defaults (a broken file is reported, never overwritten silently).
pub fn load() {
    let o = match std::fs::read_to_string(file()) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| {
            eprintln!("zebra: ignoring {}: {e}", file().display());
            Overrides::default()
        }),
        Err(_) => Overrides::default(),
    };
    let mut fixed = o.clone();
    drop_bad_keys(&mut fixed);
    if fixed != o {
        save(&fixed);
    }
    install(fixed);
}

/// Forget saved keys that can't be shortcuts (a bare ⌘ from an older
/// build): an action left with none of its own goes back to its default.
fn drop_bad_keys(o: &mut Overrides) {
    o.keys.retain(|_, v| {
        let had = !v.is_empty();
        v.retain(|s| parse(s).is_some());
        !(had && v.is_empty())
    });
}

/// Use `o` from now on (no saving).
pub fn install(o: Overrides) {
    let bindings = compute(&o);
    let mut s = STATE.write().unwrap_or_else(|e| e.into_inner());
    *s = State { overrides: o, bindings };
}

pub fn overrides() -> Overrides {
    STATE.read().unwrap_or_else(|e| e.into_inner()).overrides.clone()
}

fn save(o: &Overrides) {
    let path = file();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let r = if o.keys.is_empty() { std::fs::remove_file(&path).or(Ok(())) } else { std::fs::write(&path, serde_json::to_string_pretty(o).unwrap_or_default()) };
    if let Err(e) = r {
        eprintln!("zebra: could not save {}: {e}", path.display());
    }
}

/// Give `a` the shortcut `m`+`k` (its only one). Returns the action that
/// had those keys before, which loses them.
pub fn set(a: &Action, m: Modifiers, k: Key) -> Option<Action> {
    let before = with(|b| b.iter().find(|x| x.1 == k && same_mods(x.0, m) && &x.2 != a).map(|x| x.2.clone()));
    let mut o = overrides();
    if let Some(prev) = &before {
        if rebindable().contains(prev) {
            let keep: Vec<String> = bindings_for(prev).into_iter().filter(|(pm, pk)| !(*pk == k && same_mods(*pm, m))).map(|(pm, pk)| format(pm, pk)).collect();
            o.keys.insert(id(prev), keep);
        }
    }
    o.keys.insert(id(a), vec![format(m, k)]);
    tidy(&mut o);
    save(&o);
    install(o);
    before
}

/// Remove `a`'s shortcut.
pub fn clear(a: &Action) {
    let mut o = overrides();
    o.keys.insert(id(a), vec![]);
    tidy(&mut o);
    save(&o);
    install(o);
}

/// Put every shortcut back as it came.
pub fn reset_all() {
    let o = Overrides::default();
    save(&o);
    install(o);
}

/// Drop entries that equal the defaults, so the file holds only changes.
fn tidy(o: &mut Overrides) {
    o.keys.retain(|k, v| {
        let Some(a) = rebindable().into_iter().find(|a| &id(a) == k) else { return false };
        let defaults: Vec<String> = DEFAULTS.iter().filter(|d| d.2 == a).map(|d| format(d.0, d.1)).collect();
        *v != defaults
    });
}

/// Is `a` shown with a key that differs from the default?
pub fn is_changed(a: &Action) -> bool {
    overrides().keys.contains_key(&id(a))
}

/// A key that can't be a shortcut on its own while typing elsewhere:
/// plain letters are fine (tools use them), bare modifiers are not.
pub fn usable(k: Key) -> bool {
    !matches!(k, Key::Escape)
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test touches the shared keymap, so the steps run in order.
    #[test]
    fn rebinding_moves_keys_and_reset_restores_them() {
        let cmd = Modifiers::COMMAND;
        install(Overrides::default());
        assert_eq!(action_for(cmd, Key::L), Some(Action::ToggleLock));

        // Give ⌘L to Group: Lock loses it, Group's old ⌥⌘G is gone.
        let mut o = Overrides::default();
        o.keys.insert(id(&Action::Group), vec![format(cmd, Key::L)]);
        install(o);
        assert_eq!(action_for(cmd, Key::L), Some(Action::Group));
        assert_eq!(binding_for(&Action::ToggleLock), None);
        assert_eq!(action_for(Modifiers { alt: true, ..cmd }, Key::G), None);
        assert_eq!(Action::Group.shortcut_text(), Some(crate::actions::fmt_shortcut(cmd, Key::L)));

        // An empty list removes the shortcut.
        let mut o = Overrides::default();
        o.keys.insert(id(&Action::ToggleLock), vec![]);
        install(o);
        assert_eq!(action_for(cmd, Key::L), None);

        install(Overrides::default());
        assert_eq!(action_for(cmd, Key::L), Some(Action::ToggleLock));
    }

    #[test]
    fn a_saved_bare_modifier_gives_the_action_its_default_back() {
        let mut o = Overrides::default();
        o.keys.insert(id(&Action::SendToBack), vec!["SuperLeft".into()]);
        o.keys.insert(id(&Action::ToggleLock), vec![]);
        drop_bad_keys(&mut o);
        assert!(!o.keys.contains_key(&id(&Action::SendToBack)));
        assert_eq!(o.keys.get(&id(&Action::ToggleLock)), Some(&vec![]));
        assert_eq!(parse("cmd+ShiftLeft"), None);
    }

    #[test]
    fn keys_round_trip_through_text() {
        for (m, k, _) in DEFAULTS {
            let s = format(*m, *k);
            let (pm, pk) = parse(&s).unwrap_or_else(|| panic!("{s}"));
            assert_eq!(pk, *k, "{s}");
            assert!(same_mods(pm, *m), "{s}");
        }
    }

    #[test]
    fn every_default_is_listed_or_fixed() {
        let r = rebindable();
        assert!(r.contains(&Action::ToggleLock) && r.contains(&Action::FrameTool));
        assert!(!r.contains(&Action::Copy) && !r.contains(&Action::Quit));
    }
}
