//! Native macOS menu bar (NSMenu via muda). Menu clicks become the same
//! [`Action`]s the toolbar buttons and shortcuts use.
//!
//! Every item carries its real key equivalent, including ⌘C/⌘V/⌘X/⌘A/⌘Z,
//! so the menus look and behave like any Mac app. NSMenu handles those keys
//! before the window sees them, so while an egui text field has focus the
//! clipboard and undo items are forwarded to egui as events (Copy, Cut,
//! Paste with the pasteboard's text, ⌘Z, ⇧⌘Z, ⌘A) instead of acting on the
//! canvas.
//!
//! Item titles, enabled state and ✓ marks follow the app every frame
//! (`menus::state`), pushed to AppKit only when they change.
#![cfg(target_os = "macos")]

use std::collections::HashMap;

use muda::{accelerator::Accelerator, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::actions::{Action, SHORTCUTS};
use crate::app::ZebraApp;
use crate::menus::{is_checkable, spec, state, Item, ItemState};

enum Native {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

struct Entry {
    item: Native,
    action: Action,
    last: ItemState,
}

pub struct MacMenu {
    _menu: Menu,
    ids: HashMap<String, Action>,
    entries: Vec<Entry>,
    recent: Submenu,
    recent_shown: Vec<std::path::PathBuf>,
}

/// Actions whose key equivalent the native menu owns.
pub fn menu_owns(a: &Action) -> bool {
    accel_for(a).is_some()
}

fn accel_for(a: &Action) -> Option<Accelerator> {
    let (m, k, _) = SHORTCUTS.iter().find(|(_, _, x)| x == a)?;
    // Plain-letter tool keys and ⌫/Esc stay with egui: as menu key
    // equivalents they would break typing.
    let is_f = matches!(k, egui::Key::F5);
    if !(m.command || m.mac_cmd) && !is_f {
        return None;
    }
    let key = match k {
        egui::Key::Enter => "Enter".to_string(),
        egui::Key::Period => "Period".into(),
        egui::Key::Comma => "Comma".into(),
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
    if m.ctrl {
        s.push_str("Control+");
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

/// Undo/Redo/clipboard shortcuts egui text fields also need.
fn text_field_accel(a: &Action) -> Option<Accelerator> {
    let s = match a {
        Action::Undo => "CmdOrCtrl+Z",
        Action::Redo => "CmdOrCtrl+Shift+Z",
        Action::Cut => "CmdOrCtrl+X",
        Action::Copy => "CmdOrCtrl+C",
        Action::Paste => "CmdOrCtrl+V",
        Action::SelectAll => "CmdOrCtrl+A",
        _ => return None,
    };
    s.parse().ok()
}

fn build_items(sub: &Submenu, items: &[Item], m: &mut MacMenu, next: &mut u32, app: &ZebraApp) {
    for it in items {
        match it {
            Item::A(a) => {
                *next += 1;
                let id = format!("zebra.{next}");
                let st = state(app, a);
                let accel = text_field_accel(a).or_else(|| accel_for(a));
                let item = if is_checkable(a) {
                    let c = CheckMenuItem::with_id(id.clone(), &st.title, st.enabled, st.checked.unwrap_or(false), accel);
                    let _ = sub.append(&c);
                    Native::Check(c)
                } else {
                    let mi = MenuItem::with_id(id.clone(), &st.title, st.enabled, accel);
                    let _ = sub.append(&mi);
                    Native::Plain(mi)
                };
                m.ids.insert(id, a.clone());
                m.entries.push(Entry { item, action: a.clone(), last: st });
            }
            Item::Sep => {
                let _ = sub.append(&PredefinedMenuItem::separator());
            }
            Item::Sub(t, children) => {
                let s = Submenu::new(*t, true);
                build_items(&s, children, m, next, app);
                let _ = sub.append(&s);
            }
            Item::Recent => {
                let _ = sub.append(&m.recent);
            }
        }
    }
}

impl MacMenu {
    /// Call once the event loop is running (first frame).
    pub fn install(app: &ZebraApp) -> MacMenu {
        let menu = Menu::new();
        let mut m = MacMenu { _menu: menu, ids: HashMap::new(), entries: vec![], recent: Submenu::new("Open Recent", true), recent_shown: vec![] };
        let mut next = 0u32;

        // App menu
        let app_menu = Submenu::new("Zebra", true);
        let about = MenuItem::with_id("zebra.about", "About Zebra", true, None);
        m.ids.insert("zebra.about".to_string(), Action::About);
        let settings = MenuItem::with_id("zebra.settings", "Settings…", true, "CmdOrCtrl+Comma".parse().ok());
        m.ids.insert("zebra.settings".to_string(), Action::Settings);
        let quit = MenuItem::with_id("zebra.quit", "Quit Zebra", true, "CmdOrCtrl+Q".parse().ok());
        m.ids.insert("zebra.quit".to_string(), Action::Quit);
        let _ = app_menu.append_items(&[
            &about,
            &PredefinedMenuItem::separator(),
            &settings,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &quit,
        ]);
        let _ = m._menu.append(&app_menu);

        let mut help: Option<Submenu> = None;
        for (title, items) in spec() {
            let sub = Submenu::new(title, true);
            build_items(&sub, &items, &mut m, &mut next, app);
            if title == "Help" {
                // Window menu goes before Help, macOS convention.
                let win = Submenu::new("Window", true);
                let _ = win.append_items(&[
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::maximize(None),
                    &PredefinedMenuItem::fullscreen(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::bring_all_to_front(None),
                ]);
                let _ = m._menu.append(&win);
                win.set_as_windows_menu_for_nsapp();
                help = Some(sub.clone());
            }
            let _ = m._menu.append(&sub);
        }
        m._menu.init_for_nsapp();
        // Help menu: AppKit adds the search field that finds any menu item.
        if let Some(h) = &help {
            h.set_as_help_menu_for_nsapp();
        }
        m
    }

    /// Push changed titles, enabled states and ✓ marks to AppKit.
    fn sync(&mut self, app: &ZebraApp) {
        for e in &mut self.entries {
            let st = state(app, &e.action);
            if st == e.last {
                continue;
            }
            match &e.item {
                Native::Plain(mi) => {
                    if st.title != e.last.title {
                        mi.set_text(&st.title);
                    }
                    if st.enabled != e.last.enabled {
                        mi.set_enabled(st.enabled);
                    }
                }
                Native::Check(c) => {
                    if st.title != e.last.title {
                        c.set_text(&st.title);
                    }
                    if st.enabled != e.last.enabled {
                        c.set_enabled(st.enabled);
                    }
                    if st.checked != e.last.checked {
                        c.set_checked(st.checked.unwrap_or(false));
                    }
                }
            }
            e.last = st;
        }
    }
}

/// Read the general pasteboard's plain text (for ⌘V into a text field).
fn pasteboard_text() -> Option<String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    let pb = NSPasteboard::generalPasteboard();
    // SAFETY: NSPasteboardTypeString is a static NSString constant.
    let s = pb.stringForType(unsafe { NSPasteboardTypeString })?;
    Some(s.to_string())
}

pub fn poll(app: &mut ZebraApp, ctx: &egui::Context) {
    if app.menu.is_none() {
        app.menu = Some(MacMenu::install(app));
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
                let name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                let mi = MenuItem::with_id(id.clone(), name, true, None);
                let _ = m.recent.append(&mi);
                m.ids.insert(id, Action::OpenPath(p.clone()));
            }
            m.recent_shown = app.recent.clone();
        }
    }
    if let Some(mut m) = app.menu.take() {
        m.sync(app);
        app.menu = Some(m);
    }
    while let Ok(ev) = MenuEvent::receiver().try_recv() {
        let Some(a) = app.menu.as_ref().and_then(|m| m.ids.get(ev.id().0.as_str()).cloned()) else { continue };
        // Clipboard and undo items while typing go to the text field.
        if ctx.egui_wants_keyboard_input() {
            let cmd = egui::Modifiers::COMMAND;
            let key = |k: egui::Key, modifiers: egui::Modifiers| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers };
            let ev = match a {
                Action::Copy => Some(egui::Event::Copy),
                Action::Cut => Some(egui::Event::Cut),
                Action::Paste => pasteboard_text().map(egui::Event::Paste),
                Action::Undo => Some(key(egui::Key::Z, cmd)),
                Action::Redo => Some(key(egui::Key::Z, egui::Modifiers { shift: true, ..cmd })),
                Action::SelectAll => Some(key(egui::Key::A, cmd)),
                _ => None,
            };
            if matches!(a, Action::Copy | Action::Cut | Action::Paste | Action::Undo | Action::Redo | Action::SelectAll) {
                if let Some(e) = ev {
                    app.inject.push(e);
                    ctx.request_repaint();
                }
                continue;
            }
        }
        app.queued.push(a);
        ctx.request_repaint();
    }
}
