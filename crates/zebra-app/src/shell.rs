//! The app with several document windows, like Keynote: every document is
//! its own window (macOS groups them into tabs by itself when the person
//! prefers tabs, or with Window › Merge All Windows).
//!
//! Each window is a whole [`ZebraApp`]. The first one is eframe's root
//! window, the others are egui viewports. The shell:
//! - opens a window for New and Open when the front one already holds a
//!   document, and closes windows (the last one quits, as before);
//! - keeps the settings every window shares in step (AI switch, theme for
//!   new documents, snapping, recent files, the clipboard...);
//! - answers the agent API for every document: `documents` lists them and a
//!   call can name one; without a name it goes to the front window;
//! - moves the menu bar and Settings to the window the person is using.

use std::sync::Arc;

use crate::actions::Action;
use crate::app::{ScrollMode, ZebraApp};

/// Settings and state every document window shares.
#[derive(Clone)]
struct Shared {
    ai_enabled: bool,
    ai_saved: bool,
    api: Option<Arc<zebra_api::ApiServer>>,
    api_error: Option<String>,
    ai_off_at: Option<f64>,
    default_theme: String,
    snapping: bool,
    scroll_mode: ScrollMode,
    motion_override: Option<bool>,
    reduce_motion: bool,
    recent: Vec<std::path::PathBuf>,
    clipboard: Vec<zebra_core::Element>,
    paste_count: u32,
    show_settings: bool,
    settings_pane: crate::settings::Pane,
    key_capture: Option<crate::keymap::Capture>,
    key_message: Option<String>,
    permission: zebra_core::view::Permission,
    paused: bool,
}

impl Shared {
    fn read(w: &ZebraApp) -> Self {
        Shared {
            ai_enabled: w.ai_enabled,
            ai_saved: w.ai_saved,
            api: w.api.clone(),
            api_error: w.api_error.clone(),
            ai_off_at: w.ai_off_at,
            default_theme: w.default_theme.clone(),
            snapping: w.snapping,
            scroll_mode: w.scroll_mode,
            motion_override: w.motion_override,
            reduce_motion: w.editor.view.reduce_motion,
            recent: w.recent.clone(),
            clipboard: w.clipboard.clone(),
            paste_count: w.paste_count,
            show_settings: w.show_settings,
            settings_pane: w.settings_pane,
            key_capture: w.key_capture.clone(),
            key_message: w.key_message.clone(),
            permission: w.editor.view.agent_permission,
            paused: w.editor.view.agent_paused,
        }
    }

    fn write(&self, w: &mut ZebraApp) {
        w.ai_enabled = self.ai_enabled;
        w.ai_saved = self.ai_saved;
        w.api = self.api.clone();
        w.api_error = self.api_error.clone();
        w.ai_off_at = self.ai_off_at;
        w.default_theme = self.default_theme.clone();
        w.snapping = self.snapping;
        w.scroll_mode = self.scroll_mode;
        w.motion_override = self.motion_override;
        w.editor.view.reduce_motion = self.reduce_motion;
        w.recent = self.recent.clone();
        w.clipboard = self.clipboard.clone();
        w.paste_count = self.paste_count;
        w.show_settings = self.show_settings;
        w.settings_pane = self.settings_pane;
        w.key_capture = self.key_capture.clone();
        w.key_message = self.key_message.clone();
        w.editor.view.agent_permission = self.permission;
        w.editor.view.agent_paused = self.paused;
    }

