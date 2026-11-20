//! The application shell: owns the `Editor`, routes every UI action through
//! the zebra-core command bus, pumps the automation API, and lays out panels.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use base64::Engine;
use egui::{Context, Pos2};
use zebra_core::view::Blank;
use zebra_core::{
    Command, Editor, ElementId, ElementKind, Mode, NewElement, Origin, StepTarget, Tool, Vec2, ViewCommand,
    ZOrderOp,
};
use zebra_render::Renderer;

use crate::actions::Action;
use crate::canvas::CanvasState;
use crate::thumbs::Thumbs;

const H: Origin = Origin::Human;
pub const HUMAN: Origin = Origin::Human;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum ScrollMode {
    /// Trackpad scroll pans, pinch zooms; a mouse wheel zooms.
    #[default]
    Auto,
    /// Scrolling always zooms.
    Zoom,
    /// Scrolling always pans; ⌘-scroll or pinch zooms.
    Pan,
}

#[derive(Default)]
pub struct AppOptions {
    pub open: Option<PathBuf>,
    /// Dev: `--api` switches AI control on at launch (socket/permission
    /// overrides). End users use the AI button instead.
    pub api: Option<zebra_api::ApiOptions>,
    /// Debug: (seconds, png path): screenshot the window then quit.
    pub screenshot: Option<(f64, PathBuf)>,
    /// Debug: start in a demo state (sample, pathedit, present, agent, text).
    pub demo: Option<String>,
    /// Tests: start like a launch with a window (last run's untitled
    /// documents come back, the template chooser is offered) without a GPU.
    pub as_launch: bool,
}

impl AppOptions {
    /// A launch with nothing to open offers the template chooser (unless
    /// last run's untitled documents come back). One with a file, from the
    /// command line or from Finder, just opens it.
    pub fn offers_chooser(&self) -> bool {
        self.open.is_none() && self.demo.is_none() && self.screenshot.is_none()
    }
}

#[derive(Clone)]
pub struct Toast {
    pub text: String,
    pub until: f64,
}

pub struct ZebraApp {
    pub editor: Editor,
    /// This document's id for agents ("d1", "d2", ...).
    pub doc_id: String,
    /// Which new document of this run this is (1, 2, ...), given by the
    /// shell while it has no file; 0 until then.
    pub untitled_no: u32,
    /// One window among several, run by [`crate::shell::Shell`]: New and
    /// Open go to a new window, closing closes only this one, and the shell
    /// answers the agent API for every window.
    pub hosted: bool,
    /// The document window the person is working in (owns the menu bar
    /// and Settings).
    pub front: bool,
    /// For the shell: actions that need a new window (New, Open...).
    pub spawn: Vec<Action>,
    /// From the shell: the files open in the other windows.
    pub open_elsewhere: Vec<PathBuf>,
    /// For the shell: close this window (its document is saved or discarded).
    pub close_me: bool,
    /// For the shell: the person chose Quit.
    pub quit_all: bool,
    /// For the shell: this window was asked to close as part of Quit.
    pub quit_pending: bool,
    /// The native window this document is in (the shell sets it).
    pub viewport: egui::ViewportId,
    /// The window is out of sight but has work that needs a frame: the
    /// next pass runs `frame` once anyway.
    pub wake_hidden: bool,
    /// The size this window opens at: that of the window it was made from,
    /// as in Keynote (without it every new window came up 800 by 600).
    pub open_size: Option<egui::Vec2>,
    /// Where this window was put and noted (see `window_frames`).
    pub placement: crate::window_frames::Placement,
    /// `editor.loads` when `after_load` last ran.
    pub seen_loads: u64,
    /// When decoded pictures were last trimmed.
    pub last_image_trim: f64,
    pub renderer: Arc<Renderer>,
    pub wgpu: Option<egui_wgpu::RenderState>,
    pub gpu_canvas: Option<zebra_render::gpu::VelloCanvas>,
    /// False when the renderer can't draw (falls back to the egui painter).
    pub render_ok: bool,
    /// Shared by every window (one socket for the whole app).
    pub api: Option<Arc<zebra_api::ApiServer>>,
    pub dispatcher: Option<zebra_api::Dispatcher>,
    pub api_error: Option<String>,
    /// Socket override from `--api-socket` (dev); used whenever AI control
    /// is switched on.
    pub api_socket: Option<PathBuf>,
    /// AI control is on right now ("Allow AI control").
    pub ai_enabled: bool,
    /// The person's saved choice, restored at the next launch. Only the
    /// person's own toggle (and an agent's `disconnect`) changes it; a
    /// developer `--api` launch never does.
    pub ai_saved: bool,
    /// An agent called `disconnect`: switch off at this time (after its reply
    /// has been written).
    pub ai_off_at: Option<f64>,
    /// Debug/demo: open the AI popover on the next frame.
    pub open_ai_panel: bool,
    pub ctx: Context,
    pub t0: Instant,
    pub canvas: CanvasState,
    pub thumbs: Thumbs,
    pub show_sidebar: bool,
    pub show_props: bool,
    /// Path sidebar and inspector widths, set by dragging their edges;
    /// remembered between launches.
    pub sidebar_w: f32,
    pub inspector_w: f32,
    /// The inspector tab last chosen (kept across selections when it fits).
    pub inspector_tab: crate::inspector::Tab,
    pub show_activity: bool,
    pub palette: Option<(String, usize)>,
    /// Where the sidebar drew each step's row in the last frame.
    pub step_rows: Vec<egui::Rect>,
    /// How wide the sidebar drew the step number column in the last frame.
    pub step_number_w: f32,
    pub show_shortcuts: bool,
    pub library: crate::shape_library::Library,
    pub show_about: bool,
    pub selected_step: Option<usize>,
    /// The path list has the keys (it was clicked last, not the canvas):
    /// Keynote's navigator model, spec 08. See `sidebar::list_focused`.
    pub path_focus: bool,
    /// Several steps selected in the path list (⇧/⌘-click, Select All).
    pub step_multi: Option<crate::sidebar::StepMulti>,
    /// An empty step waiting for a frame (insert between steps). Interface
    /// state only: never saved, never seen by agents.
    pub step_pick: Option<crate::sidebar::Pick>,
    /// Objects copied in any window (shared by all of them).
    pub clipboard: Arc<zebra_core::clip::Clip>,
    /// The text Copy put on the system clipboard: while it's still there,
    /// Paste uses Zebra's own copy.
    pub clipboard_marker: Option<String>,
    /// Cells copied from a table: the text put on the clipboard and the
    /// cells' inputs, so pasting them back into a table keeps formulas.
    pub table_clip: Option<crate::table_ui::TableClip>,
    pub paste_count: u32,
    pub recent: Vec<PathBuf>,
    pub toast: Option<Toast>,
    /// A short note under the toolbar about what just happened on the
    /// canvas ("2 objects no longer move with this frame.").
    pub note: Option<Toast>,
    /// The sheet waiting for an answer (save changes, errors, confirmations).
    pub sheet: Option<crate::sheets::SheetKind>,
    /// What the beta-features offer carries on with after Turn On.
    pub beta_pending: Option<crate::beta_ui::Pending>,
    /// File › Import… under way (its sheet is up).
    pub import: Option<crate::import_ui::Flow>,
    /// Insert › Web Video…'s sheet.
    pub web_sheet: Option<crate::web_video_ui::Sheet>,
    /// Use native NSAlert sheets and system dialogs (macOS with a window);
    /// off in UI tests, where sheets are drawn in egui.
    pub native_dialogs: bool,
    /// The native sheet on screen, waiting for its answer (macOS).
    #[cfg(target_os = "macos")]
    pub open_sheet: Option<crate::sheets::OpenSheet>,
    /// An open or save panel asked for: it goes up as a sheet on this
    /// window once the frame is drawn (see `pickers`).
    pub want_pick: Option<crate::pickers::Pick>,
    /// The pass in which `want_pick` was set: the panel waits until a
    /// frame without the menu that asked for it has been drawn.
    pub want_pick_pass: u64,
    #[cfg(target_os = "macos")]
    pub open_picker: Option<crate::sheets::OpenPicker>,
    pub show_settings: bool,
    /// Debug: draw the icon sheet (`--demo icons`).
    pub show_icon_sheet: bool,
    pub settings_pane: crate::settings::Pane,
    /// Settings › Keyboard: recording a new shortcut (all shortcuts are off).
    pub key_capture: Option<crate::keymap::Capture>,
    /// Settings › Keyboard: what the last change did ("⌘L was Lock").
    pub key_message: Option<String>,
    /// Theme for File › New (Settings › General).
    pub default_theme: String,
    /// Snap to other objects while moving (Settings › Canvas).
    pub snapping: bool,
    /// macOS window state last pushed to AppKit.
    pub mac_sync: crate::mac::WindowSync,
    /// Step that was just added: its sidebar row highlights briefly.
    pub flash_step: Option<(usize, f64)>,
    /// The step nearest the camera, with what it was worked out for
    /// (revision, loads, camera and viewport bits): the sidebar asks for it
    /// a few times a frame, and every frame while idle.
    nearest_step: std::cell::Cell<Option<((u64, u64, [u64; 6]), Option<usize>)>>,
    /// Every step's camera for the revision, loads and viewport it was
    /// built for: `nearest_step` searches it.
    step_index: std::cell::RefCell<Option<((u64, u64, [u64; 2]), zebra_core::step_index::StepIndex)>>,
    pub queued: Vec<Action>,
    /// Faces of font families (the Weight pop-up, the inspector's Font
    /// line), per family, for the document as last opened.
    font_faces: std::cell::RefCell<(u64, std::collections::HashMap<String, Vec<zebra_core::FontFace>>)>,
    /// Actions held while the document is being saved (Close, Quit, Open,
    /// another Save, and what a Save Changes sheet's Save goes on to do).
    /// They run when the save is done, and are dropped if it failed.
    pub after_save: Vec<Action>,
    /// A big file being opened on a worker thread.
    pub opening: Option<crate::opening::Opening>,
    pub scroll_mode: ScrollMode,
    /// Reduce Motion chosen in Settings; `None` follows the system.
    pub motion_override: Option<bool>,
    pub last_autosave: f64,
    pub autosaved_rev: u64,
    pub autosave_job: Arc<crate::files::AutosaveJob>,
    pub recovered: Option<PathBuf>,
    /// This window's untitled autosave is `<name>.zebra.autosave`.
    pub autosave_name: String,
    /// Untitled documents from the last run to reopen, each in its own
    /// window (the first in this one).
    pub reopen_untitled: Vec<PathBuf>,
    /// Untitled documents the last launch quit while opening: each is
    /// offered in a sheet instead of reopened (Open Anyway, Move to Bin,
    /// Keep for Later).
    pub crashed_untitled: Vec<PathBuf>,
    /// The marker of the autosave this window reopened, and when; removed
    /// once the document has been shown for `files::OPENING_SECS`.
    pub opening_marker: Option<(PathBuf, f64)>,
    /// How many of those have been asked about (to number "Untitled 2").
    pub crashed_seq: u32,
    /// The highest "Untitled" number the shell has given a window: crashed
    /// documents are numbered after it, so none shares a name with an open
    /// window. 0 until the shell has numbered the windows.
    pub untitled_floor: u32,
    /// An agent made this document: its window opens without taking focus.
    pub opened_by_agent: bool,
    pub was_presenting: bool,
    pub force_quit: bool,
    pub title_buf: Option<String>,
    /// The pass the title field was last drawn in.
    pub title_seen: u64,
    pub last_title: String,
    pub screenshot: Option<(f64, PathBuf)>,
    pub screenshot_requested: bool,
    pub demo: Option<String>,
    pub demo_done: bool,
    /// Agent name to show when no API client name is known (demo/tests).
    pub demo_agent: Option<String>,
    /// Events to inject into egui next frame (menu → text field copy/paste).
    pub inject: Vec<egui::Event>,
    /// Builds revealed while presenting: id -> time revealed (fade-in).
    pub revealed: HashMap<ElementId, f64>,
    pub prev_hidden: HashSet<ElementId>,
    pub last_pointer_move: f64,
    pub last_pointer: Option<Pos2>,
    /// Presenter view, playback options, notes typing, play display.
    pub stage: crate::presenting::Stage,
    pub theme_ed: Option<crate::theme_editor::ThemeEditor>,
    /// The template chooser, when open.
    pub welcome: Option<crate::welcome::Welcome>,
    /// Template and layout previews (rendered once, on demand).
    pub previews: Option<crate::welcome::Previews>,
    pub show_layouts: bool,
    /// Insert › Diagram… gallery.
    pub show_diagrams: bool,
    /// Help › Acknowledgements.
    pub show_credits: bool,
    /// The Symbols tab's tiles and search.
    pub symbols: crate::symbols_ui::Symbols,
    #[cfg(target_os = "macos")]
    pub menu: Option<crate::menu_mac::MacMenu>,
    /// Video and audio players, crop mode, clipboard (session state).
    pub media: crate::media_ui::Media,
}

