//! Native macOS menu bar (NSMenu via muda). Menu clicks become the same
//! [`Action`]s the toolbar buttons and shortcuts use.
//!
//! Only ⌘-shortcuts that text fields don't need get menu key equivalents;
//! ⌘C/⌘V/⌘X/⌘A/⌘Z stay with egui so they work inside text boxes too (the menu
//! shows them in the item title instead).
#![cfg(target_os = "macos")]

use std::collections::HashMap;

use muda::{accelerator::Accelerator, AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::actions::{Action, SHORTCUTS};
use crate::app::ZebraApp;
use crate::menus::{spec, Item};

pub struct MacMenu {
    _menu: Menu,
    ids: HashMap<String, Action>,
    recent: Submenu,
    recent_shown: Vec<std::path::PathBuf>,
}

/// Actions whose key equivalent the native menu owns.
pub fn menu_owns(a: &Action) -> bool {
    accel_for(a).is_some()
}

fn accel_for(a: &Action) -> Option<Accelerator> {
    let (m, k, _) = SHORTCUTS.iter().find(|(_, _, x)| x == a)?;
    if matches!(a, Action::Undo | Action::Redo | Action::Copy | Action::Cut | Action::Paste | Action::SelectAll | Action::Delete | Action::Deselect) {
        return None;
    }
    let is_f = matches!(k, egui::Key::F5);
    if !(m.command || m.mac_cmd) && !is_f {
        return None; // plain letters would break typing
    }
    let key = match k {
        egui::Key::Enter => "Enter".to_string(),
        egui::Key::Period => "Period".into(),
        egui::Key::Equals => "Equal".into(),
        egui::Key::Minus => "Minus".into(),
        egui::Key::OpenBracket => "BracketLeft".into(),
        egui::Key::CloseBracket => "BracketRight".into(),
        egui::Key::Backslash => "Backslash".into(),
        egui::Key::Num0 => "Digit0".into(),
        egui::Key::Plus => return None,
        k => k.name().to_uppercase(),
    };
    let mut s = String::new();
    if m.command || m.mac_cmd {
        s.push_str("CmdOrCtrl+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    s.push_str(&key);
    s.parse().ok()
}

fn title_with_hint(a: &Action) -> String {
    // Show text-field shortcuts in the title since they have no key equivalent.
    match a {
        Action::Undo => "Undo\t⌘Z".into(),
        Action::Redo => "Redo\t⇧⌘Z".into(),
        Action::Copy => "Copy\t⌘C".into(),
        Action::Cut => "Cut\t⌘X".into(),
        Action::Paste => "Paste\t⌘V".into(),
        Action::SelectAll => "Select All\t⌘A".into(),
        a => a.label(),
    }
}

fn build_items(sub: &Submenu, items: &[Item], ids: &mut HashMap<String, Action>, next: &mut u32, recent: &Submenu) {
    for it in items {
        match it {
            Item::A(a) => {
                *next += 1;
                let id = format!("zebra.{next}");
                let mi = MenuItem::with_id(id.clone(), title_with_hint(a), true, accel_for(a));
                let _ = sub.append(&mi);
                ids.insert(id, a.clone());
            }
            Item::Sep => {
                let _ = sub.append(&PredefinedMenuItem::separator());
            }
            Item::Sub(t, children) => {
                let s = Submenu::new(*t, true);
                build_items(&s, children, ids, next, recent);
                let _ = sub.append(&s);
            }
            Item::Recent => {
                let _ = sub.append(recent);
            }
        }
    }
}

impl MacMenu {
    /// Call once the event loop is running (first frame).
    pub fn install() -> MacMenu {
        let menu = Menu::new();
        let mut ids = HashMap::new();
        let mut next = 0u32;
        let recent = Submenu::new("Open Recent", true);

        // App menu
        let app_menu = Submenu::new("Zebra", true);
        let about = MenuItem::with_id("zebra.about", "About Zebra", true, None);
        ids.insert("zebra.about".to_string(), Action::About);
        let shortcuts = MenuItem::with_id("zebra.settings", "Settings…", true, "CmdOrCtrl+Comma".parse().ok());
        ids.insert("zebra.settings".to_string(), Action::ShowShortcuts);
        let quit = MenuItem::with_id("zebra.quit", "Quit Zebra", true, "CmdOrCtrl+Q".parse().ok());
        ids.insert("zebra.quit".to_string(), Action::Quit);
        let _ = app_menu.append_items(&[
            &about,
            &PredefinedMenuItem::separator(),
            &shortcuts,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &quit,
        ]);
        let _ = menu.append(&app_menu);
        let _ = AboutMetadata::default();

        let mut help: Option<Submenu> = None;
        for (title, items) in spec() {
            let sub = Submenu::new(title, true);
            build_items(&sub, &items, &mut ids, &mut next, &recent);
            if title == "Agent" {
                // Window menu goes before Help, macOS convention.
                let _ = menu.append(&sub);
                let win = Submenu::new("Window", true);
                let _ = win.append_items(&[
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::fullscreen(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::bring_all_to_front(None),
                ]);
                let _ = menu.append(&win);
                win.set_as_windows_menu_for_nsapp();
                continue;
            }
            let _ = menu.append(&sub);
            if title == "Help" {
                help = Some(sub);
            }
        }
        menu.init_for_nsapp();
        if let Some(h) = &help {
            h.set_as_help_menu_for_nsapp();
        }
        MacMenu { _menu: menu, ids, recent, recent_shown: vec![] }
    }
}

pub fn poll(app: &mut ZebraApp, ctx: &egui::Context) {
    if app.menu.is_none() {
        app.menu = Some(MacMenu::install());
    }
    // Keep Open Recent in sync.
    if let Some(m) = &mut app.menu {
        if m.recent_shown != app.recent {
            for item in m.recent.items() {
                if let Some(mi) = item.as_menuitem() {
                    let _ = m.recent.remove(mi);
                }
            }
            m.ids.retain(|k, _| !k.starts_with("zebra.recent."));
            for (i, p) in app.recent.iter().enumerate() {
                let id = format!("zebra.recent.{i}");
                let mi = MenuItem::with_id(id.clone(), p.display().to_string(), true, None);
                let _ = m.recent.append(&mi);
                m.ids.insert(id, Action::OpenPath(p.clone()));
            }
            m.recent_shown = app.recent.clone();
        }
    }
    while let Ok(ev) = MenuEvent::receiver().try_recv() {
        let Some(a) = app.menu.as_ref().and_then(|m| m.ids.get(ev.id().0.as_str()).cloned()) else { continue };
        // Clipboard/undo items while typing go to the text field.
        if ctx.egui_wants_keyboard_input() {
            let ev = match a {
                Action::Copy => Some(egui::Event::Copy),
                Action::Cut => Some(egui::Event::Cut),
                _ => None,
            };
            if let Some(e) = ev {
                app.inject.push(e);
                ctx.request_repaint();
                continue;
            }
            if matches!(a, Action::Paste | Action::Undo | Action::Redo | Action::SelectAll) {
                continue;
            }
        }
        app.queued.push(a);
        ctx.request_repaint();
    }
}
