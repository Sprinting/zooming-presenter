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

use crate::actions::Action;
use crate::app::ZebraApp;
use crate::menus::{is_checkable, native, spec, state, Item, ItemState};

enum Native {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

struct Entry {
    item: Native,
    action: Action,
    last: ItemState,
    /// The key equivalent pushed to AppKit (follows Settings › Keyboard).
    accel: Option<String>,
}

/// Held by the window the person is using: it has the menu bar. The bar
/// itself is one per app (see `Bar`).
pub struct MacMenu(());

/// The menu bar. It is built once, before the app finishes launching (see
/// `install_early`), and then only changed in place. AppKit adds its own
/// commands to the Window menu (Show Next Tab, Move Tab to New Window, Merge
/// All Windows) a single time per app; a bar built anew after that (we used
/// to on a language change, and whenever the front window closed) never
/// gets them again, and nothing we can call asks for them.
struct Bar {
    _menu: Menu,
    ids: HashMap<String, Action>,
    entries: Vec<Entry>,
    recent: Submenu,
    recent_shown: Vec<std::path::PathBuf>,
    /// Our submenus in the order `menus::sub_titles` lists their titles.
    subs: Vec<Submenu>,
    app_menu: Option<Submenu>,
    window: Option<Submenu>,
    about: Option<MenuItem>,
    quit: Option<MenuItem>,
}

thread_local! {
    // AppKit menus live on the main thread, and so does this.
    static BAR: std::cell::RefCell<Option<Bar>> = const { std::cell::RefCell::new(None) };
}

/// Actions whose key equivalent the native menu owns.
pub fn menu_owns(a: &Action) -> bool {
    accel_for(a).is_some()
}

fn accel_for(a: &Action) -> Option<Accelerator> {
    accel_spec(a)?.parse().ok()
}

/// The key equivalent of `a` in muda's notation ("CmdOrCtrl+Shift+A").
fn accel_spec(a: &Action) -> Option<String> {
    if let Some(s) = text_field_spec(a) {
        return Some(s.to_string());
    }
    let (m, k) = crate::keymap::binding_for(a)?;
    let (m, k) = (&m, &k);
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
    Some(s)
}

/// The keys of a menu item's key equivalent, as egui names them.
fn menu_keys(a: &Action) -> Option<(egui::Modifiers, egui::Key)> {
    let cmd = egui::Modifiers::COMMAND;
    let fixed = match a {
        Action::Undo => Some((cmd, egui::Key::Z)),
        Action::Redo => Some((egui::Modifiers { shift: true, ..cmd }, egui::Key::Z)),
        Action::Cut => Some((cmd, egui::Key::X)),
        Action::Copy => Some((cmd, egui::Key::C)),
        Action::Paste => Some((cmd, egui::Key::V)),
        Action::SelectAll => Some((cmd, egui::Key::A)),
        _ => None,
    };
    fixed.or_else(|| crate::keymap::binding_for(a))
}

/// Undo/Redo/clipboard shortcuts egui text fields also need (fixed).
fn text_field_spec(a: &Action) -> Option<&'static str> {
    Some(match a {
        Action::Undo => "CmdOrCtrl+Z",
        Action::Redo => "CmdOrCtrl+Shift+Z",
        Action::Cut => "CmdOrCtrl+X",
        Action::Copy => "CmdOrCtrl+C",
        Action::Paste => "CmdOrCtrl+V",
        Action::SelectAll => "CmdOrCtrl+A",
        Action::Quit => "CmdOrCtrl+Q",
        _ => return None,
    })
}

/// An item's state when the bar is built: the app's, or, before there is an
/// app, its plain title and off (the first `sync` brings it up to date).
fn first_state(app: Option<&ZebraApp>, a: &Action) -> ItemState {
    match app {
        Some(app) => state(app, a),
        None => ItemState { title: a.label(), enabled: false, checked: is_checkable(a).then_some(false) },
    }
}

