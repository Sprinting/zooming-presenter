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
    pub t0: Instant,
    pub canvas: CanvasState,
    pub thumbs: Thumbs,
    pub show_sidebar: bool,
    pub show_props: bool,
    pub show_activity: bool,
    pub palette: Option<(String, usize)>,
    pub show_shortcuts: bool,
    pub show_about: bool,
    pub selected_step: Option<usize>,
    pub clipboard: Vec<Element>,
    pub paste_count: u32,
    pub recent: Vec<PathBuf>,
    pub toast: Option<Toast>,
    /// Action waiting for "Save changes?" confirmation.
    pub confirm: Option<Action>,
    pub queued: Vec<Action>,
    pub scroll_mode: ScrollMode,
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
    #[cfg(target_os = "macos")]
    pub menu: Option<crate::menu_mac::MacMenu>,
}

impl ZebraApp {
    /// Build the app. `wgpu` is `None` in UI tests.
    pub fn new(ctx: &Context, wgpu: Option<egui_wgpu::RenderState>, storage: Option<&dyn eframe::Storage>, opts: AppOptions) -> Self {
        crate::style::install_fonts(ctx);
        crate::style::apply_visuals(ctx);
        let renderer = Arc::new(Renderer::new());
        let mut editor = Editor::default();
        editor.measure = renderer.measure();
        let recent: Vec<PathBuf> = storage.and_then(|s| eframe::get_value(s, "recent")).unwrap_or_default();
        let scroll_mode = storage.and_then(|s| eframe::get_value(s, "scroll_mode")).unwrap_or_default();
        let reduce_motion: bool = storage.and_then(|s| eframe::get_value(s, "reduce_motion")).unwrap_or(false);
        editor.view.reduce_motion = reduce_motion;

        let mut app = ZebraApp {
            editor,
            renderer: renderer.clone(),
            gpu_canvas: wgpu.as_ref().map(zebra_render::gpu::VelloCanvas::new),
            wgpu,
            render_ok: true,
            api: None,
            dispatcher: None,
            api_error: None,
            t0: Instant::now(),
            canvas: CanvasState::default(),
            thumbs: Thumbs::new(ctx.clone()),
            show_sidebar: true,
            show_props: true,
            show_activity: false,
            palette: None,
            show_shortcuts: false,
            show_about: false,
            selected_step: None,
            clipboard: vec![],
            paste_count: 0,
            recent,
            toast: None,
            confirm: None,
            queued: vec![],
            scroll_mode,
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
        };
        if let Some(api_opts) = opts.api {
            let c = ctx.clone();
            let waker: zebra_api::Waker = Arc::new(move || c.request_repaint());
            match zebra_api::ApiServer::start(api_opts, waker) {
                Ok(s) => {
                    app.dispatcher = Some(zebra_api::Dispatcher::new(renderer));
                    app.api = Some(s);
                }
                Err(e) => app.api_error = Some(format!("Automation API failed to start: {e}")),
            }
        }
        if let Some(p) = opts.open {
            app.open_path(&p);
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

    /// Report a core error to the human (agents get them over the API).
    pub fn report<T>(&mut self, r: zebra_core::ZResult<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                let msg = match &e.hint {
                    Some(h) => format!("{} — {}", e.message, h),
                    None => e.message.clone(),
                };
                self.toast(msg);
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

    pub fn agent_name(&self) -> String {
        if let Some(c) = self.api.as_ref().and_then(|a| a.clients().into_iter().next()) {
            return c.name;
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
            .unwrap_or_else(|| "Agent".into())
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
            New | NewFromSample | Open | OpenPath(_) | Quit if self.editor.dirty() && self.confirm.is_none() && !self.force_quit => {
                self.confirm = Some(a);
            }
            New => {
                self.editor.load(zebra_core::Document::default(), None);
                self.after_load();
            }
            NewFromSample => {
                let doc = crate::sample::build(&*self.renderer);
                self.editor.load(doc, None);
                self.after_load();
                self.toast("Sample loaded — press F5 to present it");
            }
            Open => {
                if let Some(p) = rfd::FileDialog::new().add_filter("Zebra presentation", &["zebra"]).pick_file() {
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
                let r = self.editor.undo(H);
                if let Some(Some(l)) = self.report(r) {
                    self.toast(format!("Undo {l}"));
                }
            }
            Redo => {
                let r = self.editor.redo(H);
                if let Some(Some(l)) = self.report(r) {
                    self.toast(format!("Redo {l}"));
                }
            }
            Copy | Cut => {
                let ids = self.editor.expand(&self.selection(), true);
                self.clipboard = ids.iter().filter_map(|id| self.editor.doc.get(id).cloned()).collect();
                self.paste_count = 0;
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
                    self.toast(format!("{} frame", crate::actions::frame_name(s)));
                } else {
                    self.view_cmd(ViewCommand::SetTool { tool: Tool::Frame });
                }
            }
            SetFrameStyle(s) => {
                self.view_cmd(ViewCommand::SetFrameStyle { style: s });
                self.view_cmd(ViewCommand::SetTool { tool: Tool::Frame });
            }
            InsertImage => {
                if let Some(p) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "webp", "gif"]).pick_file() {
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
                    let r = self.editor.apply_batch(cmds, Some(if lock { "Lock" } else { "Unlock" }.into()), H);
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
                self.view_cmd(ViewCommand::SetReduceMotion { on });
                self.toast(if on { "Reduce motion on: gentle cross-fades, no rotation" } else { "Reduce motion off" });
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
                    self.apply(Command::ClearPath);
                }
            }
            PresentStart => self.view_cmd(ViewCommand::Present { step: None }),
            PresentCurrent => {
                let step = self.current_step();
                self.view_cmd(ViewCommand::Present { step });
            }
            ToggleActivity => self.show_activity = !self.show_activity,
            PauseAgents => {
                let p = !self.editor.view.agent_paused;
                self.view_cmd(ViewCommand::PauseAgents { paused: p });
                self.toast(if p { "Agent paused — its changes are refused until you resume" } else { "Agent resumed" });
            }
            StopAgents => {
                self.view_cmd(ViewCommand::PauseAgents { paused: true });
                self.toast("Agent stopped (paused). Resume from the agent pill.");
            }
            FollowAgent => {
                let on = !self.editor.view.follow_agent;
                self.view_cmd(ViewCommand::FollowAgent { on });
            }
            RevertAgent => {
                let name = self.agent_name();
                let n = self.editor.revert_agent_session(&name);
                self.toast(if n == 0 { format!("Nothing from {name} to revert") } else { format!("Reverted {n} change group(s) by {name}") });
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
        let r = self.editor.apply_batch(cmds, Some("Paste".into()), H);
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
        let r = self.editor.apply_batch(cmds, Some("Add to path".into()), H);
        if self.report(r).is_some() {
            self.selected_step = Some(self.editor.doc.path.len() - 1);
            self.toast(if n == 1 { format!("Added step {}", self.editor.doc.path.len()) } else { format!("Added {n} steps") });
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
        let r = self.editor.apply_batch(vec![cmd], Some("Add current view".into()), H);
        if let Some(out) = self.report(r) {
            if let Some(id) = out.created.first().cloned() {
                self.apply(Command::AddStep { target: StepTarget::Element { id }, index: None, builds: vec![], notes: String::new() });
                self.selected_step = Some(self.editor.doc.path.len() - 1);
                self.toast(format!("Added view as step {}", self.editor.doc.path.len()));
            }
        }
    }

    /// The step to present from: the one selected in the sidebar, else the
    /// step whose view is closest to the current camera.
    pub fn current_step(&self) -> Option<usize> {
        if self.editor.doc.path.is_empty() {
            return None;
        }
        if let Some(s) = self.selected_step.filter(|s| *s < self.editor.doc.path.len()) {
            return Some(s);
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
            Err(e) => self.toast(format!("Could not read {}: {e}", p.display())),
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
            Err(e) => self.toast(format!("Could not open {}: {}", p.display(), e.message)),
        }
    }

    pub fn after_load(&mut self) {
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
                .add_filter("Zebra presentation", &["zebra"])
                .set_file_name(format!("{}.zebra", sanitize(&self.editor.doc.title)))
                .save_file()
                .map(|p| if p.extension().is_none() { p.with_extension("zebra") } else { p }),
        };
        let Some(path) = path else { return false };
        self.save_to(&path)
    }

    pub fn save_to(&mut self, path: &std::path::Path) -> bool {
        match zebra_core::io::save(&self.editor.doc, path) {
            Ok(()) => {
                self.editor.mark_saved(Some(path.to_path_buf()));
                crate::files::remove_autosave(path);
                crate::files::remove_autosave_untitled();
                self.push_recent(path.to_path_buf());
                self.toast(format!("Saved {}", path.file_name().unwrap_or_default().to_string_lossy()));
                true
            }
            Err(e) => {
                self.toast(format!("Save failed: {}", e.message));
                false
            }
        }
    }

    pub fn window_title(&self) -> String {
        let name = match &self.editor.file_path {
            Some(p) => p.file_stem().unwrap_or_default().to_string_lossy().to_string(),
            None => self.editor.doc.title.clone(),
        };
        format!("{name}{} — Zebra", if self.editor.dirty() { " (edited)" } else { "" })
    }

    // ------------------------------------------------------------------
    // Frame loop

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = self.now();
        if self.editor.view.tick(now) {
            ctx.request_repaint();
        }

        // Automation API
        if let (Some(server), Some(d)) = (&self.api, &mut self.dispatcher) {
            if d.pump(server, &mut self.editor) {
                ctx.request_repaint();
            }
            // Core events (document/selection/mode changes) — the UI polls
            // state directly, so these are only drained here.
            let _ = d.take_events();
            // While agents are connected keep a slow heartbeat so presence
            // (flashes, cursor) animates even when the waker is quiet.
            if !server.clients().is_empty() {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        } else {
            self.editor.drain_events();
        }

        #[cfg(target_os = "macos")]
        crate::menu_mac::poll(self, &ctx);

        if !self.demo_done {
            self.demo_done = true;
            if let Some(d) = self.demo.clone() {
                crate::sample::demo(self, &ctx, &d);
            }
        }

        self.handle_dropped_files(&ctx);
        self.handle_close(&ctx);
        self.sync_present_mode(&ctx);

        if self.editor.view.mode == Mode::Present {
            crate::present::ui(self, ui);
        } else {
            self.handle_shortcuts(&ctx);
            crate::panels::layout(self, ui);
        }

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
        if let Some(t) = &self.toast {
            if now > t.until {
                self.toast = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
        self.thumbs.update(&self.editor, now);
        crate::files::debug_screenshot(self, &ctx);
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
                let text_uses = matches!(key, egui::Key::A | egui::Key::C | egui::Key::V | egui::Key::X | egui::Key::Z | egui::Key::Y)
                    || !has_cmd;
                if text_uses {
                    continue;
                }
            }
            if self.palette.is_some() && *action != Action::CommandPalette {
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
        let r = self.editor.apply_batch(vec![Command::Move { ids, dx: w.x, dy: w.y, with_contents: true }], Some("Nudge".into()), H);
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
            match f.bytes() {
                Ok(bytes) => {
                    let off = 30.0 * i as f64 / self.editor.view.camera.zoom;
                    self.insert_image_bytes(bytes, at + Vec2::new(off, off), p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                }
                Err(e) => self.toast(format!("Could not read dropped file: {e}")),
            }
        }
    }

    fn handle_close(&mut self, ctx: &Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.force_quit && self.editor.dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Action::Quit);
        }
    }

    fn sync_present_mode(&mut self, ctx: &Context) {
        let presenting = self.editor.view.mode == Mode::Present;
        if presenting != self.was_presenting {
            self.was_presenting = presenting;
            crate::canvas::commit_text_edit(self);
            if self.wgpu.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(presenting));
            }
            self.revealed.clear();
            self.prev_hidden = self.editor.hidden_builds();
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

pub fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
    let t = t.trim().to_string();
    if t.is_empty() { "Untitled".into() } else { t }
}

impl eframe::App for ZebraApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "recent", &self.recent);
        eframe::set_value(storage, "scroll_mode", &self.scroll_mode);
        eframe::set_value(storage, "reduce_motion", &self.editor.view.reduce_motion);
    }

    fn raw_input_hook(&mut self, _ctx: &Context, raw: &mut egui::RawInput) {
        raw.events.append(&mut self.inject);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}