    fn same(&self, o: &Self) -> bool {
        let api_same = match (&self.api, &o.api) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        api_same
            && self.ai_enabled == o.ai_enabled
            && self.ai_saved == o.ai_saved
            && self.api_error == o.api_error
            && self.ai_off_at == o.ai_off_at
            && self.default_theme == o.default_theme
            && self.snapping == o.snapping
            && self.scroll_mode == o.scroll_mode
            && self.motion_override == o.motion_override
            && self.reduce_motion == o.reduce_motion
            && self.recent == o.recent
            && self.clipboard == o.clipboard
            && self.paste_count == o.paste_count
            && self.show_settings == o.show_settings
            && self.settings_pane == o.settings_pane
            && self.key_capture.as_ref().map(|c| (&c.action, c.got)) == o.key_capture.as_ref().map(|c| (&c.action, c.got))
            && self.key_message == o.key_message
            && self.permission == o.permission
            && self.paused == o.paused
    }
}

pub struct Shell {
    /// Document windows; `windows[0]` is eframe's root window.
    pub windows: Vec<ZebraApp>,
    /// Index of the window the person is using.
    pub front: usize,
    shared: Shared,
    dispatcher: Option<zebra_api::Dispatcher>,
    next_id: u32,
    quitting: bool,
    /// Every document is closed: the root window is hidden and Zebra runs
    /// with only its menu bar, like Keynote.
    pub empty: bool,
    /// A window made by the tab bar's + button: (new window, the window
    /// whose tab group it joins) until AppKit has both.
    tab_into: Option<(String, String)>,
}

/// The placeholder title a new window is created with, so its NSWindow
/// can be found before the real title is set.
fn placeholder_title(id: &str) -> String {
    format!("zebra-window-{id}")
}

impl Shell {
    pub fn new(mut first: ZebraApp) -> Self {
        first.hosted = true;
        first.front = true;
        first.doc_id = "d1".into();
        let dispatcher = first.dispatcher.take();
        crate::mac::install_reopen_hook(&first.ctx);
        let reopen = std::mem::take(&mut first.reopen_untitled);
        let mut shell = Shell { shared: Shared::read(&first), windows: vec![first], front: 0, dispatcher, next_id: 2, quitting: false, empty: false, tab_into: None };
        // More untitled documents from the last run: a window each.
        for p in reopen {
            let id = format!("d{}", shell.next_id);
            let mut w = shell.windows[0].new_window(id);
            if w.reopen_autosave(&p) {
                shell.next_id += 1;
                shell.shared.write(&mut w);
                shell.windows.push(w);
            }
        }
        shell
    }

    /// After window `i` ran: if it changed a shared setting, every window
    /// gets the change.
    fn sync_from(&mut self, i: usize) {
        let now = Shared::read(&self.windows[i]);
        if now.same(&self.shared) {
            return;
        }
        self.shared = now;
        for (j, w) in self.windows.iter_mut().enumerate() {
            if j != i {
                self.shared.write(w);
            }
        }
    }

    fn set_front(&mut self, i: usize) {
        if i == self.front || i >= self.windows.len() {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let menu = self.windows[self.front].menu.take();
            self.windows[i].menu = menu;
        }
        self.windows[self.front].front = false;
        self.windows[i].front = true;
        self.front = i;
    }

    fn open_window(&mut self, action: Action) {
        let id = format!("d{}", self.next_id);
        self.next_id += 1;
        let mut w = self.windows[self.front].new_window(id);
        self.shared.write(&mut w);
        w.queued.push(action);
        self.windows.push(w);
        let i = self.windows.len() - 1;
        self.set_front(i);
    }

