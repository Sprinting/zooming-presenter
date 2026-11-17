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
    Command, Editor, Element, ElementId, ElementKind, Mode, NewElement, Origin, StepTarget, Tool, Vec2, ViewCommand,
    ZOrderOp,
};
use zebra_render::Renderer;

use crate::actions::{mods_match, Action, SHORTCUTS};
use crate::canvas::CanvasState;
use crate::thumbs::Thumbs;

const H: Origin = Origin::Human;
pub const HUMAN: Origin = Origin::Human;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum ScrollMode {
    /// Trackpad scroll pans, pinch zooms; a mouse wheel zooms.
    #[default]
    Auto,
    /// Scrolling always zooms (classic Prezi).
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
}

#[derive(Clone)]
pub struct Toast {
    pub text: String,
    pub until: f64,
}

pub struct ZebraApp {
    pub editor: Editor,
    pub renderer: Arc<Renderer>,
    pub wgpu: Option<egui_wgpu::RenderState>,
    pub gpu_canvas: Option<zebra_render::gpu::VelloCanvas>,
    /// False when the renderer can't draw (falls back to the egui painter).
    pub render_ok: bool,
    pub api: Option<zebra_api::ApiServer>,
    pub dispatcher: Option<zebra_api::Dispatcher>,
    pub api_error: Option<String>,
    /// Socket override from `--api-socket` (dev); used whenever AI control
    /// is switched on.
    pub api_socket: Option<PathBuf>,
    /// AI control is on right now ("Let AI agents control Zebra").
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
    /// The inspector tab last chosen (kept across selections when it fits).
    pub inspector_tab: crate::inspector::Tab,
    pub show_activity: bool,
    pub palette: Option<(String, usize)>,
    pub show_shortcuts: bool,
    pub library: crate::shape_library::Library,
    pub show_about: bool,
    pub selected_step: Option<usize>,
    pub clipboard: Vec<Element>,
    pub paste_count: u32,
    pub recent: Vec<PathBuf>,
    pub toast: Option<Toast>,
    /// The sheet waiting for an answer (save changes, errors, confirmations).
    pub sheet: Option<crate::sheets::SheetKind>,
    /// Use native NSAlert sheets and system dialogs (macOS with a window);
    /// off in UI tests, where sheets are drawn in egui.
    pub native_dialogs: bool,
    /// The native sheet on screen, waiting for its answer (macOS).
    #[cfg(target_os = "macos")]
    pub open_sheet: Option<crate::sheets::OpenSheet>,
    pub show_settings: bool,
    /// Debug: draw the icon sheet (`--demo icons`).
    pub show_icon_sheet: bool,
    pub settings_pane: crate::settings::Pane,
    /// Theme for File › New (Settings › General).
    pub default_theme: String,
    /// Snap to other objects while moving (Settings › Canvas).
    pub snapping: bool,
    /// macOS window state last pushed to AppKit.
    pub mac_sync: crate::mac::WindowSync,
    /// Step that was just added: its sidebar row highlights briefly.
    pub flash_step: Option<(usize, f64)>,
    pub queued: Vec<Action>,
    pub scroll_mode: ScrollMode,
    /// Reduce Motion chosen in Settings; `None` follows the system.
    pub motion_override: Option<bool>,
    pub last_autosave: f64,
    pub autosaved_rev: u64,
    pub recovered: Option<PathBuf>,
    pub was_presenting: bool,
    pub force_quit: bool,
    pub title_buf: Option<String>,
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
    #[cfg(target_os = "macos")]
    pub menu: Option<crate::menu_mac::MacMenu>,
    /// Video and audio players, crop mode, clipboard (session state).
    pub media: crate::media_ui::Media,
}