fn build_items(sub: &Submenu, items: &[Item], m: &mut Bar, next: &mut u32, app: Option<&ZebraApp>) {
    for it in items {
        match it {
            Item::A(a) => {
                *next += 1;
                let id = format!("zebra.{next}");
                let st = first_state(app, a);
                let spec = accel_spec(a);
                let accel = spec.as_deref().and_then(|s| s.parse().ok());
                let item = if is_checkable(a) {
                    let c = CheckMenuItem::with_id(id.clone(), native(&st.title), st.enabled, st.checked.unwrap_or(false), accel);
                    let _ = sub.append(&c);
                    Native::Check(c)
                } else {
                    let mi = MenuItem::with_id(id.clone(), native(&st.title), st.enabled, accel);
                    let _ = sub.append(&mi);
                    Native::Plain(mi)
                };
                m.ids.insert(id, a.clone());
                m.entries.push(Entry { item, action: a.clone(), last: st, accel: spec });
            }
            Item::Sep => {
                let _ = sub.append(&PredefinedMenuItem::separator());
            }
            Item::Sub(t, children) => {
                let s = Submenu::new(native(&t), true);
                m.subs.push(s.clone());
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
    /// Give `app` the menu bar: built the first time (once the event loop is
    /// running), and after that brought up to date in place, e.g. in a new
    /// language.
    pub fn install(app: &ZebraApp) -> MacMenu {
        BAR.with(|b| {
            let mut b = b.borrow_mut();
            match b.as_mut() {
                Some(bar) => bar.retitle(),
                None => *b = Some(Bar::build(Some(app))),
            }
        });
        MacMenu(())
    }
}

/// Build the menu bar before `eframe::run_native`, so it is in place when
/// NSApp finishes launching. In a bundled app AppKit decides then whether
/// the Window menu gets its tab commands (Show Previous Tab, Show Next Tab,
/// Move Tab to New Window, Merge All Windows); a Window menu registered on
/// the first frame, as before, never got them in Zebra.app (a bare binary
/// did, which hid the problem). winit's own default menu must stay off
/// (`with_default_menu(false)`), or it replaces this one at launch.
pub fn install_early() {
    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    BAR.with(|b| {
        let mut b = b.borrow_mut();
        if b.is_none() {
            *b = Some(Bar::build(None));
        }
    });
}

impl Bar {
    fn build(app: Option<&ZebraApp>) -> Bar {
        let menu = Menu::new();
        let mut m = Bar {
            _menu: menu,
            ids: HashMap::new(),
            entries: vec![],
            recent: Submenu::new(crate::t!("menu-open-recent"), true),
            recent_shown: vec![],
            subs: vec![],
            app_menu: None,
            window: None,
            about: None,
            quit: None,
        };
        let mut next = 0u32;

        // App menu
        let app_menu = Submenu::new(crate::t!("menu-mac-app"), true);
        let about = MenuItem::with_id("zebra.about", native(&Action::About.label()), true, None);
        m.ids.insert("zebra.about".to_string(), Action::About);
        let settings_spec = accel_spec(&Action::Settings);
        let settings = MenuItem::with_id("zebra.settings", native(&Action::Settings.label()), true, accel_for(&Action::Settings));
        m.ids.insert("zebra.settings".to_string(), Action::Settings);
        let quit = MenuItem::with_id("zebra.quit", native(&Action::Quit.label()), true, accel_for(&Action::Quit));
        m.ids.insert("zebra.quit".to_string(), Action::Quit);
        let _ = app_menu.append_items(&[
            &about,
            &PredefinedMenuItem::separator(),
            &settings,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(Some(&native(&crate::t!("menu-mac-services")))),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some(&native(&crate::t!("menu-mac-hide")))),
            &PredefinedMenuItem::hide_others(Some(&native(&crate::t!("menu-mac-hide-others")))),
            &PredefinedMenuItem::show_all(Some(&native(&crate::t!("menu-mac-show-all")))),
            &PredefinedMenuItem::separator(),
            &quit,
        ]);
        let _ = m._menu.append(&app_menu);
        m.entries.push(Entry { item: Native::Plain(settings.clone()), action: Action::Settings, last: first_state(app, &Action::Settings), accel: settings_spec });

        let mut help: Option<Submenu> = None;
        let mut window: Option<Submenu> = None;
        let help_title = crate::t!("menu-help");
        for (title, items) in spec() {
            let sub = Submenu::new(native(&title), true);
            m.subs.push(sub.clone());
            build_items(&sub, &items, &mut m, &mut next, app);
            if title == help_title {
                // Window menu goes before Help, macOS convention.
                let win = Submenu::new(crate::t!("menu-mac-window"), true);
                let _ = win.append_items(&[
                    &PredefinedMenuItem::minimize(Some(&native(&crate::t!("menu-mac-minimize")))),
                    &PredefinedMenuItem::maximize(Some(&native(&crate::t!("menu-mac-zoom")))),
                    &PredefinedMenuItem::fullscreen(Some(&native(&crate::t!("menu-mac-full-screen")))),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::bring_all_to_front(Some(&native(&crate::t!("menu-mac-bring-all-to-front")))),
                ]);
                let _ = m._menu.append(&win);
                window = Some(win);
                help = Some(sub.clone());
            }
            let _ = m._menu.append(&sub);
        }
        m._menu.init_for_nsapp();
        // Both only take once the menu bar is installed: muda finds the
        // submenu through NSApp's main menu, and before that quietly does
        // nothing. The Window menu is where AppKit adds the window list and
        // the tab commands (Merge All Windows, Show Next Tab…).
        if let Some(w) = &window {
            w.set_as_windows_menu_for_nsapp();
        }
        // Help menu: AppKit adds the search field that finds any menu item.
        if let Some(h) = &help {
            h.set_as_help_menu_for_nsapp();
        }
        m.app_menu = Some(app_menu);
        m.window = window;
        m.about = Some(about);
        m.quit = Some(quit);
        m
    }

    /// The titles that don't follow an action's state, in the current
    /// language. (Items with an action are retitled by `sync`.)
    fn retitle(&mut self) {
        for (sub, title) in self.subs.iter().zip(crate::menus::sub_titles()) {
            sub.set_text(native(&title));
        }
        self.recent.set_text(native(&crate::t!("menu-open-recent")));
        if let Some(s) = &self.app_menu {
            s.set_text(native(&crate::t!("menu-mac-app")));
        }
        if let Some(s) = &self.window {
            s.set_text(native(&crate::t!("menu-mac-window")));
        }
        if let Some(i) = &self.about {
            i.set_text(native(&Action::About.label()));
        }
        if let Some(i) = &self.quit {
            i.set_text(native(&Action::Quit.label()));
        }
    }

    /// Push changed titles, enabled states and ✓ marks to AppKit.
    fn sync(&mut self, app: &ZebraApp) {
        for e in &mut self.entries {
            let spec = accel_spec(&e.action);
            if spec != e.accel {
                let accel = spec.as_deref().and_then(|s| s.parse().ok());
                let _ = match &e.item {
                    Native::Plain(mi) => mi.set_accelerator(accel),
                    Native::Check(c) => c.set_accelerator(accel),
                };
                e.accel = spec;
            }
            let st = state(app, &e.action);
            if st == e.last {
                continue;
            }
            match &e.item {
                Native::Plain(mi) => {
                    if st.title != e.last.title {
                        mi.set_text(native(&st.title));
                    }
                    if st.enabled != e.last.enabled {
                        mi.set_enabled(st.enabled);
                    }
                }
                Native::Check(c) => {
                    if st.title != e.last.title {
                        c.set_text(native(&st.title));
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

fn recent_key(p: &std::path::Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Read the general pasteboard's plain text (for ⌘V into a text field).
fn pasteboard_text() -> Option<String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    let pb = NSPasteboard::generalPasteboard();
    // SAFETY: NSPasteboardTypeString is a static NSString constant.
    let s = pb.stringForType(unsafe { NSPasteboardTypeString })?;
    Some(s.to_string())
}

/// Menu clicks, delivered here and waking the app: with no window open,
/// nothing else would wake it to act on them.
fn next_event(ctx: &egui::Context) -> Option<MenuEvent> {
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Mutex, OnceLock};
    static RX: OnceLock<Mutex<Receiver<MenuEvent>>> = OnceLock::new();
    let rx = RX.get_or_init(|| {
        let (tx, rx) = channel();
        let c = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = tx.send(e);
            c.request_repaint();
        }));
        Mutex::new(rx)
    });
    rx.lock().ok()?.try_recv().ok()
}

pub fn poll(app: &mut ZebraApp, ctx: &egui::Context) {
    // AppKit menus exist only on the main thread. Test harnesses run the
    // app on worker threads: they get egui's own shortcuts instead.
    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    if app.menu.is_none() {
        app.menu = Some(MacMenu::install(app));
    }
    BAR.with(|b| {
        let mut b = b.borrow_mut();
        let Some(m) = b.as_mut() else { return };
        // Keep Open Recent in sync.
        if m.recent_shown != app.recent {
            for item in m.recent.items() {
                if let Some(mi) = item.as_menuitem() {
                    let _ = m.recent.remove(mi);
                }
            }
            m.ids.retain(|k, _| !k.starts_with("zebra.recent."));
            for p in app.recent.iter() {
                // The id names the file, not its place in the list: a click
                // still waiting when the list is rebuilt opens what was clicked.
                let id = format!("zebra.recent.{}", recent_key(p));
                let name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                let mi = MenuItem::with_id(id.clone(), native(&name), true, None);
                let _ = m.recent.append(&mi);
                m.ids.insert(id, Action::OpenPath(p.clone()));
            }
            m.recent_shown = app.recent.clone();
        }
        m.sync(app);
    });
    while let Some(ev) = next_event(ctx) {
        let Some(a) = BAR.with(|b| b.borrow().as_ref().and_then(|m| m.ids.get(ev.id().0.as_str()).cloned())) else { continue };
        // Settings › Keyboard is recording: a key the menu bar owns was
        // pressed. Hand its keys to the recorder instead of acting.
        if let Some(cap) = &mut app.key_capture {
            cap.got = menu_keys(&a);
            ctx.request_repaint();
            continue;
        }
        // Clipboard and undo items while typing go to the text field.
        // A text box open on the canvas takes these even in the moment
        // egui has not given it the keyboard yet.
        if ctx.egui_wants_keyboard_input() || app.canvas.text.is_some() {
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