/// A window's name for a document never saved: the second new document
/// of a run is "Untitled 2", as in Keynote (two were both "Untitled").
/// A title the person typed is left alone.
pub fn untitled_name(title: &str, untitled: &str, no: u32) -> String {
    let title = if title.is_empty() { untitled } else { title };
    if no > 1 && title == untitled { crate::t!("app-untitled-numbered", title = title, no = no.to_string()) } else { title.to_string() }
}

/// The size a window made from this one opens at: the same, unless this one
/// is full screen (there macOS decides: a tab, or its own default).
pub fn open_size_from(from: &egui::ViewportInfo) -> Option<egui::Vec2> {
    if from.fullscreen == Some(true) {
        return None;
    }
    from.inner_rect.map(|r| r.size())
}

impl ZebraApp {
    /// Build the app. `wgpu` is `None` in UI tests.
    pub fn new(ctx: &Context, wgpu: Option<egui_wgpu::RenderState>, storage: Option<&dyn eframe::Storage>, opts: AppOptions) -> Self {
        Self::build(ctx, wgpu, storage, opts, None)
    }

    /// Another document window next to this one: same renderer and
    /// picture cache, an empty document, no template chooser. The shell
    /// copies the shared settings in.
    /// The panels that sit in the middle of the window, one at a time:
    /// opening one over another left two stacked (Acknowledgements on top
    /// of Diagrams).
    pub fn close_centre_panels(&mut self) {
        self.show_layouts = false;
        self.show_diagrams = false;
        self.show_credits = false;
    }

    pub fn new_window(&self, doc_id: String) -> Self {
        let mut w = Self::build(&self.ctx, self.wgpu.clone(), None, AppOptions::default(), Some(self.renderer.clone()));
        w.welcome = None;
        w.doc_id = doc_id;
        w.hosted = true;
        w.front = false;
        w.native_dialogs = self.native_dialogs;
        w.open_size = self.ctx.input(|i| i.raw.viewports.get(&self.viewport).and_then(open_size_from));
        w
    }

    fn build(ctx: &Context, wgpu: Option<egui_wgpu::RenderState>, storage: Option<&dyn eframe::Storage>, opts: AppOptions, shared: Option<Arc<Renderer>>) -> Self {
        let is_root = shared.is_none();
        let offers_chooser = opts.offers_chooser();
        let launch = is_root && (wgpu.is_some() || opts.as_launch) && opts.demo.is_none() && opts.screenshot.is_none();
        let renderer = match shared {
            Some(r) => r,
            None => {
                crate::style::install_fonts(ctx);
                crate::mac::refresh_accent();
                crate::style::apply_visuals(ctx);
                let renderer = Arc::new(Renderer::new());
                // Pictures decode on worker threads; repaint when one lands.
                let waker_ctx = ctx.clone();
                renderer.images().set_waker(Some(Arc::new(move || waker_ctx.request_repaint())));
                // Imported PDF pages are drawn again from the PDF when
                // zoomed in past their pictures, by the PDF helper
                // process (never PDFium in this one).
                renderer.set_page_raster(Some(Arc::new(|r: &zebra_render::pages::PageRequest| {
                    zebra_import::pdf::raster(&r.pdf, &r.bytes, r.index, r.page_px, r.rect)
                })));
                renderer
            }
        };
        let mut editor = Editor::new(new_document(&zebra_core::Document::default().theme));
        editor.labeler = Some(crate::words::phrase);
        editor.measure = renderer.measure();
        let recent: Vec<PathBuf> = storage.and_then(|s| eframe::get_value(s, "recent")).unwrap_or_default();
        let scroll_mode = storage.and_then(|s| eframe::get_value(s, "scroll_mode")).unwrap_or_default();
        // Reduce Motion follows System Settings › Accessibility unless the
        // person chose otherwise in Zebra's Settings.
        let motion_override: Option<bool> = storage.and_then(|s| eframe::get_value(s, "motion_override")).unwrap_or(None);
        editor.view.reduce_motion = motion_override.unwrap_or_else(crate::mac::system_reduce_motion);

        let has_window = wgpu.is_some();
        let mut app = ZebraApp {
            editor,
            doc_id: "d1".into(),
            untitled_no: 0,
            hosted: false,
            front: true,
            spawn: vec![],
            close_me: false,
            quit_all: false,
            quit_pending: false,
            viewport: egui::ViewportId::ROOT,
            wake_hidden: false,
            open_size: None,
            placement: Default::default(),
            seen_loads: 0,
            last_image_trim: 0.0,
            open_elsewhere: vec![],
            renderer: renderer.clone(),
            gpu_canvas: wgpu.as_ref().map(zebra_render::gpu::VelloCanvas::new),
            wgpu,
            render_ok: true,
            api: None,
            dispatcher: None,
            api_error: None,
            api_socket: None,
            ai_enabled: false,
            ai_saved: false,
            ai_off_at: None,
            open_ai_panel: false,
            ctx: ctx.clone(),
            t0: Instant::now(),
            canvas: CanvasState::default(),
            thumbs: Thumbs::new(ctx.clone(), renderer.clone()),
            stage: crate::presenting::Stage::new(ctx, renderer.clone(), storage),
            theme_ed: None,
            step_rows: vec![],
            step_number_w: 0.0,
            welcome: None,
            previews: None,
            show_layouts: false,
            show_diagrams: false,
            show_credits: false,
            symbols: Default::default(),
            show_sidebar: true,
            show_props: true,
            sidebar_w: storage.and_then(|s| eframe::get_value::<f32>(s, "sidebar_width")).map_or(crate::panels::SIDEBAR_W, crate::panels::clamp_sidebar),
            inspector_w: storage.and_then(|s| eframe::get_value::<f32>(s, "inspector_width")).map_or(crate::inspector::WIDTH, crate::panels::clamp_inspector),
            inspector_tab: crate::inspector::Tab::default(),
            show_activity: false,
            palette: None,
            show_shortcuts: false,
            library: Default::default(),
            show_about: false,
            selected_step: None,
            path_focus: false,
            step_multi: None,
            step_pick: None,
            clipboard: Default::default(),
            clipboard_marker: None,
            table_clip: None,
            paste_count: 0,
            recent,
            toast: None,
            note: None,
            sheet: None,
            beta_pending: None,
            import: None,
            web_sheet: None,
            native_dialogs: cfg!(target_os = "macos") && has_window,
            #[cfg(target_os = "macos")]
            open_sheet: None,
            want_pick: None,
            want_pick_pass: 0,
            #[cfg(target_os = "macos")]
            open_picker: None,
            show_settings: false,
            show_icon_sheet: false,
            settings_pane: storage.and_then(|s| eframe::get_value(s, "settings_pane")).unwrap_or_default(),
            key_capture: None,
            key_message: None,
            default_theme: storage.and_then(|s| eframe::get_value(s, "default_theme")).unwrap_or_else(|| zebra_core::Document::default().theme),
            snapping: storage.and_then(|s| eframe::get_value(s, "snapping")).unwrap_or(true),
            mac_sync: Default::default(),
            flash_step: None,
            nearest_step: Default::default(),
            step_index: Default::default(),
            queued: vec![],
            font_faces: Default::default(),
            after_save: vec![],
            opening: None,
            scroll_mode,
            motion_override,
            last_autosave: 0.0,
            autosaved_rev: 0,
            autosave_job: Default::default(),
            recovered: None,
            autosave_name: crate::files::new_autosave_name(),
            reopen_untitled: vec![],
            crashed_untitled: vec![],
            opening_marker: None,
            crashed_seq: 0,
            untitled_floor: 0,
            opened_by_agent: false,
            was_presenting: false,
            force_quit: false,
            title_buf: None,
            title_seen: 0,
            last_title: String::new(),
            screenshot: opts.screenshot,
            screenshot_requested: false,
            demo: opts.demo,
            demo_done: false,
            demo_agent: None,
            inject: vec![],
            revealed: HashMap::new(),
            prev_hidden: HashSet::new(),
            last_pointer_move: 0.0,
            last_pointer: None,
            #[cfg(target_os = "macos")]
            menu: None,
            media: Default::default(),
        };
        // The person's choice is kept with the grant (the keychain in a
        // signed Mac build, where nothing else can change it); the app's
        // own settings file only counts for older grants that didn't keep it.
        let switch = zebra_api::discovery::ai_switch();
        let saved_ai: bool = switch.on.unwrap_or_else(|| storage.and_then(|s| eframe::get_value(s, "ai_enabled")).unwrap_or(false));
        app.ai_saved = saved_ai;
        // Beta features: the person's choice, off until they turn it on.
        app.editor.view.beta_features = crate::beta_ui::saved(storage);
        // The first time is always a person: until someone has switched AI
        // control on in this window, neither a launch flag nor a saved
        // setting turns it on. Once switched off (by the person, or an
        // agent's `disconnect`), a launch flag can't switch it back on.
        let granted = switch.granted;
        if let Some(api_opts) = opts.api.filter(|_| switch.lets_flags_start()) {
            app.api_socket = api_opts.socket.clone();
            // The person's choice; the flag only ever lowers it.
            let saved = storage.and_then(|s| eframe::get_value::<String>(s, "agent_permission")).map(|p| parse_permission(&p));
            app.editor.view.agent_permission = launch_permission(saved, api_opts.permission);
            app.set_ai_control(true);
        } else if saved_ai && granted {
            if let Some(p) = storage.and_then(|s| eframe::get_value::<String>(s, "agent_permission")) {
                app.editor.view.agent_permission = parse_permission(&p);
            }
            app.set_ai_control(true);
        }
        let launch_welcome = launch && offers_chooser;
        // Like Keynote: untitled documents the last run left unsaved come
        // back, still unsaved, unless the Mac closes windows at quit. A
        // file the launch was for (command line, Finder) opens beside them
        // in a window of its own, in front: queued here, the shell gives
        // it that window. With nothing to bring back, the file takes this
        // window, or a fresh launch offers the template chooser.
        if launch && crate::mac::restores_windows() {
            // One the last launch died opening is asked about, not reopened.
            let (left, crashed) = crate::files::split_crashed(crate::files::leftover_untitled());
            app.crashed_untitled = crashed;
            let mut left = left.into_iter();
            while let Some(p) = left.next() {
                if app.reopen_autosave(&p) {
                    app.reopen_untitled = left.collect();
                    app.queued.extend(opts.open.map(Action::OpenPath));
                    return app;
                }
            }
        }
        if let Some(p) = opts.open {
            app.open_path(&p);
        }
        if launch_welcome && app.recovered.is_none() && storage.and_then(|s| s.get_string("welcome_at_launch")).is_none_or(|v| v != "false") {
            app.welcome = Some(crate::welcome::Welcome::new());
        }
        app
    }