impl ZebraApp {
    /// Build the app. `wgpu` is `None` in UI tests.
    pub fn new(ctx: &Context, wgpu: Option<egui_wgpu::RenderState>, storage: Option<&dyn eframe::Storage>, opts: AppOptions) -> Self {
        crate::style::install_fonts(ctx);
        crate::mac::refresh_accent();
        crate::style::apply_visuals(ctx);
        let renderer = Arc::new(Renderer::new());
        // Pictures decode on worker threads; repaint when one lands.
        let waker_ctx = ctx.clone();
        renderer.images().set_waker(Some(Arc::new(move || waker_ctx.request_repaint())));
        let mut editor = Editor::default();
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
            welcome: None,
            previews: None,
            show_layouts: false,
            show_sidebar: true,
            show_props: true,
            inspector_tab: crate::inspector::Tab::default(),
            show_activity: false,
            palette: None,
            show_shortcuts: false,
            library: Default::default(),
            show_about: false,
            selected_step: None,
            clipboard: vec![],
            paste_count: 0,
            recent,
            toast: None,
            sheet: None,
            native_dialogs: cfg!(target_os = "macos") && has_window,
            #[cfg(target_os = "macos")]
            open_sheet: None,
            show_settings: false,
            show_icon_sheet: false,
            settings_pane: storage.and_then(|s| eframe::get_value(s, "settings_pane")).unwrap_or_default(),
            default_theme: storage.and_then(|s| eframe::get_value(s, "default_theme")).unwrap_or_else(|| zebra_core::Document::default().theme),
            snapping: storage.and_then(|s| eframe::get_value(s, "snapping")).unwrap_or(true),
            mac_sync: Default::default(),
            flash_step: None,
            queued: vec![],
            scroll_mode,
            motion_override,
            last_autosave: 0.0,
            autosaved_rev: 0,
            recovered: None,
            was_presenting: false,
            force_quit: false,
            title_buf: None,
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
        let saved_ai: bool = storage.and_then(|s| eframe::get_value(s, "ai_enabled")).unwrap_or(false);
        app.ai_saved = saved_ai;
        // The first time is always a person: until someone has switched AI
        // control on in this window, neither a launch flag nor a saved
        // setting turns it on.
        let granted = zebra_api::discovery::human_has_granted();
        if let Some(api_opts) = opts.api.filter(|_| granted) {
            app.api_socket = api_opts.socket.clone();
            app.editor.view.agent_permission = api_opts.permission;
            app.set_ai_control(true);
        } else if saved_ai && granted {
            if let Some(p) = storage.and_then(|s| eframe::get_value::<String>(s, "agent_permission")) {
                app.editor.view.agent_permission = parse_permission(&p);
            }
            app.set_ai_control(true);
        }
        let launch_welcome = opts.open.is_none() && app.demo.is_none() && app.screenshot.is_none() && app.wgpu.is_some();
        if let Some(p) = opts.open {
            app.open_path(&p);
        }
        // Like Keynote: a fresh launch offers the template chooser.
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

    /// Switch "Let AI agents control Zebra" on or off. Only the human UI
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
                self.dispatcher = Some(zebra_api::Dispatcher::new(self.renderer.clone()));
                self.api = Some(s);
                self.api_error = None;
            }
            Err(e) => self.api_error = Some(crate::t!("app-ai-start-failed", error = e.to_string())),
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
                self.toast(crate::words::error(&e.code));
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
        self.view_cmd(ViewCommand::Select { ids });
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