    /// Remove window `i`. The root window can't go while others are open:
    /// another document moves into it instead.
    fn close_window(&mut self, ctx: &egui::Context, i: usize) {
        if self.windows.len() == 1 {
            if self.quitting {
                self.windows[0].force_quit = true;
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                return;
            }
            // The last window: like Keynote, Zebra keeps running with only
            // its menu bar. The hidden root window waits with a new document.
            let id = format!("d{}", self.next_id);
            self.next_id += 1;
            let mut fresh = self.windows[0].new_window(id);
            fresh.front = true;
            #[cfg(target_os = "macos")]
            {
                fresh.menu = self.windows[0].menu.take();
                fresh.mac_sync = std::mem::take(&mut self.windows[0].mac_sync);
            }
            self.shared.write(&mut fresh);
            self.windows[0] = fresh;
            self.front = 0;
            self.empty = true;
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(false));
            return;
        }
        if i == 0 {
            // The root window takes over the next document and its place.
            let next = self.windows.remove(1);
            let rect = viewport_rect(ctx, &next.doc_id);
            let old = std::mem::replace(&mut self.windows[0], next);
            drop(old);
            let w = &mut self.windows[0];
            w.last_title.clear();
            #[cfg(target_os = "macos")]
            {
                w.mac_sync = Default::default();
            }
            if let Some(r) = rect {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::OuterPosition(r.min));
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::InnerSize(r.size()));
            }
        } else {
            self.windows.remove(i);
        }
        let front = if self.front >= i && self.front > 0 { self.front - 1 } else { self.front }.min(self.windows.len() - 1);
        for (j, w) in self.windows.iter_mut().enumerate() {
            w.front = j == front;
        }
        self.front = front;
    }

    /// Requests windows left for the shell this frame.
    fn handle_requests(&mut self, ctx: &egui::Context) {
        // New and Open that need a window of their own.
        let mut spawn = vec![];
        for w in &mut self.windows {
            spawn.append(&mut w.spawn);
        }
        for a in spawn {
            self.open_window(a);
        }
        if self.windows.iter_mut().any(|w| std::mem::take(&mut w.quit_all)) {
            self.quitting = true;
        }
        if self.quitting {
            // Each window closes in turn; one with unsaved changes asks
            // first, and Cancel there stops the quit.
            for w in &mut self.windows {
                if !w.quit_pending {
                    w.quit_pending = true;
                    w.queued.push(Action::Close);
                } else if !w.close_me && !w.sheet_showing() && w.queued.is_empty() {
                    self.quitting = false;
                }
            }
            if !self.quitting {
                for w in &mut self.windows {
                    w.quit_pending = false;
                }
            }
        }
        if let Some(i) = self.windows.iter().position(|w| w.close_me) {
            self.close_window(ctx, i);
            ctx.request_repaint();
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        let api = self.shared.api.clone();
        match (&api, &mut self.dispatcher) {
            (Some(_), None) => self.dispatcher = Some(zebra_api::Dispatcher::new(self.windows[0].renderer.clone())),
            (None, Some(d)) => {
                d.shutdown_docs(&mut Docs { windows: &mut self.windows, front: self.front });
                self.dispatcher = None;
            }
            _ => {}
        }
        let (Some(server), Some(d)) = (api, &mut self.dispatcher) else {
            for w in &mut self.windows {
                w.editor.drain_events();
            }
            return;
        };
        if d.pump_docs(&server, &mut Docs { windows: &mut self.windows, front: self.front }) {
            ctx.request_repaint();
        }
        let _ = d.take_events();
        if std::mem::take(&mut d.disconnect_requested) && self.shared.ai_off_at.is_none() {
            let w = &mut self.windows[self.front];
            w.ai_off_at = Some(w.now() + 0.3);
            ctx.request_repaint_after(std::time::Duration::from_millis(350));
            self.sync_from(self.front);
        }
    }
}

/// The current window is out of sight: a background tab, minimised, or
/// fully covered.
fn hidden(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().occluded == Some(true) || i.viewport().minimized == Some(true))
}

/// Where a document window is on screen (egui's last report).
fn viewport_rect(ctx: &egui::Context, id: &str) -> Option<egui::Rect> {
    let vid = egui::ViewportId::from_hash_of(("document", id));
    ctx.input(|i| i.raw.viewports.get(&vid).and_then(|v| v.outer_rect))
}

/// The open documents, for the agent API.
struct Docs<'a> {
    windows: &'a mut Vec<ZebraApp>,
    front: usize,
}