    pub fn now(&self) -> f64 {
        self.t0.elapsed().as_secs_f64()
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        let until = self.now() + 2.6;
        self.toast = Some(Toast { text: text.into(), until });
    }

    /// Show a short note under the toolbar for a few seconds.
    pub fn note(&mut self, text: impl Into<String>) {
        let until = self.now() + 4.0;
        self.note = Some(Toast { text: text.into(), until });
    }

    /// Switch "Allow AI control" on or off. Only the human UI
    /// calls this with `true`; agents can only reach the `false` side (the
    /// `disconnect` API method). Off closes the socket, disconnects every
    /// agent and removes the discovery file, so nothing can find Zebra.
    pub fn set_ai_control(&mut self, on: bool) {
        self.ai_off_at = None;
        if !on {
            if let Some(d) = &mut self.dispatcher {
                d.shutdown(&mut self.editor);
            }
            self.dispatcher = None;
            self.api = None;
            self.api_error = None;
            self.ai_enabled = false;
            self.editor.view.agent_paused = false;
            return;
        }
        self.ai_enabled = true;
        if self.api.is_some() {
            return;
        }
        let c = self.ctx.clone();
        let waker: zebra_api::Waker = Arc::new(move || c.request_repaint());
        let opts = zebra_api::ApiOptions { socket: self.api_socket.clone(), permission: self.editor.view.agent_permission };
        match zebra_api::ApiServer::start(opts, waker) {
            Ok(s) => {
                // Hosted windows are answered by the shell's dispatcher.
                if !self.hosted {
                    let mut d = zebra_api::Dispatcher::new(self.renderer.clone());
                    d.web_labels = crate::files::web_labels();
                    self.dispatcher = Some(d);
                }
                self.api = Some(Arc::new(s));
                self.api_error = None;
            }
            Err(e) => {
                // Also on stderr: with --background nobody may see the window.
                eprintln!("zebra: AI control could not start: {e}");
                self.api_error = Some(crate::t!("app-ai-start-failed", error = e.to_string()));
            }
        }
    }