    pub fn perform(&mut self, ctx: &Context, a: Action) {
        use Action::*;
        // Commit any in-place text edit first so actions see the final text.
        if !matches!(a, Undo | Redo) {
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
        match a {
            New | NewFromSample | NewFromTemplate(_) | NewFromTemplateFile(_) | Open | OpenPath(_) | Quit | Close if self.editor.dirty() && self.sheet.is_none() && !self.force_quit => {
                crate::sheets::request(self, crate::sheets::SheetKind::SaveChanges(a));
            }
            // One document per window for now: closing the window quits.
            Close => {
                self.force_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            New => {
                let doc = zebra_core::Document { theme: self.default_theme.clone(), ..Default::default() };
                self.editor.load(doc, None);
                self.after_load();
            }
            ShowTemplates => self.welcome = Some(crate::welcome::Welcome::new()),
            NewFromTemplate(t) => crate::welcome::new_from_template(self, &t),
            NewFromTemplateFile(p) => crate::welcome::new_from_template_file(self, &p),
            SaveAsTemplate => crate::welcome::save_as_template(self),
            InsertLayout => self.show_layouts = true,
            ThemeEditor => self.theme_ed = Some(crate::theme_editor::ThemeEditor::open(self)),
            NewFromSample => {
                let doc = crate::sample::build(&*self.renderer);
                self.editor.load(doc, None);
                self.after_load();
            }
            Open => {
                if let Some(p) = rfd::FileDialog::new().add_filter(crate::t!("app-file-filter-zebra"), &["zebra"]).pick_file() {
                    self.open_path(&p);
                }
            }
            OpenPath(p) => self.open_path(&p),
            Save => {
                self.save(false);
            }
            SaveAs => {
                self.save(true);
            }
            ExportPdf => crate::files::export_pdf(self),
            ExportPng => crate::files::export_png(self),
            Quit => {
                self.force_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Undo => {
                crate::canvas::cancel_text_edit(self);
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
                self.clipboard = ids.iter().filter_map(|id| self.editor.doc.get(id).cloned()).collect();
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
            Delete => {
                let ids = self.editable_selection();
                if !ids.is_empty() {
                    self.apply(Command::Delete { ids });
                }
            }
            SelectAll => self.view_cmd(ViewCommand::SelectAll),
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
            InsertVideo => crate::media_ui::insert_video_dialog(self),
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
            InsertImage => {
                if let Some(p) = rfd::FileDialog::new().add_filter(crate::t!("app-file-filter-image"), &["png", "jpg", "jpeg", "webp", "gif"]).pick_file() {
                    let c = self.canvas_center_world();
                    self.insert_image_file(&p, c);
                }
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
                self.apply(Command::AutoPath);
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
            PresentStart => self.view_cmd(ViewCommand::Present { step: None }),
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
                if on {
                    if let Err(e) = zebra_api::discovery::record_human_grant() {
                        eprintln!("zebra: couldn't record the AI control grant: {e}");
                    }
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
                    let all_on = ids.iter().all(|id| match self.editor.doc.get(id).map(|e| &e.kind) {
                        Some(ElementKind::Text(t)) => if a == Bold { t.bold } else { t.italic },
                        _ => false,
                    });
                    let cmds = ids
                        .into_iter()
                        .map(|id| {
                            let mut patch = zebra_core::ElementPatch::default();
                            if a == Bold {
                                patch.bold = Some(!all_on);
                            } else {
                                patch.italic = Some(!all_on);
                            }
                            Command::Update { id, patch }
                        })
                        .collect();
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
        ids.iter()
            .filter(|id| !self.editor.doc.parent_frame(id).is_some_and(|p| ids.contains(&p)))
            .cloned()
            .collect()
    }

    fn paste(&mut self) {
        // Something newer on the system clipboard (a picture, files from
        // Finder, text from another app) wins over Zebra's own copy.
        if crate::media_ui::paste_from_system(self) {
            return;
        }
        if self.clipboard.is_empty() {
            return;
        }
        self.paste_count += 1;
        let d = 24.0 * self.paste_count as f64 / self.editor.view.camera.zoom;
        // If the copied things are off-screen, paste them at the view centre.
        let b = self.clipboard.iter().map(|e| e.bounds()).reduce(|a, b| a.union(&b)).unwrap();
        let vis = self.editor.view.camera.visible_bounds(self.editor.view.viewport);
        let (dx, dy) = if vis.intersects(&b) {
            (d, d)
        } else {
            let c = vis.center() - b.center();
            (c.x, c.y)
        };
        let cmds: Vec<Command> = self
            .clipboard
            .iter()
            .map(|e| {
                let keep_w = !matches!(&e.kind, ElementKind::Text(t) if !t.fixed_width);
                Command::AddElement {
                    element: NewElement {
                        id: None,
                        x: e.transform.x + dx,
                        y: e.transform.y + dy,
                        scale: e.transform.scale,
                        rotation: e.transform.rotation,
                        w: keep_w.then_some(e.size.w),
                        h: keep_w.then_some(e.size.h),
                        name: e.name.clone(),
                        kind: e.kind.clone(),
                    },
                }
            })
            .collect();
        let r = self.editor.apply_batch(cmds, Some(crate::t!("app-undo-paste")), H);
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

    /// Classic Prezi "Add current view": an invisible frame around the view.
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
                kind: ElementKind::Frame(zebra_core::FrameData { style: zebra_core::FrameStyle::Invisible, color: None }),
            },
        };
        let r = self.editor.apply_batch(vec![cmd], Some(crate::t!("app-undo-add-current-view")), H);
        if let Some(out) = self.report(r) {
            if let Some(id) = out.created.first().cloned() {
                self.apply(Command::AddStep { target: StepTarget::Element { id }, index: None, builds: vec![], notes: String::new() });
                self.selected_step = Some(self.editor.doc.path.len() - 1);
                self.flash_step = Some((self.editor.doc.path.len() - 1, self.now()));
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
        let cam = self.editor.view.camera;
        (0..self.editor.doc.path.len()).min_by(|a, b| {
            let d = |i: usize| {
                let c = self.editor.camera_for_step(i);
                let dist = (c.center() - cam.center()).len() * cam.zoom / self.editor.view.viewport.x;
                dist + (c.zoom / cam.zoom).ln().abs()
            };
            d(*a).total_cmp(&d(*b))
        })
    }

    pub fn canvas_center_world(&self) -> Vec2 {
        self.editor.view.camera.center()
    }

    pub fn insert_image_file(&mut self, p: &std::path::Path, at: Vec2) {
        match std::fs::read(p) {
            Ok(bytes) => self.insert_image_bytes(bytes, at, p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
            Err(e) => self.toast(crate::t!("app-toast-read-failed", path = p.display().to_string(), error = e.to_string())),
        }
    }

    pub fn insert_image_bytes(&mut self, bytes: Vec<u8>, at: Vec2, alt: String) {
        let vw = self.editor.view.viewport.x / self.editor.view.camera.zoom;
        let data_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let r = self.editor.apply(Command::AddImage { data_base64, x: at.x, y: at.y, width: Some(vw * 0.4), alt, id: None }, H);
        if let Some(out) = self.report(r) {
            self.select(out.created);
            self.view_cmd(ViewCommand::SetTool { tool: Tool::Select });
        }
    }

    // ------------------------------------------------------------------
    // Files

    pub fn open_path(&mut self, p: &std::path::Path) {
        match zebra_core::io::load(p) {
            Ok(doc) => {
                self.editor.load(doc, Some(p.to_path_buf()));
                self.after_load();
                self.push_recent(p.to_path_buf());
                self.recovered = crate::files::autosave_newer(p);
            }
            Err(e) => {
                // Keep technical details out of the toast; log them instead.
                eprintln!("zebra: could not open {}: {e}", p.display());
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
                let message = match e.code {
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

    pub fn after_load(&mut self) {
        self.media.stop_all();
        self.selected_step = None;
        self.canvas = CanvasState::default();
        self.thumbs.clear();
        self.recovered = None;
        self.autosaved_rev = self.editor.revision;
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

    pub fn save_to(&mut self, path: &std::path::Path) -> bool {
        match zebra_core::io::save(&mut self.editor.doc, path) {
            Ok(()) => {
                self.editor.mark_saved(Some(path.to_path_buf()));
                crate::files::remove_autosave(path);
                crate::files::remove_autosave_untitled();
                self.push_recent(path.to_path_buf());
                true
            }
            Err(e) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                crate::sheets::request(
                    self,
                    crate::sheets::SheetKind::Alert { title: crate::t!("app-save-failed-title", name = name.as_str()), message: e.message.clone() },
                );
                false
            }
        }
    }

    pub fn window_title(&self) -> String {
        let name = match &self.editor.file_path {
            Some(p) => p.file_stem().unwrap_or_default().to_string_lossy().to_string(),
            None => self.editor.doc.title.clone(),
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
        // Debug screenshots must render even if the window opened behind
        // others or on another Space (eframe otherwise skips the ui pass).
        if self.screenshot.is_some() && !self.screenshot_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Automation API
        if let (Some(server), Some(d)) = (&self.api, &mut self.dispatcher) {
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
                self.ai_saved = false;
                self.set_ai_control(false);
                self.toast(crate::t!("app-toast-agent-switched-ai-off", name = name.as_str()));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64((t - self.now()).max(0.016)));
            }
        }

    }

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = self.now();
        self.stage.frame_start = Some(std::time::Instant::now());
        if self.editor.view.tick(now) {
            ctx.request_repaint();
        }
        crate::presenting::tick_autoplay(self, &ctx);
        crate::presenting::tick_notes(self);

        #[cfg(target_os = "macos")]
        crate::menu_mac::poll(self, &ctx);

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

        if self.editor.view.mode == Mode::Present {
            crate::present::ui(self, ui);
        } else {
            self.handle_shortcuts(&ctx);
            crate::panels::layout(self, ui);
            crate::presenting::playback_window(self, &ctx);
            crate::theme_editor::window(self, &ctx);
            crate::welcome::layout_gallery(self, &ctx);
            crate::welcome::window(self, &ctx);
        }
        crate::presenting::presenter_window(self, &ctx);

        crate::settings::show(self, &ctx);

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
        if let Some(t) = &self.toast {
            let left = t.until - now;
            if left <= 0.0 {
                self.toast = None;
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
        caches.insert("thumbnails".into(), self.thumbs.textures.iter().flatten().count() as u64);
        caches.insert("presenter_kb".into(), self.stage.slides.texture_bytes() / 1024);
        caches.insert("previews".into(), self.previews.as_ref().map(|p| p.textures.len() as u64).unwrap_or(0));
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        let text_focus = ctx.egui_wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { continue };
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
            let Some((_, _, action)) = SHORTCUTS.iter().find(|(m, k, _)| *k == key && mods_match(modifiers, *m)) else { continue };
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
        // Clipboard (egui turns ⌘C/⌘X/⌘V into these events)
        if !text_focus && self.palette.is_none() {
            let evs = ctx.input(|i| i.events.clone());
            for ev in evs {
                match ev {
                    egui::Event::Copy => self.queued.push(Action::Copy),
                    egui::Event::Cut => self.queued.push(Action::Cut),
                    egui::Event::Paste(_) => self.queued.push(Action::Paste),
                    egui::Event::PasteImage(img) => {
                        let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
                        crate::media_ui::paste_rgba(self, img.size[0] as u32, img.size[1] as u32, rgba);
                    }
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
        let r = self.editor.apply_batch(vec![Command::Move { ids, dx: w.x, dy: w.y, with_contents: true }], Some(crate::t!("app-undo-nudge")), H);
        self.report(r);
    }

    fn handle_dropped_files(&mut self, ctx: &Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if files.is_empty() {
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
                Ok(bytes) => {
                    let off = 30.0 * i as f64 / self.editor.view.camera.zoom;
                    self.insert_image_bytes(bytes, at + Vec2::new(off, off), p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                }
                Err(e) => self.toast(crate::t!("app-toast-read-dropped-failed", error = e.to_string())),
            }
        }
    }

    fn handle_close(&mut self, ctx: &Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.force_quit && self.editor.dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            crate::sheets::request(self, crate::sheets::SheetKind::SaveChanges(Action::Quit));
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

pub fn parse_permission(s: &str) -> zebra_core::view::Permission {
    use zebra_core::view::Permission::*;
    match s {
        "observe" => Observe,
        "full" => Full,
        _ => Edit,
    }
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
            crate::mac::sync_window(self, frame);
            // An open sheet is polled when AppKit wakes us, not every frame.
            let _ = crate::sheets::run_native(self, frame);
        }
        let _ = frame;
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "recent", &self.recent);
        eframe::set_value(storage, "scroll_mode", &self.scroll_mode);
        eframe::set_value(storage, "motion_override", &self.motion_override);
        eframe::set_value(storage, "snapping", &self.snapping);
        eframe::set_value(storage, "default_theme", &self.default_theme);
        eframe::set_value(storage, "settings_pane", &self.settings_pane);
        eframe::set_value(storage, "ai_enabled", &self.ai_saved);
        eframe::set_value(storage, "agent_permission", &permission_name(self.editor.view.agent_permission).to_string());
        self.stage.save(storage);
    }

    fn raw_input_hook(&mut self, _ctx: &Context, raw: &mut egui::RawInput) {
        raw.events.append(&mut self.inject);
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}