impl zebra_api::Documents for Docs<'_> {
    fn list(&self) -> Vec<zebra_api::DocInfo> {
        // Front window first.
        let mut v: Vec<zebra_api::DocInfo> =
            self.windows.iter().enumerate().map(|(i, w)| zebra_api::doc_info(&w.doc_id, &w.editor, i == self.front)).collect();
        v.sort_by_key(|d| !d.front);
        v
    }
    fn editor(&mut self, id: &str) -> Option<&mut zebra_core::Editor> {
        self.windows.iter_mut().find(|w| w.doc_id == id).map(|w| &mut w.editor)
    }
}

impl Shell {
    /// No window open: the menu bar and a Dock click bring one back.
    fn logic_empty(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        crate::menu_mac::poll(&mut self.windows[0], ctx);
        let w = &mut self.windows[0];
        if w.queued.contains(&Action::Quit) {
            w.force_quit = true;
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            return;
        }
        let reopen = crate::mac::take_reopen();
        if reopen && w.queued.is_empty() {
            // Like a fresh launch: the template chooser.
            w.welcome = Some(crate::welcome::Welcome::new());
        }
        if reopen || !w.queued.is_empty() {
            self.empty = false;
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            ctx.request_repaint();
        }
    }
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.empty {
            self.logic_empty(ctx);
        }
        for i in 0..self.windows.len() {
            self.windows[i].logic(ctx);
            self.sync_from(i);
        }
        self.pump(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // The other windows are drawn during the root window's pass, so it
        // must run even when the root window is a tab in the background.
        if self.windows.len() > 1 {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::RequestPaintWhileHidden);
        }
        // The root window.
        if ctx.input(|i| i.viewport().focused == Some(true)) {
            self.set_front(0);
        }
        if !hidden(&ctx) || self.windows.len() == 1 {
            self.windows[0].frame(ui);
        }
        #[cfg(target_os = "macos")]
        {
            crate::mac::sync_window(&mut self.windows[0], Some(frame));
            let _ = crate::sheets::run_native(&mut self.windows[0], Some(frame));
        }
        let _ = frame;
        self.sync_from(0);

        // The other windows, each in its own native window.
        let mut focused = None;
        for i in 1..self.windows.len() {
            let id = self.windows[i].doc_id.clone();
            let vid = egui::ViewportId::from_hash_of(("document", id.as_str()));
            let builder = crate::window_builder(&placeholder_title(&id));
            let w = &mut self.windows[i];
            let got_focus = ctx.show_viewport_immediate(vid, builder, |ui, _class| {
                #[cfg(target_os = "macos")]
                crate::mac::adopt_window(w, &placeholder_title(&w.doc_id));
                let f = ui.ctx().input(|i| i.viewport().focused == Some(true));
                // A tab in the background or a minimised window draws nothing
                // until it shows again (macOS tells us, and we repaint then).
                if !hidden(ui.ctx()) {
                    w.frame(ui);
                }
                #[cfg(target_os = "macos")]
                {
                    crate::mac::sync_window(w, None);
                    let _ = crate::sheets::run_native(w, None);
                }
                f
            });
            if got_focus {
                focused = Some(i);
            }
            self.sync_from(i);
        }
        if let Some(i) = focused {
            self.set_front(i);
        }
        self.handle_requests(&ctx);

        // The + in a tab bar: a new document as the next tab.
        if crate::mac::take_new_tab() {
            let parent = self.windows[self.front].doc_id.clone();
            self.open_window(Action::New);
            self.tab_into = Some((self.windows[self.front].doc_id.clone(), parent));
        }
        if let Some((child, parent)) = self.tab_into.clone() {
            let find = |id: &str| self.windows.iter().position(|w| w.doc_id == id);
            match (find(&child), find(&parent)) {
                (Some(c), Some(p)) => {
                    if crate::mac::add_tab(&self.windows[p], &self.windows[c]) {
                        self.tab_into = None;
                    }
                }
                _ => self.tab_into = None,
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::App::save(&mut self.windows[self.front], storage);
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}