    /// Report a core error to the human (agents get them over the API).
    pub fn report<T>(&mut self, r: zebra_core::ZResult<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                // Core's messages and hints are written for agents ("unlock it
                // first: update {patch:...}"). Log them; show the person a
                // translated line for the kind of error.
                eprintln!("zebra: {}: {}{}", e.code, e.message, e.hint.as_ref().map(|h| format!(" ({h})")).unwrap_or_default());
                self.toast(crate::words::error(&e));
                None
            }
        }
    }

    pub fn apply(&mut self, cmd: Command) -> Option<zebra_core::Outcome> {
        let r = self.editor.apply(cmd, H);
        self.report(r)
    }

    pub fn view_cmd(&mut self, cmd: ViewCommand) {
        let r = self.editor.view(cmd, H);
        self.report(r);
    }

    pub fn select(&mut self, ids: Vec<ElementId>) {
        self.view_cmd(ViewCommand::Select { ids, frame_only: false });
    }

    pub fn selection(&self) -> Vec<ElementId> {
        self.editor.view.selection.clone()
    }

    /// Selection minus locked elements (what move/delete act on).
    pub fn editable_selection(&self) -> Vec<ElementId> {
        self.editor.view.selection.iter().filter(|id| self.editor.doc.get(id).is_some_and(|e| !e.locked)).cloned().collect()
    }

    pub fn agent_connected(&self) -> bool {
        self.api.as_ref().is_some_and(|a| !a.clients().is_empty()) || self.demo_agent.is_some()
    }

    /// The agent to name in the status bar and AI popover: the connected
    /// agent that changed something most recently, else the first one
    /// connected. With several agents connected, the name always matches
    /// the latest action shown next to it.
    pub fn agent_name(&self) -> String {
        let clients = self.api.as_ref().map(|a| a.clients()).unwrap_or_default();
        let latest = self.editor.activity.iter().rev().find_map(|a| match &a.origin {
            Origin::Agent { name } if clients.iter().any(|c| c.name == *name) => Some(name.clone()),
            _ => None,
        });
        if let Some(n) = latest.or_else(|| clients.into_iter().next().map(|c| c.name)) {
            return n;
        }
        if let Some(n) = &self.demo_agent {
            return n.clone();
        }
        self.editor
            .activity
            .iter()
            .rev()
            .find_map(|a| match &a.origin {
                Origin::Agent { name } => Some(name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| crate::t!("app-agent-default-name"))
    }

    // ------------------------------------------------------------------
    // Actions

    /// The upright faces of a font family, lightest first (cached: an
    /// installed font's files are read once).
    pub fn font_faces(&self, family: &str) -> Vec<zebra_core::FontFace> {
        let mut c = self.font_faces.borrow_mut();
        if c.0 != self.editor.loads {
            *c = (self.editor.loads, Default::default());
        }
        c.1.entry(family.to_string()).or_insert_with(|| self.editor.measure.font_faces(&self.editor.doc, family)).clone()
    }

    pub fn perform(&mut self, ctx: &Context, a: Action) {
        use Action::*;
        // A sheet or the template chooser has the window: what the
        // document would do (Paste, Delete, Undo…) must not reach the
        // document behind it. Opening, closing and quitting still work.
        if self.sheet_up() && !works_under_a_sheet(&a) {
            return;
        }
        // Commit any in-place text edit first so actions see the final text
        // (Move to New Text Box needs the selection in it, and commits it).
        if !matches!(a, Undo | Redo | MoveToNewTextBox) {
            crate::canvas::commit_text_edit(self);
        }
        let is_quit = a == Quit;
        self.perform_inner(ctx, a);
        if !is_quit {
            self.force_quit = false;
        }
    }

    fn perform_inner(&mut self, ctx: &Context, a: Action) {
        use Action::*;
        // Cells selected in a table: the clipboard, Delete, Esc and Select
        // All act on the cells.
        if crate::table_ui::perform(self, &a) {
            return;
        }
        match a {
            // Like Keynote: a new or opened document gets its own window,
            // unless this one is still an untouched new document.
            New | NewFromSample | NewFromTemplate(_) | NewFromTemplateFile(_) | OpenPath(_) | ReopenAutosave(_) if self.hosted && !self.is_pristine() => {
                self.spawn.push(a);
            }
            // A file another window has open: the shell shows that window
            // (two windows on one file would overwrite each other's saves).
            OpenPath(ref p) if self.hosted && self.open_elsewhere.contains(&same_file(p)) => {
                self.spawn.push(a);
            }
            Quit if self.hosted => self.quit_all = true,
            // The document is being written: closing it, replacing it or
            // saving it again waits for that save, then asks afresh.
            New | NewFromSample | NewFromTemplate(_) | NewFromTemplateFile(_) | Open | OpenPath(_) | Quit | Close | Save | SaveAs if self.editor.saving().is_some() => {
                self.after_save.push(a);
            }
            // As in Keynote: a document that has a file is saved in place,
            // without asking, and the action follows once it's written. Only
            // an Untitled one asks Save Changes.
            New | NewFromSample | NewFromTemplate(_) | NewFromTemplateFile(_) | Open | OpenPath(_) | Quit | Close
                if self.editor.dirty() && !self.force_quit && self.editor.file_path.is_some() && !self.asking_to_save() =>
            {
                if self.save(false) {
                    self.after_save.push(a);
                }
            }
            New | NewFromSample | NewFromTemplate(_) | NewFromTemplateFile(_) | Open | OpenPath(_) | Quit | Close if self.editor.dirty() && !self.force_quit => {
                // Already asking (a second Close, from the close button
                // say): that question stands. It used to fall through and
                // close the window unasked (B4).
                if !self.asking_to_save() {
                    crate::sheets::request(self, crate::sheets::SheetKind::SaveChanges(a));
                }
            }
            Close if self.hosted => {
                // A show ends with its window: the menu bar and Dock come
                // back, and the next document doesn't inherit the stage.
                if self.editor.view.mode == Mode::Present {
                    let _ = self.editor.view(ViewCommand::ExitPresent, H);
                    self.sync_present_mode(ctx);
                }
                self.close_me = true;
                self.media.stop_all();
            }
            // A single window (tests): closing it quits.
            Close => {
                self.force_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            New => {
                let doc = new_document(&self.default_theme);
                self.editor.load(doc, None);
                self.after_load();
            }
            ShowTemplates => self.welcome = Some(crate::welcome::Welcome::new()),
            NewFromTemplate(t) => crate::welcome::new_from_template(self, &t),
            NewFromTemplateFile(p) => crate::welcome::new_from_template_file(self, &p),
            SaveAsTemplate => crate::welcome::save_as_template(self),
            InsertLayout => {
                self.close_centre_panels();
                self.show_layouts = true
            }
            InsertDiagram => {
                self.close_centre_panels();
                self.show_diagrams = true
            }
            InsertSymbols => self.library.open_symbols(),
            ThemeEditor => self.theme_ed = Some(crate::theme_editor::ThemeEditor::open(self)),
            NewFromSample => {
                let doc = crate::sample::build(&*self.renderer);
                self.editor.load(doc, None);
                self.after_load();
            }
            Open => crate::pickers::ask(self, crate::pickers::Pick::Open),
            OpenPath(p) => self.open_path(&p),
            ReopenAutosave(p) => {
                if !self.reopen_autosave(&p) {
                    let title = crate::t!("app-open-failed-title", name = crate::t!("app-untitled"));
                    crate::sheets::request(self, crate::sheets::SheetKind::Alert { title, message: crate::t!("app-open-failed-not-zebra") });
                }
            }
            Save => {
                self.save(false);
            }
            SaveAs => {
                self.save(true);
            }
            ExportPdf => crate::files::export_pdf(self),
            ExportPng => crate::files::export_png(self),
            ExportHtml => crate::files::export_html(self),
            Import => crate::import_ui::choose_file(self),
            Quit => {
                self.force_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Undo => {
                // Text still being typed is kept as its own step, which this
                // then undoes (Redo brings it back); it used to be thrown
                // away, and the step before it undone as well.
                crate::canvas::commit_text_edit(self);
                // The Edit menu names the action ("Undo Rotate"); no toast.
                let r = self.editor.undo(H);
                self.report(r);
            }
            Redo => {
                let r = self.editor.redo(H);
                self.report(r);
            }
            Copy | Cut => {
                let ids = self.editor.expand(&self.selection(), true);
                self.clipboard = Arc::new(zebra_core::clip::Clip::of(&self.editor, &ids));
                self.paste_count = 0;
                if !ids.is_empty() {
                    crate::media_ui::copied_to_system(self, &ids);
                }
                if a == Cut {
                    let del = self.editable_selection();
                    if !del.is_empty() {
                        self.apply(Command::Delete { ids: del });
                    }
                }
            }
            Paste => self.paste(),
            Duplicate => {
                let ids = self.selection();
                if !ids.is_empty() {
                    let d = 24.0 / self.editor.view.camera.zoom;
                    if let Some(out) = self.apply(Command::Duplicate { ids, dx: d, dy: d }) {
                        let top: Vec<_> = self.top_level(&out.created);
                        self.select(top);
                    }
                }
            }
            // Keys and menus go to the pane clicked last (spec 08).
            Delete if self.step_pick.is_some() && self.path_focus => crate::sidebar::cancel_pick(self),
            Delete if crate::sidebar::list_focused(self) => crate::sidebar::delete_selected(self),
            Delete => {
                let ids = self.editable_selection();
                if !ids.is_empty() {
                    self.apply(Command::Delete { ids });
                }
            }
            SelectAll if crate::sidebar::list_focused(self) => crate::sidebar::select_all_steps(self),
            SelectAll => self.view_cmd(ViewCommand::SelectAll),
            SelectFrameOnly => {
                if let [id] = self.selection().as_slice() {
                    crate::canvas::select_frame_only(self, id.clone());
                }
            }
            Deselect if self.step_pick.is_some() => crate::sidebar::cancel_pick(self),
            Deselect if crate::sidebar::list_focused(self) => self.path_focus = false,
            Deselect => {
                if self.editor.view.mode == Mode::PathEdit {
                    self.view_cmd(ViewCommand::SetMode { mode: Mode::Edit });
                } else if self.editor.view.tool != Tool::Select {
                    self.view_cmd(ViewCommand::SetTool { tool: Tool::Select });
                } else {
                    self.view_cmd(ViewCommand::ClearSelection);
                }
            }
            SetTool(t) => self.view_cmd(ViewCommand::SetTool { tool: t }),
            FrameTool => {
                if self.editor.view.tool == Tool::Frame {
                    let s = self.editor.view.frame_style.next();
                    self.view_cmd(ViewCommand::SetFrameStyle { style: s });
                } else {
                    self.view_cmd(ViewCommand::SetTool { tool: Tool::Frame });
                }
            }
            SetFrameStyle(s) => {
                self.view_cmd(ViewCommand::SetFrameStyle { style: s });
                self.view_cmd(ViewCommand::SetTool { tool: Tool::Frame });
            }
            InsertFrame => crate::canvas::insert_frame(self),
            InsertVideo => crate::media_ui::insert_video_dialog(self),
            InsertWebVideo => crate::web_video_ui::open(self),
            InsertMusic => crate::media_ui::soundtrack_dialog(self),
            ReplaceImage => {
                if let [id] = self.selection().as_slice() {
                    crate::media_ui::replace_image_dialog(self, id);
                }
            }
            CropImage => {
                if let [id] = self.selection().as_slice() {
                    crate::media_ui::begin_crop(self, id);
                }
            }
            PlayPauseVideo => {
                if let [id] = self.selection().as_slice() {
                    crate::media_ui::toggle(self, id);
                }
            }
            InsertTable => crate::table_ui::insert_table(self, zebra_core::table::TableStyle::default()),
            InsertImage => {
                let c = self.canvas_center_world();
                crate::pickers::ask(self, crate::pickers::Pick::InsertImage(c));
            }
            WrapInFrame => {
                let ids = self.selection();
                if !ids.is_empty() {
                    let style = self.editor.view.frame_style;
                    if let Some(out) = self.apply(Command::WrapInFrame { ids, style, padding: 0.12, add_to_path: false }) {
                        self.select(out.created);
                    }
                }
            }
            BringForward => self.zorder(ZOrderOp::Forward),
            SendBackward => self.zorder(ZOrderOp::Backward),
            BringToFront => self.zorder(ZOrderOp::Front),
            SendToBack => self.zorder(ZOrderOp::Back),
            ToggleLock => {
                let sel = self.selection();
                if !sel.is_empty() {
                    let lock = !sel.iter().all(|id| self.editor.doc.get(id).is_some_and(|e| e.locked));
                    let cmds = sel
                        .into_iter()
                        .map(|id| Command::Update { id, patch: zebra_core::ElementPatch { locked: Some(lock), ..Default::default() } })
                        .collect();
                    let r = self.editor.apply_batch(cmds, Some(if lock { crate::t!("app-undo-lock") } else { crate::t!("app-undo-unlock") }), H);
                    self.report(r);
                }
            }
            ZoomIn => self.view_cmd(ViewCommand::Zoom { factor: 1.25, anchor: None }),
            ZoomOut => self.view_cmd(ViewCommand::Zoom { factor: 0.8, anchor: None }),
            FitAll => self.view_cmd(ViewCommand::FitAll { animate: true }),
            ZoomToSelection => {
                if !self.editor.view.selection.is_empty() {
                    self.view_cmd(ViewCommand::ZoomToSelection)
                }
            }
            ZoomReset => {
                let mut c = self.editor.view.target_camera();
                c.zoom = 1.0;
                c.rotation = 0.0;
                self.view_cmd(ViewCommand::SetCamera { camera: c, animate: true });
            }
            ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            ToggleProperties => self.show_props = !self.show_props,
            ToggleReduceMotion => {
                let on = !self.editor.view.reduce_motion;
                self.motion_override = Some(on);
                self.view_cmd(ViewCommand::SetReduceMotion { on });
            }
            CommandPalette => self.palette = Some((String::new(), 0)),
            ShowShortcuts => self.show_shortcuts = !self.show_shortcuts,
            TogglePathEdit => {
                let m = if self.editor.view.mode == Mode::PathEdit { Mode::Edit } else { Mode::PathEdit };
                self.view_cmd(ViewCommand::SetMode { mode: m });
            }
            AddToPath => self.add_to_path(),
            AddCurrentView => self.add_current_view(),
            AutoPath => {
                // Notes on steps the new path drops: ask first (like Clear Path).
                match self.editor.apply(Command::AutoPath { discard_notes: false }, H) {
                    Err(e) if e.code == "notes_lost" => crate::sheets::request(self, crate::sheets::SheetKind::AutoPath),
                    r => {
                        self.report(r);
                    }
                }
            }
            ClearPath => {
                if !self.editor.doc.path.is_empty() {
                    crate::sheets::request(self, crate::sheets::SheetKind::ClearPath);
                }
            }
            PresentStart | PresentCurrent if self.editor.doc.elements.is_empty() => {
                // The Play button is disabled; this covers the shortcut.
                self.toast(crate::t!("app-toast-nothing-to-play"));
            }
            PresentStart => {
                // From the first step, every video card starts over.
                self.media.card_resume.clear();
                self.view_cmd(ViewCommand::Present { step: None })
            }
            PresentCurrent => {
                let step = self.current_step();
                self.view_cmd(ViewCommand::Present { step });
            }
            TogglePresenterView => {
                self.stage.presenter_view = !self.stage.presenter_view;
                self.toast(if self.stage.presenter_view { crate::t!("app-toast-presenter-view-on") } else { crate::t!("app-toast-presenter-view-off") });
            }
            PlaybackOptions => self.stage.show_playback = true,
            ToggleAiControl => {
                // Only the person reaches this (toolbar, menu, Settings).
                let on = !self.ai_enabled;
                if let Err(e) = zebra_api::discovery::record_ai_switch(on) {
                    eprintln!("zebra: couldn't keep the AI control switch: {e}");
                }
                self.ai_saved = on;
                self.set_ai_control(on);
            }
            ToggleActivity => self.show_activity = !self.show_activity,
            PauseAgents => {
                let p = !self.editor.view.agent_paused;
                self.view_cmd(ViewCommand::PauseAgents { paused: p });
            }
            StopAgents => {
                self.view_cmd(ViewCommand::PauseAgents { paused: true });
                self.toast(crate::t!("app-toast-agent-stopped"));
            }
            FollowAgent => {
                let on = !self.editor.view.follow_agent;
                self.view_cmd(ViewCommand::FollowAgent { on });
            }
            RevertAgent => {
                let name = self.agent_name();
                crate::sheets::request(self, crate::sheets::SheetKind::RevertAgent { name });
            }
            SetAgentPermission(perm) => {
                let cur = self.editor.view.agent_permission;
                if perm == zebra_core::view::Permission::Full && cur != perm {
                    crate::sheets::request(self, crate::sheets::SheetKind::RaisePermission);
                } else {
                    let _ = self.editor.view(ViewCommand::SetAgentPermission { permission: perm }, H);
                }
            }
            Settings => self.show_settings = true,
            ShapeLibrary => self.library.toggle(),
            Bold | Italic | Underline if self.canvas.text.is_some() => self.canvas.text_menu = Some(a.clone()),
            MoveToNewTextBox => crate::text_drag::move_to_new_box(self),
            Group => {
                let ids = self.editable_selection();
                let members = self.editor.doc.expand_groups(&ids);
                if members.len() >= 2 {
                    if self.apply(Command::Group { ids, id: None }).is_some() {
                        self.select(members);
                    }
                }
            }
            Ungroup => {
                let ids = self.editable_selection();
                if ids.iter().any(|id| self.editor.doc.get(id).is_some_and(|e| e.group.is_some())) {
                    self.apply(Command::Ungroup { ids });
                }
            }
            Underline => {
                // Whole boxes: underline every selected text box, or remove
                // it if all of them are already underlined.
                let ids: Vec<ElementId> = self
                    .editable_selection()
                    .into_iter()
                    .filter(|id| matches!(self.editor.doc.get(id).map(|e| &e.kind), Some(ElementKind::Text(_))))
                    .collect();
                if !ids.is_empty() {
                    let all_on = ids.iter().all(|id| match self.editor.doc.get(id).map(|e| &e.kind) {
                        Some(ElementKind::Text(t)) => !t.spans.is_empty() && t.spans.iter().all(|sp| sp.format.underline == Some(true)),
                        _ => false,
                    });
                    let cmds = ids
                        .into_iter()
                        .map(|id| Command::FormatText {
                            id,
                            start: None,
                            end: None,
                            find: None,
                            format: zebra_core::SpanFormat { underline: Some(!all_on), ..Default::default() },
                        })
                        .collect();
                    let r = self.editor.apply_batch(cmds, Some(a.label()), H);
                    self.report(r);
                }
            }
            Bold | Italic => {
                let ids: Vec<ElementId> = self
                    .editable_selection()
                    .into_iter()
                    .filter(|id| matches!(self.editor.doc.get(id).map(|e| &e.kind), Some(ElementKind::Text(_))))
                    .collect();
                if !ids.is_empty() {
                    let theme = self.editor.theme();
                    let all_on = ids.iter().all(|id| match self.editor.doc.get(id).map(|e| &e.kind) {
                        // Bold or heavier counts as on: in a Semibold style the
                        // B is on, and Bold turns the words Regular (Keynote).
                        Some(ElementKind::Text(t)) => if a == Bold { zebra_core::rich::all_bold(t, theme.text_style(t.style).weight, 0, 0) } else { t.italic },
                        _ => false,
                    });
                    let mut cmds = vec![];
                    for id in ids {
                        let mut patch = zebra_core::ElementPatch::default();
                        if a == Italic {
                            patch.italic = Some(!all_on);
                            cmds.push(Command::Update { id, patch });
                            continue;
                        }
                        patch.bold = Some(!all_on);
                        // The box setting alone can't make a heavy style
                        // Regular, nor undo bold set on some words.
                        let heavy_or_mixed = match self.editor.doc.get(&id).map(|e| &e.kind) {
                            Some(ElementKind::Text(t)) => theme.text_style(t.style).weight >= zebra_core::theme::BOLD_FROM || t.spans.iter().any(|s| s.format.bold.is_some()),
                            _ => false,
                        };
                        cmds.push(Command::Update { id: id.clone(), patch });
                        if heavy_or_mixed {
                            cmds.push(Command::FormatText {
                                id,
                                start: None,
                                end: None,
                                find: None,
                                format: zebra_core::SpanFormat { bold: Some(!all_on), ..Default::default() },
                            });
                        }
                    }
                    let r = self.editor.apply_batch(cmds, Some(a.label()), H);
                    self.report(r);
                }
            }
            SetTheme(t) => {
                if t != self.editor.doc.theme {
                    self.apply(Command::SetTheme { theme: t });
                }
            }
            About => self.show_about = true,
            Acknowledgements => {
                self.close_centre_panels();
                self.show_credits = true
            }
        }
    }

    fn zorder(&mut self, order: ZOrderOp) {
        let ids = self.selection();
        if !ids.is_empty() {
            self.apply(Command::ZOrder { ids, order });
        }
    }

    /// Only ids whose parent frame is not also in the list (for selecting
    /// duplicates/pastes as the user sees them).
    pub fn top_level(&self, ids: &[ElementId]) -> Vec<ElementId> {
        let set: std::collections::HashSet<&ElementId> = ids.iter().collect();
        ids.iter()
            .filter(|id| !self.editor.parent_frame(id).is_some_and(|p| set.contains(&p)))
            .cloned()
            .collect()
    }

    fn paste(&mut self) {
        // Something newer on the system clipboard (a picture, files from
        // Finder, text from another app) wins over Zebra's own copy.
        if crate::media_ui::paste_from_system(self) {
            return;
        }
        self.paste_own();
    }

    /// Paste Zebra's own copied objects.
    pub fn paste_own(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        self.paste_count += 1;
        let d = 24.0 * self.paste_count as f64 / self.editor.view.camera.zoom;
        // If the copied things are off-screen, paste them at the view centre.
        let Some(b) = self.clipboard.bounds() else { return };
        let vis = self.editor.view.camera.visible_bounds(self.editor.view.viewport);
        let (dx, dy) = if vis.intersects(&b) {
            (d, d)
        } else {
            let c = vis.center() - b.center();
            (c.x, c.y)
        };
        let clip = self.clipboard.clone();
        let r = self.editor.paste_clip(&clip, dx, dy, Some(crate::t!("app-undo-paste")), H);
        if let Some(out) = self.report(r) {
            let top = self.top_level(&out.created);
            self.select(top);
        }
    }

    pub fn add_to_path(&mut self) {
        let sel = self.selection();
        if sel.is_empty() {
            self.add_current_view();
            return;
        }
        let top = self.top_level(&sel);
        let cmds: Vec<Command> =
            top.into_iter().map(|id| Command::AddStep { target: StepTarget::Element { id }, index: None, builds: vec![], notes: String::new() }).collect();
        let n = cmds.len();
        let r = self.editor.apply_batch(cmds, Some(crate::t!("app-undo-add-to-path")), H);
        if self.report(r).is_some() {
            // Show the change itself: the new step's row highlights.
            let last = self.editor.doc.path.len() - 1;
            self.selected_step = Some(last);
            self.flash_step = Some((last + 1 - n.min(last + 1), self.now()));
        }
    }

    /// "Add current view": an invisible frame around the view.
    pub fn add_current_view(&mut self) {
        let cam = self.editor.view.target_camera();
        let vr = cam.view_rect(self.editor.view.viewport);
        let cmd = Command::AddElement {
            element: NewElement {
                id: None,
                x: vr.cx,
                y: vr.cy,
                scale: 1.0 / cam.zoom,
                rotation: vr.rotation,
                w: Some(vr.width * cam.zoom * 0.96),
                h: Some(vr.height * cam.zoom * 0.96),
                name: String::new(),
                kind: ElementKind::Frame(zebra_core::FrameData { style: zebra_core::FrameStyle::Invisible, color: None, slide: false }),
            },
        };
        // The frame and its step are one thing to the person: one undo step.
        if let Err(e) = self.editor.begin(crate::t!("app-undo-add-current-view"), H) {
            self.report::<()>(Err(e));
            return;
        }
        let r = self.editor.apply(cmd, H).and_then(|out| {
            let id = out.created.first().cloned().ok_or_else(|| zebra_core::ZError::new("internal", "no frame was made"))?;
            self.editor.apply(Command::AddStep { target: StepTarget::Element { id }, index: None, builds: vec![], notes: String::new() }, H)
        });
        match r {
            Ok(_) => {
                self.editor.commit_as(&H);
                self.selected_step = Some(self.editor.doc.path.len() - 1);
                self.flash_step = Some((self.editor.doc.path.len() - 1, self.now()));
            }
            Err(e) => {
                self.editor.abort_as(&H);
                self.report::<()>(Err(e));
            }
        }
    }

    /// The step to present from: the one selected in the sidebar, else the
    /// step whose view is closest to the current camera.
    pub fn current_step(&self) -> Option<usize> {
        if self.editor.doc.path.is_empty() {
            return None;
        }
        // The step the sidebar highlights: from the canvas selection, else
        // the step clicked last.
        if let Some(s) = crate::sidebar::selected_steps(self).first() {
            return Some(*s);
        }
        let ed = &self.editor;
        let (cam, vp) = (ed.view.camera, ed.view.viewport);
        let key = (ed.revision, ed.loads, [cam.cx, cam.cy, cam.zoom, cam.rotation, vp.x, vp.y].map(f64::to_bits));
        if let Some((k, s)) = self.nearest_step.get() {
            if k == key {
                return s;
            }
        }
        // Step cameras once per revision and viewport (a long path is
        // thousands); each camera change then searches them sub-linearly.
        let ix_key = (ed.revision, ed.loads, [vp.x, vp.y].map(f64::to_bits));
        let mut ix = self.step_index.borrow_mut();
        if ix.as_ref().is_none_or(|(k, _)| *k != ix_key) {
            *ix = Some((ix_key, zebra_core::step_index::StepIndex::build(ed)));
        }
        let s = ix.as_ref().and_then(|(_, ix)| ix.nearest(&cam, vp.x));
        self.nearest_step.set(Some((key, s)));
        s
    }

    /// Settings › General › Language. Every string is looked up each
    /// frame, so the window follows at once; the Mac menu bar is rebuilt.
    pub fn set_language(&mut self, id: Option<&str>) {
        crate::i18n::set_language(id);
        #[cfg(target_os = "macos")]
        {
            self.menu = None;
        }
    }

    pub fn canvas_center_world(&self) -> Vec2 {
        self.editor.view.camera.center()
    }

    /// Insert a picture file: copied (cloned on APFS) and mapped, never
    /// read into memory whole.
    pub fn insert_image_file(&mut self, p: &std::path::Path, at: Vec2) {
        if let Err(e) = std::fs::metadata(p) {
            self.toast(crate::media_ui::unreadable(p, &e));
            return;
        }
        let alt = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        self.insert_image(String::new(), Some(p.to_string_lossy().to_string()), at, alt);
    }

    pub fn insert_image_bytes(&mut self, bytes: Vec<u8>, at: Vec2, alt: String) {
        let data_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        drop(bytes);
        self.insert_image(data_base64, None, at, alt);
    }

    fn insert_image(&mut self, data_base64: String, path: Option<String>, at: Vec2, alt: String) {
        let vw = self.editor.view.viewport.x / self.editor.view.camera.zoom;
        let r = self.editor.apply(Command::AddImage { data_base64, path, x: at.x, y: at.y, width: Some(vw * 0.4), alt, id: None }, H);
        if let Some(out) = self.report(r) {
            self.select(out.created);
            self.view_cmd(ViewCommand::SetTool { tool: Tool::Select });
        }
    }

    // ------------------------------------------------------------------
    // Files

    /// Open `p` in this window: a big file on a worker thread, with
    /// progress in the window; a small one at once.
    pub fn open_path(&mut self, p: &std::path::Path) {
        if crate::opening::is_big(p) {
            crate::opening::start(self, p);
            return;
        }
        let r = zebra_core::io::load(p).map(|doc| zebra_core::Editor::prepare(doc, &*self.editor.measure, &|_| {}));
        self.finish_open(p, r);
    }

    /// The file being opened, or the one this window has open.
    pub fn shown_file(&self) -> Option<&std::path::Path> {
        self.opening.as_ref().map(|o| o.path.as_path()).or(self.editor.file_path.as_deref())
    }

    /// Load `p`, read and prepared, or say why it couldn't be opened.
    pub fn finish_open(&mut self, p: &std::path::Path, r: zebra_core::ZResult<zebra_core::Prepared>) {
        match r {
            Ok(prepared) => {
                // What a save or open of this file that crashed left beside it.
                if let Some(dir) = p.parent() {
                    zebra_core::io::sweep_temp(dir, zebra_api::discovery::pid_alive);
                    crate::files::sweep_autosave_media(dir);
                }
                self.editor.load_prepared(prepared, Some(p.to_path_buf()));
                self.after_load();
                self.push_recent(p.to_path_buf());
                self.recovered = crate::files::autosave_newer(p);
                // A file Finder sent after the chooser came up replaces it.
                self.welcome = None;
            }
            Err(e) => {
                // Keep technical details out of the toast; log them instead.
                eprintln!("zebra: could not open {}: {e}", p.display());
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                let message = match e.code {
                    _ if e.about == Some(zebra_core::error::About::Document) => crate::t!("app-open-failed-too-large"),
                    "bad_file" if e.message.contains("newer") => crate::t!("app-open-failed-newer"),
                    "bad_file" => crate::t!("app-open-failed-not-zebra"),
                    _ if e.message.contains("No such file") => crate::t!("app-open-failed-missing"),
                    _ if e.message.contains("ermission") => crate::t!("app-open-failed-permission"),
                    _ => crate::t!("app-open-failed-unreadable"),
                };
                let title = crate::t!("app-open-failed-title", name = name.as_str());
                crate::sheets::request(self, crate::sheets::SheetKind::Alert { title, message });
            }
        }
    }

    /// An id for this window's own widget or area: two document windows
    /// never share scroll positions, popups or drag state.
    pub fn uid(&self, name: &str) -> egui::Id {
        egui::Id::unique((name, &self.doc_id))
    }

    /// A sheet or the template chooser is over the document (see
    /// [`works_under_a_sheet`]).
    pub fn sheet_up(&self) -> bool {
        self.welcome.is_some() || self.sheet_showing()
    }

    /// A Save Changes or other sheet is up (or waiting to go up).
    /// Save Changes is up, or about to be.
    pub fn asking_to_save(&self) -> bool {
        #[cfg(target_os = "macos")]
        if self.open_sheet.as_ref().is_some_and(|o| matches!(o.kind(), crate::sheets::SheetKind::SaveChanges(_))) {
            return true;
        }
        matches!(self.sheet, Some(crate::sheets::SheetKind::SaveChanges(_)))
    }

    pub fn sheet_showing(&self) -> bool {
        #[cfg(target_os = "macos")]
        if self.open_sheet.is_some() || self.open_picker.is_some() {
            return true;
        }
        self.sheet.is_some()
    }

    /// A new document nobody has touched: New and Open reuse its window.
    pub fn is_pristine(&self) -> bool {
        self.editor.file_path.is_none() && !self.editor.dirty() && self.editor.doc.elements.is_empty() && self.editor.revision == 0
    }

    /// Open an untitled document's autosave in this window, as the same
    /// untitled, unsaved document (it keeps writing to that autosave).
    ///
    /// It is adopted (renamed for this process) and marked as opening
    /// first: should showing it crash the app, the next launch asks about
    /// it instead of crashing again (see `files::opening_marker`).
    pub fn reopen_autosave(&mut self, path: &std::path::Path) -> bool {
        // Open Anyway: the last launch's marker goes; this one's is new.
        let _ = std::fs::remove_file(crate::files::opening_marker(path));
        let name = crate::files::adopt_autosave(path);
        let adopted = crate::files::untitled_dir().join(format!("{name}.zebra.autosave"));
        let path = if adopted.exists() { adopted } else { path.to_path_buf() };
        let marker = crate::files::mark_opening(&path);
        let unmark = |m: &Option<PathBuf>| {
            if let Some(m) = m {
                let _ = std::fs::remove_file(m);
            }
        };
        let doc = match zebra_core::io::load_autosave(&path, None) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("zebra: could not reopen {}: {e}", path.display());
                unmark(&marker);
                return false;
            }
        };
        // An empty document has nothing to bring back: no blank window.
        if doc.elements.is_empty() {
            unmark(&marker);
            zebra_core::io::remove_autosave(&path);
            return false;
        }
        self.editor.load(doc, None);
        self.after_load();
        // Unsaved work: the document shows as edited and asks before closing.
        self.editor.revision += 1;
        self.autosaved_rev = self.editor.revision;
        self.welcome = None;
        self.autosave_name = name;
        self.opening_marker = marker.map(|m| (m, self.now()));
        true
    }

    /// The reopened document has been shown long enough, or its window is
    /// going: it didn't crash the app, so its marker goes.
    pub fn clear_opening_marker(&mut self, now: bool) {
        if let Some((m, at)) = &self.opening_marker {
            if now || self.now() - at >= crate::files::OPENING_SECS {
                let _ = std::fs::remove_file(m);
                self.opening_marker = None;
            }
        }
    }

    /// Ask about the next untitled document the last launch quit while
    /// opening, once no other sheet is up.
    fn ask_about_crashed(&mut self) {
        if self.sheet.is_some() || self.crashed_untitled.is_empty() {
            return;
        }
        #[cfg(target_os = "macos")]
        if self.open_sheet.is_some() {
            return;
        }
        // In a window, wait for the shell to number the open windows.
        if self.hosted && self.untitled_floor == 0 {
            return;
        }
        let path = self.crashed_untitled.remove(0);
        // Its name, without opening it (opening it is what crashed): the
        // next number after the open windows'.
        self.crashed_seq += 1;
        let name = untitled_name("", &crate::t!("app-untitled"), self.untitled_floor + self.crashed_seq);
        crate::sheets::request(self, crate::sheets::SheetKind::CrashedOpening { path, name });
    }

    pub fn after_load(&mut self) {
        self.media.stop_all();
        self.selected_step = None;
        self.path_focus = false;
        self.step_multi = None;
        self.step_pick = None;
        self.canvas = CanvasState::default();
        self.thumbs.clear();
        self.recovered = None;
        self.autosaved_rev = self.editor.revision;
        self.seen_loads = self.editor.loads;
    }

    pub fn push_recent(&mut self, p: PathBuf) {
        self.recent.retain(|r| r != &p);
        self.recent.insert(0, p);
        self.recent.truncate(10);
    }

    /// Returns true if saved.
    pub fn save(&mut self, save_as: bool) -> bool {
        let path = match (&self.editor.file_path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new()
                .add_filter(crate::t!("app-file-filter-zebra"), &["zebra"])
                .set_file_name(format!("{}.zebra", sanitize(&self.editor.doc.title)))
                .save_file()
                .map(|p| if p.extension().is_none() { p.with_extension("zebra") } else { p }),
        };
        let Some(path) = path else { return false };
        self.save_to(&path)
    }

    /// Start saving to `path` on a worker thread (the window stays
    /// responsive); [`ZebraApp::poll_save`] finishes it. False if it
    /// couldn't start (the person has been told why).
    pub fn save_to(&mut self, path: &std::path::Path) -> bool {
        let ctx = self.ctx.clone();
        let viewport = self.viewport;
        match self.editor.start_save(path.to_path_buf(), H, Box::new(move || ctx.request_repaint_of(viewport))) {
            Ok(()) => true,
            Err(e) => {
                self.save_failed(path, &e);
                false
            }
        }
    }

    fn save_failed(&mut self, path: &std::path::Path, e: &zebra_core::ZError) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        crate::sheets::request(self, crate::sheets::SheetKind::Alert { title: crate::t!("app-save-failed-title", name = name.as_str()), message: e.message.clone() });
    }

    /// Wait for the save in flight, then finish it (tests, scripts).
    pub fn wait_save(&mut self) {
        self.editor.wait_save();
        self.poll_save();
    }

    /// Finish a save that is done (the person's or an agent's), and run
    /// what waited for it.
    pub fn poll_save(&mut self) {
        self.editor.poll_save();
        let mut failed = false;
        for s in self.editor.take_saved(|s| s.by == H) {
            match s.result {
                Ok(()) => {
                    crate::files::remove_autosave(&s.path);
                    crate::files::remove_autosave_untitled(self);
                    self.push_recent(s.path);
                }
                Err(e) => {
                    failed = true;
                    self.save_failed(&s.path, &e);
                }
            }
        }
        if self.editor.saving().is_none() && !self.after_save.is_empty() {
            let waiting = std::mem::take(&mut self.after_save);
            if !failed {
                self.queued.extend(still_for_this_document(waiting));
            }
        }
    }

    pub fn window_title(&self) -> String {
        let name = match &self.editor.file_path {
            Some(p) => p.file_stem().unwrap_or_default().to_string_lossy().to_string(),
            None => untitled_name(&self.editor.doc.title, &crate::t!("app-untitled"), self.untitled_no),
        };
        // On the Mac the title bar shows "— Edited" and the dot in the close
        // button itself (setDocumentEdited); elsewhere say it in the title.
        if cfg!(target_os = "macos") {
            name
        } else {
            if self.editor.dirty() { crate::t!("app-window-title-edited", name = name.as_str()) } else { name }
        }
    }

    // ------------------------------------------------------------------
    // Frame loop

    /// Work that must go on even while the window is hidden or covered
    /// (eframe then runs no ui pass): answering the automation API, so
    /// `zebractl` never hangs on a background window.
    pub fn logic(&mut self, ctx: &egui::Context) {
        // The clock runs whether or not the window is drawn: undo entries
        // and agent steps are timed by it, and `frame` (the only other
        // place it is set) doesn't run for a window that is hidden or idle.
        self.editor.view.now = self.now();
        // Cells selected in a table take Tab and the arrows: egui must not
        // also move keyboard focus to the first field it finds.
        if crate::table_ui::active(self) && !crate::table_ui::editing(self) && ctx.memory(|m| m.focused().is_none()) {
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        // A document loaded behind our back (an agent's `open`): drop what
        // we held about the old one, as our own Open does.
        if self.editor.loads != self.seen_loads {
            self.after_load();
        }
        // Debug screenshots must render even if the window opened behind
        // others or on another Space (eframe otherwise skips the ui pass).
        if self.screenshot.is_some() && !self.screenshot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Automation API (hosted windows: the shell pumps for all of them)
        if self.hosted {
        } else if let (Some(server), Some(d)) = (&self.api, &mut self.dispatcher) {
            if d.pump(server, &mut self.editor) {
                ctx.request_repaint();
            }
            // Core events (document/selection/mode changes) — the UI polls
            // state directly, so these are only drained here.
            let _ = d.take_events();
            if std::mem::take(&mut d.disconnect_requested) && self.ai_off_at.is_none() {
                self.ai_off_at = Some(self.now() + 0.3);
                ctx.request_repaint_after(std::time::Duration::from_millis(350));
            }
            // No heartbeat while an agent is connected: the server wakes us on
            // connect, request and disconnect, and each presence animation
            // (flash, cursor, busy dot, status bar) asks for its own frames.
        } else {
            self.editor.drain_events();
        }
        if let Some(t) = self.ai_off_at {
            if self.now() >= t {
                let name = self.agent_name();
                if let Err(e) = zebra_api::discovery::record_ai_switch(false) {
                    eprintln!("zebra: couldn't keep the AI control switch: {e}");
                }
                self.ai_saved = false;
                self.set_ai_control(false);
                self.toast(crate::t!("app-toast-agent-switched-ai-off", name = name.as_str()));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64((t - self.now()).max(0.016)));
            }
        }
        self.background_work(ctx);
    }

    /// The window is minimised, a tab in the background, or fully covered:
    /// no frame is drawn for it.
    pub fn out_of_sight(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| i.raw.viewports.get(&self.viewport).is_some_and(|v| v.occluded == Some(true) || v.minimized == Some(true)))
    }

    /// What must not wait for the window to be drawn: autosave (an agent
    /// may be editing a minimised window), and the menu and queued actions
    /// of a window that is out of sight (Quit, Close, Save).
    fn background_work(&mut self, ctx: &egui::Context) {
        self.poll_save();
        self.clear_opening_marker(false);
        if let Some((_, at)) = &self.opening_marker {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((crate::files::OPENING_SECS - (self.now() - at)).max(0.1)));
        }
        self.ask_about_crashed();
        crate::opening::poll(self);
        crate::files::autosave_tick(self);
        if let Some(wait) = crate::files::autosave_due_in(self) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        // Decoded pictures not drawn for five minutes are given back. A
        // window in view redraws once a minute while any are held, which
        // keeps its own; a hidden window's go.
        let now = self.now();
        if now - self.last_image_trim >= IMAGE_TRIM_EVERY {
            self.last_image_trim = now;
            let (_, more) = self.renderer.images().trim_idle(std::time::Duration::from_secs_f64(IMAGE_IDLE));
            if more {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(IMAGE_TRIM_EVERY));
            }
        }
        if !self.out_of_sight(ctx) {
            return;
        }
        #[cfg(target_os = "macos")]
        if self.front {
            crate::menu_mac::poll(self, ctx);
        }
        if !self.queued.is_empty() && !self.wake_hidden {
            // Actions run in `frame`, where the window is the current
            // one: ask for one frame although nothing will be seen.
            self.wake_hidden = true;
            ctx.send_viewport_cmd_to(self.viewport, egui::ViewportCommand::RequestPaintWhileHidden);
            if self.viewport != egui::ViewportId::ROOT {
                // Document windows are drawn during the root window's pass.
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::RequestPaintWhileHidden);
            }
            ctx.request_repaint_of(self.viewport);
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.wake_hidden = false;
        if self.editor.loads != self.seen_loads {
            self.after_load();
        }
        // Menu → text field copy/paste, delivered to this window's input.
        if !self.inject.is_empty() {
            let mut ev = std::mem::take(&mut self.inject);
            ctx.input_mut(|i| i.events.append(&mut ev));
        }
        let now = self.now();
        self.stage.frame_start = Some(std::time::Instant::now());
        if self.editor.view.tick(now) {
            ctx.request_repaint();
        }
        crate::presenting::tick_autoplay(self, &ctx);
        crate::presenting::tick_notes(self);

        #[cfg(target_os = "macos")]
        if self.front {
            crate::menu_mac::poll(self, &ctx);
        }

        if !self.demo_done {
            self.demo_done = true;
            if let Some(d) = self.demo.clone() {
                crate::sample::demo(self, &ctx, &d);
            }
        }

        self.handle_dropped_files(&ctx);
        crate::media_ui::tick(self);
        self.handle_close(&ctx);
        self.sync_present_mode(&ctx);
        self.pdf_notes();
        crate::sidebar::check_focus(self);

        if self.editor.view.mode == Mode::Present {
            crate::present::ui(self, ui);
        } else {
            self.handle_shortcuts(&ctx);
            crate::panels::layout(self, ui);
            // A title typed in the inspector, then a click on the canvas:
            // the field is gone before it could lose focus. Keep the title.
            if self.title_buf.is_some() && self.title_seen != ctx.cumulative_pass_nr() {
                let t = self.title_buf.take().unwrap_or_default().trim().to_string();
                if !t.is_empty() && t != self.editor.doc.title {
                    self.apply(Command::SetTitle { title: t });
                }
            }
            crate::presenting::playback_window(self, &ctx);
            crate::theme_editor::window(self, &ctx);
            crate::welcome::layout_gallery(self, &ctx);
            crate::welcome::diagram_gallery(self, &ctx);
            crate::credits::window(self, &ctx);
            crate::welcome::window(self, &ctx);
        }
        crate::presenting::presenter_window(self, &ctx);

        if self.front {
            crate::settings::show(self, &ctx);
        }

        self.symbols.after_frame(&ctx);

        let queued = std::mem::take(&mut self.queued);
        for a in queued {
            self.perform(&ctx, a);
        }

        crate::files::autosave_tick(self);
        let title = self.window_title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        // agent flashes fade over ~1 s
        if self.editor.flashes.iter().any(|(_, t)| self.editor.view.now - *t < 1.2) {
            ctx.request_repaint();
        }
        // Idle at rest (F26): wake only when something is due, never poll.
        for slot in [&mut self.toast, &mut self.note] {
            let Some(t) = slot else { continue };
            let left = t.until - now;
            if left <= 0.0 {
                *slot = None;
            } else if left > 0.3 {
                // Sleep until the fade-out starts, then animate it.
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(left - 0.3));
            } else {
                ctx.request_repaint();
            }
        }
        if let Some(wait) = crate::files::autosave_due_in(self) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        self.thumbs.update(&self.editor, now);
        crate::files::debug_screenshot(self, &ctx);
        self.record_perf(now);
        for c in ctx.repaint_causes() {
            self.editor.perf.record_cause(now, c.to_string());
        }
    }

    /// Frame time and cache sizes for the `perf` query.
    fn record_perf(&mut self, now: f64) {
        if let Some(t) = self.stage.frame_start.take() {
            self.editor.perf.record_frame(now, t.elapsed().as_secs_f32() * 1000.0);
        }
        let caches = &mut self.editor.perf.caches;
        for (k, v) in self.renderer.cache_stats() {
            caches.insert(k.into(), v);
        }
        caches.insert("thumbnails".into(), self.thumbs.len() as u64);
        caches.insert("presenter_kb".into(), self.stage.slides.texture_bytes() / 1024);
        caches.insert("previews".into(), self.previews.as_ref().map(|p| p.textures.len() as u64).unwrap_or(0));
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        // Settings › Keyboard is recording keys: none of them act.
        if self.key_capture.is_some() {
            return;
        }
        let text_focus = ctx.egui_wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { continue };
            // The path list has the keys: Up/Down/Home/End move through the
            // steps (Delete and Escape reach it as actions, below).
            if !text_focus && self.palette.is_none() && self.step_pick.is_none() && crate::sidebar::list_focused(self) && crate::sidebar::key(self, key, modifiers) {
                continue;
            }
            // Cells selected in a table take arrows, Tab, Return and typing.
            if !text_focus && self.palette.is_none() && crate::table_ui::key(self, key, modifiers) {
                continue;
            }
            // Arrow keys: nudge selection (screen-relative), else pan.
            if !text_focus && self.palette.is_none() {
                let arrow = match key {
                    egui::Key::ArrowLeft => Some((-1.0, 0.0)),
                    egui::Key::ArrowRight => Some((1.0, 0.0)),
                    egui::Key::ArrowUp => Some((0.0, -1.0)),
                    egui::Key::ArrowDown => Some((0.0, 1.0)),
                    _ => None,
                };
                if let Some((x, y)) = arrow {
                    if !modifiers.command {
                        let step = if modifiers.shift { 10.0 } else { 1.0 };
                        self.nudge(x * step, y * step);
                        continue;
                    }
                }
            }
            // Esc while dragging words cancels the drag, and only that.
            if key == egui::Key::Escape && crate::text_drag::cancel(self) {
                continue;
            }
            let Some(action) = crate::keymap::action_for(modifiers, key) else { continue };
            let action = &action;
            let has_cmd = modifiers.command;
            // While typing: only ⌘ shortcuts that text fields don't use.
            if text_focus {
                // In a canvas text box ⌘B/⌘I/⌘U format and ⌘K adds a link.
                let rich_keys = self.canvas.text.is_some() && matches!(key, egui::Key::B | egui::Key::I | egui::Key::U | egui::Key::K);
                let text_uses = matches!(key, egui::Key::A | egui::Key::C | egui::Key::V | egui::Key::X | egui::Key::Z | egui::Key::Y)
                    || !has_cmd
                    || rich_keys;
                if text_uses {
                    continue;
                }
            }
            if self.palette.is_some() && *action != Action::CommandPalette {
                continue;
            }
            // On macOS the native menu owns these key equivalents.
            #[cfg(target_os = "macos")]
            if self.menu.is_some() && crate::menu_mac::menu_owns(action) {
                continue;
            }
            self.queued.push(action.clone());
        }
        // Clipboard (egui turns ⌘C/⌘X/⌘V into these events). Not while a
        // sheet or the template chooser is up: the document is behind it.
        if !text_focus && self.palette.is_none() && !self.sheet_up() {
            let evs = ctx.input(|i| i.events.clone());
            for ev in evs {
                match ev {
                    egui::Event::Copy => self.queued.push(Action::Copy),
                    egui::Event::Cut => self.queued.push(Action::Cut),
                    egui::Event::Paste(_) => self.queued.push(Action::Paste),
                    egui::Event::PasteImage(img) => crate::media_ui::paste_color_image(self, img),
                    _ => {}
                }
            }
        }
    }

    fn nudge(&mut self, sx: f64, sy: f64) {
        let ids = self.editable_selection();
        if ids.is_empty() {
            self.view_cmd(ViewCommand::Pan { dx: -sx * 20.0, dy: -sy * 20.0 });
            return;
        }
        let cam = self.editor.view.camera;
        let w = Vec2::new(sx, sy).rotate_deg(cam.rotation) * (1.0 / cam.zoom);
        let label = crate::t!("app-undo-nudge");
        // A run of nudges of the same objects is one undo step, like a drag.
        let (n, now) = (self.editor.history().len(), self.editor.view.now);
        let run = self.editor.history().last().is_some_and(|h| h.origin == H && h.label == label && h.ids == ids && now - h.time < 1.0);
        // A frame selected on its own moves alone.
        let with_contents = self.editor.frame_only().is_none();
        let r = self.editor.apply_batch(vec![Command::Move { ids, dx: w.x, dy: w.y, with_contents }], Some(label), H);
        if r.is_ok() && run && self.editor.history().len() == n + 1 {
            self.editor.merge_last_two_steps();
        }
        self.report(r);
    }

    fn handle_dropped_files(&mut self, ctx: &Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if files.is_empty() {
            return;
        }
        // A sheet or the template chooser is up: only documents open (as
        // File › Open would); nothing is added to the document behind.
        if self.sheet_up() {
            for f in &files {
                let p = f.path().to_path_buf();
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zebra")) {
                    self.queued.push(Action::OpenPath(p));
                }
            }
            return;
        }
        let pos = ctx.input(|i| i.pointer.hover_pos());
        let at = pos.map(|p| self.canvas.s2w(&self.editor, p)).unwrap_or_else(|| self.canvas_center_world());
        for (i, f) in files.iter().enumerate() {
            let p = f.path().to_path_buf();
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if ext == "zebra" {
                self.queued.push(Action::OpenPath(p));
                continue;
            }
            if !p.as_os_str().is_empty() {
                let off = 30.0 * i as f64 / self.editor.view.camera.zoom;
                crate::media_ui::insert_file(self, &p, at + Vec2::new(off, off));
                continue;
            }
            match f.bytes() {
                // Only where there is no file (a browser). A big one goes in
                // from a file, as a dropped file does:
                // its base64 would be another copy, a third larger.
                Ok(bytes) if bytes.len() > 64 << 20 => {
                    let off = 30.0 * i as f64 / self.editor.view.camera.zoom;
                    let tmp = std::env::temp_dir().join(format!("zebra-drop-{}-{i}.{}", std::process::id(), if ext.is_empty() { "bin" } else { ext.as_str() }));
                    match std::fs::write(&tmp, &bytes[..]) {
                        Ok(()) => self.insert_image_file(&tmp, at + Vec2::new(off, off)),
                        Err(e) => self.toast(crate::media_ui::unreadable(&p, &e)),
                    }
                    let _ = std::fs::remove_file(&tmp);
                }
                Ok(bytes) => {
                    let off = 30.0 * i as f64 / self.editor.view.camera.zoom;
                    self.insert_image_bytes(bytes, at + Vec2::new(off, off), p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                }
                Err(e) => self.toast(crate::media_ui::cant_read(&p, &e)),
            }
        }
    }

    fn handle_close(&mut self, ctx: &Context) {
        if self.hosted {
            // The close button: this window only, after Save Changes.
            if ctx.input(|i| i.viewport().close_requested()) && !self.force_quit {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if !self.close_me && self.sheet.is_none() && !self.queued.contains(&Action::Close) {
                    self.queued.push(Action::Close);
                }
            }
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.editor.saving().is_some() {
            // Quitting waits for the save in flight.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.after_save.contains(&Action::Quit) {
                self.after_save.push(Action::Quit);
            }
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.force_quit && self.editor.dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.editor.file_path.is_some() {
                // Saved in place, as Keynote does; then the window closes.
                if self.save(false) && !self.after_save.contains(&Action::Quit) {
                    self.after_save.push(Action::Quit);
                }
            } else if !self.asking_to_save() {
                crate::sheets::request(self, crate::sheets::SheetKind::SaveChanges(Action::Quit));
            }
        }
    }

    /// A PDF page of this document couldn't be drawn sharper (its helper
    /// ran out of memory or time on it): say so once per PDF per session,
    /// never while presenting (the note waits until the show ends).
    fn pdf_notes(&mut self) {
        if self.editor.view.mode == Mode::Present {
            return;
        }
        let assets = &self.editor.doc.assets;
        if !zebra_import::pdf::take_draw_failures(|k| assets.contains_key(k)).is_empty() {
            self.note(crate::t!("pdf-note-not-sharper"));
        }
    }

    fn sync_present_mode(&mut self, ctx: &Context) {
        let presenting = self.editor.view.mode == Mode::Present;
        if presenting != self.was_presenting {
            self.was_presenting = presenting;
            crate::canvas::commit_text_edit(self);
            crate::presenting::sync_stage(self, ctx, presenting);
            self.revealed.clear();
            self.prev_hidden = self.editor.hidden_builds();
            // Show the step HUD for a moment when the show starts.
            self.last_pointer_move = self.last_pointer_move.max(self.now());
            if !presenting {
                // Esc returns to the editor at the current step.
                if let Some(s) = self.editor.view.present.step {
                    self.selected_step = Some(s);
                }
                ctx.set_cursor_icon(egui::CursorIcon::Default);
            }
        }
    }

    /// Blank state for the present view.
    pub fn blank(&self) -> Option<Blank> {
        self.editor.view.present.blank
    }
}

/// Open a web or mail link in the default app. Other schemes are refused.
pub fn open_url(url: &str) {
    let u = url.trim();
    if !(u.starts_with("https://") || u.starts_with("http://") || u.starts_with("mailto:")) {
        eprintln!("zebra: not opening link with unsupported scheme: {u}");
        return;
    }
    #[cfg(target_os = "macos")]
    let cmd = "/usr/bin/open";
    #[cfg(not(target_os = "macos"))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd).arg(u).spawn();
}

pub fn permission_name(p: zebra_core::view::Permission) -> &'static str {
    use zebra_core::view::Permission::*;
    match p {
        Observe => "observe",
        Edit => "edit",
        Full => "full",
    }
}

/// What an agent may do in a run a launch flag started: the person's
/// saved choice (Edit if they never chose), lowered to `cap` (from
/// `--agent-permission`) but never raised by it.
pub fn launch_permission(saved: Option<zebra_core::view::Permission>, cap: zebra_core::view::Permission) -> zebra_core::view::Permission {
    saved.unwrap_or(zebra_core::view::Permission::Edit).min(cap)
}

/// The eframe app id; its settings file is `app.ron` under
/// `eframe::storage_dir(APP_ID)`.
pub const APP_ID: &str = "dev.zebra.Zebra";

pub fn parse_permission(s: &str) -> zebra_core::view::Permission {
    use zebra_core::view::Permission::*;
    match s {
        "observe" => Observe,
        "full" => Full,
        _ => Edit,
    }
}

/// A new, empty document, titled in the person's language.
pub fn new_document(theme: &str) -> zebra_core::Document {
    zebra_core::Document { title: crate::t!("app-untitled"), theme: theme.to_string(), ..Default::default() }
}

pub fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
    let t = t.trim().to_string();
    if t.is_empty() { crate::t!("app-untitled") } else { t }
}

impl eframe::App for ZebraApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ZebraApp::logic(self, ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frame(ui);
        #[cfg(target_os = "macos")]
        {
            crate::mac::sync_window(self, Some(frame));
            // An open sheet is polled when AppKit wakes us, not every frame.
            let _ = crate::sheets::run_native(self, Some(frame));
        }
        let _ = frame;
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "recent", &self.recent);
        eframe::set_value(storage, "scroll_mode", &self.scroll_mode);
        eframe::set_value(storage, "motion_override", &self.motion_override);
        eframe::set_value(storage, "snapping", &self.snapping);
        eframe::set_value(storage, crate::beta_ui::STORAGE_KEY, &self.editor.view.beta_features);
        eframe::set_value(storage, "sidebar_width", &self.sidebar_w);
        eframe::set_value(storage, "inspector_width", &self.inspector_w);
        eframe::set_value(storage, "default_theme", &self.default_theme);
        eframe::set_value(storage, "settings_pane", &self.settings_pane);
        eframe::set_value(storage, "ai_enabled", &self.ai_saved);
        eframe::set_value(storage, "agent_permission", &permission_name(self.editor.view.agent_permission).to_string());
        self.stage.save(storage);
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// One spelling for a file's path, to tell whether two paths are one file.
pub fn same_file(p: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Decoded pictures are dropped after this long without being drawn.
const IMAGE_IDLE: f64 = 300.0;
/// How often that is checked.
const IMAGE_TRIM_EVERY: f64 = 60.0;

/// Actions that still work while a sheet or the template chooser is up,
/// as in Keynote: making, opening, closing and quitting documents, and the
/// app's own windows. Everything else would act on the document behind.
pub fn works_under_a_sheet(a: &Action) -> bool {
    use Action::*;
    matches!(
        a,
        New | NewFromSample | NewFromTemplate(_) | NewFromTemplateFile(_) | Open | OpenPath(_) | ReopenAutosave(_) | Close | Quit | Settings | About | Acknowledgements | PauseAgents | StopAgents
    )
}

/// What waited for a save, in order, minus saves asked for after the
/// document is replaced or closed: they were meant for the document that
/// was just written, not the new empty one (which would put up a Save
/// dialog nobody asked for).
pub fn still_for_this_document(waiting: Vec<Action>) -> Vec<Action> {
    use Action::*;
    let mut replaced = false;
    let mut out = Vec::with_capacity(waiting.len());
    for a in waiting {
        if replaced && matches!(a, Save | SaveAs) {
            continue;
        }
        if matches!(a, New | NewFromSample | NewFromTemplate(_) | NewFromTemplateFile(_) | Open | OpenPath(_) | Close) {
            replaced = true;
        }
        out.push(a);
    }
    out
}

#[cfg(test)]
mod launch_permission_tests {
    use super::launch_permission;
    use zebra_core::view::Permission::*;

    #[test]
    fn a_launch_flag_only_lowers_the_persons_choice() {
        assert_eq!(launch_permission(None, Full), Edit, "never chose: edit, even with --agent-permission full");
        assert_eq!(launch_permission(Some(Edit), Full), Edit);
        assert_eq!(launch_permission(Some(Observe), Full), Observe);
        assert_eq!(launch_permission(Some(Full), Full), Full, "full only when the person chose it");
        assert_eq!(launch_permission(Some(Full), Observe), Observe);
        assert_eq!(launch_permission(Some(Full), Edit), Edit);
    }
}
