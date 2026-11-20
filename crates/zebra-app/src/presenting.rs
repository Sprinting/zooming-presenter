//! Presenting extras: speaker notes, autoplay and loop, per-step timing,
//! the presenter view in a second window, and playing on a chosen display
//! in a borderless window (not a full-screen Space).
//!
//! Every edit goes through the command bus (`update_step`, `set_playback`);
//! the presenter view and display choice are app settings, not document
//! state.
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};

use egui::{Align2, Color32, ColorImage, FontId, Key, Rect, RichText, TextureHandle, TextureOptions, Ui, Vec2 as EVec2};
use zebra_core::view::Blank;
use zebra_core::{Command, Document, Editor, Mode, Origin, ViewCommand};

use crate::app::ZebraApp;
use crate::style::{self, Palette, ACCENT};
use crate::widgets as w;

const H: Origin = Origin::Human;

/// Autoplay choices offered in the UI, seconds.
pub const AUTOPLAY_CHOICES: [f64; 3] = [4.0, 10.0, 20.0];

/// App-level presenting state (settings persist across launches).
pub struct Stage {
    /// Show the presenter view in a second window while presenting.
    pub presenter_view: bool,
    /// Name of the display to play on; None = the window's own display.
    pub display: Option<String>,
/// The Playback options window is open.
    pub show_playback: bool,
    /// Presenter timer: when it started, and the elapsed time while paused.
    pub timer_start: f64,
    pub timer_paused: Option<f64>,
    /// Notes text size in the presenter view, points.
    pub notes_size: f32,
    /// Notes being typed: (step, text, last keystroke time).
    pub notes_buf: Option<(usize, String, f64)>,
    /// The step those notes are for, as it was when the typing began: the
    /// path can change under them (an agent, an undo).
    pub notes_for: Option<zebra_core::model::PathStep>,
    /// Window frame to restore after playing borderless on a display.
    saved_frame: Option<Rect>,
    pub slides: Slides,
    /// Frame timing for the perf query.
    pub frame_start: Option<std::time::Instant>,
    /// A step number being typed to jump to (Keynote: type the slide
    /// number, then Return). Some("") while the field is open but empty.
    pub jump: Option<String>,
}

impl Stage {
    pub fn new(ctx: &egui::Context, renderer: std::sync::Arc<zebra_render::Renderer>, storage: Option<&dyn eframe::Storage>) -> Self {
        let get = |k: &str| storage.and_then(|s| s.get_string(k));
        Stage {
            presenter_view: get("presenter_view").map(|v| v == "true").unwrap_or(false),
            display: get("play_display").filter(|s| !s.is_empty()),
            show_playback: false,
            timer_start: 0.0,
            timer_paused: None,
            notes_size: get("notes_size").and_then(|v| v.parse().ok()).unwrap_or(22.0),
            notes_buf: None,
            notes_for: None,
            saved_frame: None,
            slides: Slides::new(ctx.clone(), renderer),
            frame_start: None,
            jump: None,
        }
    }
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        storage.set_string("presenter_view", self.presenter_view.to_string());
        storage.set_string("play_display", self.display.clone().unwrap_or_default());
        storage.set_string("notes_size", self.notes_size.to_string());
    }
    pub fn elapsed(&self, now: f64) -> f64 {
        self.timer_paused.unwrap_or(now - self.timer_start).max(0.0)
    }
}

// ----------------------------------------------------------------------
// Speaker notes

/// Multi-line notes editor for `step`. Typing is kept in a buffer and saved
/// as one undo step when the field loses focus, after a pause, or when
/// another step is shown.
pub fn notes_editor(app: &mut ZebraApp, ui: &mut Ui, step: usize, rows: usize, id: &str) {
    let now = app.now();
    if app.stage.notes_buf.as_ref().is_some_and(|b| b.0 != step) {
        commit_notes(app);
    }
    let Some(s) = app.editor.doc.path.get(step) else { return };
    let mut text = match &app.stage.notes_buf {
        Some((i, t, _)) if *i == step => t.clone(),
        _ => s.notes.clone(),
    };
    let r = ui.add(
        egui::TextEdit::multiline(&mut text)
            .id_salt(("notes", id))
            .hint_text(crate::t!("present-notes-hint"))
            .desired_rows(rows)
            .desired_width(f32::INFINITY),
    );
    if r.changed() {
        if app.stage.notes_buf.is_none() {
            app.stage.notes_for = Some(s.clone());
        }
        app.stage.notes_buf = Some((step, text, now));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(1100));
    }
    if r.lost_focus() {
        commit_notes(app);
    }
}

/// Save buffered notes (if any) through the command bus.
pub fn commit_notes(app: &mut ZebraApp) {
    let Some((step, text, _)) = app.stage.notes_buf.take() else { return };
    // The step the notes were typed for, wherever it is now. If steps were
    // removed or reordered meanwhile, its number is no longer `step`.
    let path = &app.editor.doc.path;
    let step = match app.stage.notes_for.take() {
        None => Some(step),
        Some(was) => {
            let same = |s: &zebra_core::model::PathStep| s.target == was.target;
            if path.get(step) == Some(&was) {
                Some(step)
            } else if let Some(i) = path.iter().position(|s| *s == was) {
                Some(i)
            } else if path.get(step).is_some_and(same) {
                Some(step)
            } else {
                let mut hits = path.iter().enumerate().filter(|(_, s)| same(s));
                match (hits.next(), hits.next()) {
                    (Some((i, _)), None) => Some(i),
                    _ => None,
                }
            }
        }
    };
    let Some(step) = step else { return };
    if app.editor.doc.path.get(step).is_some_and(|s| s.notes != text) {
        app.apply(Command::UpdateStep { index: step, target: None, builds: None, notes: Some(text), duration: None, dwell: None, hide_builds: None });
    }
}

/// Called every frame: saves notes after a second without typing.
pub fn tick_notes(app: &mut ZebraApp) {
    if app.stage.notes_buf.as_ref().is_some_and(|b| app.now() - b.2 > 1.0) {
        commit_notes(app);
    }
}

/// The notes pane under the path sidebar, for the selected/current step.
pub fn sidebar_notes(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let Some(step) = app.current_step() else { return };
    ui.add_space(4.0);
    notes_editor(app, ui, step, 3, "sidebar");
    ui.horizontal(|ui| {
        ui.label(RichText::new(crate::t!("present-notes-step", step = ((step + 1) as u64))).size(11.0).color(p.weak));
    });
    ui.add_space(2.0);
}

// ----------------------------------------------------------------------
// Inspector sections

/// "Path step" section for the inspector: notes, transition time and
/// autoplay time of one step.
pub fn step_section(app: &mut ZebraApp, ui: &mut Ui, p: &Palette, step: usize) {
    let Some(s) = app.editor.doc.path.get(step).cloned() else { return };
    ui.add_space(8.0);
    ui.label(RichText::new(crate::t!("present-path-step", step = ((step + 1) as u64))).font(style::semibold(11.0)).color(p.weak));
    ui.add_space(2.0);
    notes_editor(app, ui, step, 4, "inspector");
    ui.add_space(4.0);

    // Transition and autoplay as inspector rows (label column, then the
    // kit's controls), not loose egui labels, checkboxes and drag values.
    w::row(ui, &crate::t!("present-transition"), |ui| {
        let mut auto = s.duration.is_none();
        let mut secs = s.duration.unwrap_or(1.5);
        if w::checkbox(ui, &mut auto, &crate::t!("present-transition-auto")).on_hover_text(crate::t!("present-transition-auto-tip")).changed() {
            set_step_timing(app, step, Some(if auto { -1.0 } else { secs }), None);
        }
        if !auto && w::number_field(ui, &mut secs, w::Num::secs().range(0.0, 10.0).step(0.1).width(70.0)).changed() {
            set_step_timing(app, step, Some(secs), None);
        }
    });
    w::row(ui, &crate::t!("present-autoplay"), |ui| {
        let own = s.dwell.is_some();
        let mut secs = s.dwell.or(app.editor.doc.play.autoplay).unwrap_or(10.0);
        let def = match app.editor.doc.play.autoplay {
            Some(d) => crate::t!("present-dwell-default-time", time = fmt_secs(d)),
            None => crate::t!("present-dwell-default-off"),
        };
        let lbl = if own { crate::t!("present-dwell-own") } else { def };
        let width = if own { (ui.available_width() - 76.0).max(90.0) } else { ui.available_width() };
        w::popup_button(ui, ("dwell", step), &lbl, width, |ui| {
            if w::menu_check(ui, !own, &crate::t!("present-dwell-presentation-default"), None).clicked() && own {
                set_step_timing(app, step, None, Some(-1.0));
            }
            if w::menu_check(ui, own, &crate::t!("present-dwell-own-for-step"), None).clicked() && !own {
                set_step_timing(app, step, None, Some(secs));
            }
        });
        if own && w::number_field(ui, &mut secs, w::Num::secs().range(1.0, 600.0).step(1.0).decimals(0).width(70.0)).changed() {
            set_step_timing(app, step, None, Some(secs));
        }
    });
}

fn set_step_timing(app: &mut ZebraApp, step: usize, duration: Option<f64>, dwell: Option<f64>) {
    let r = app.editor.apply_batch(
        vec![Command::UpdateStep { index: step, target: None, builds: None, notes: None, duration, dwell, hide_builds: None }],
        Some(crate::t!("present-undo-change-step-timing")),
        H,
    );
    app.report(r);
}

pub fn fmt_secs(s: f64) -> String {
    let secs = if s.fract().abs() < 1e-9 { format!("{s:.0}") } else { format!("{s:.1}") };
    crate::t!("present-seconds", secs = secs)
}

/// Autoplay, loop, presenter view and display: the Playback options.
pub fn playback_ui(app: &mut ZebraApp, ui: &mut Ui) {
    let play = app.editor.doc.play;
    let p = style::Palette::of(ui.ctx());
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.label(RichText::new(crate::t!("present-autoplay")).font(style::semibold(13.0)).color(p.text));
    // A segmented control (one choice of several), not a row of tinted chips.
    let mut items: Vec<(Option<f64>, String)> = vec![(None, crate::t!("present-autoplay-off"))];
    items.extend(AUTOPLAY_CHOICES.iter().map(|s| (Some(*s), fmt_secs(*s))));
    if let Some(a) = play.autoplay.filter(|a| !AUTOPLAY_CHOICES.contains(a)) {
        items.push((Some(a), fmt_secs(a)));
    }
    let segs: Vec<(Option<f64>, w::Seg)> = items.iter().map(|(v, l)| (*v, w::Seg::Text(l.as_str()))).collect();
    let mut cur = play.autoplay;
    if w::segmented(ui, &mut cur, &segs, Some(ui.available_width())).changed() && cur != play.autoplay {
        app.apply(Command::SetPlayback { autoplay: Some(cur.unwrap_or(0.0)), looping: None });
    }
    w::caption(ui, crate::t!("present-autoplay-help", key = "P"));
    let mut lp = play.looping;
    if w::checkbox(ui, &mut lp, &crate::t!("present-loop")).changed() {
        app.apply(Command::SetPlayback { autoplay: None, looping: Some(lp) });
    }
    ui.add_space(4.0);
    w::separator(ui);
    ui.add_space(4.0);
    ui.label(RichText::new(crate::t!("present-presenting")).font(style::semibold(13.0)).color(p.text));
    w::checkbox(ui, &mut app.stage.presenter_view, &crate::t!("present-show-presenter-view")).on_hover_text(crate::t!("present-show-presenter-view-tip"));
    let displays = displays(ui.ctx());
    let current = app.stage.display.clone().unwrap_or_else(|| crate::t!("present-this-display"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(crate::t!("present-play-on"));
        w::popup_button(ui, "play-display", &current, 190.0, |ui| {
            if w::menu_check(ui, app.stage.display.is_none(), &crate::t!("present-this-display"), None).clicked() {
                app.stage.display = None;
            }
            for d in &displays {
                if w::menu_check(ui, app.stage.display.as_deref() == Some(d.name.as_str()), &d.name, None).clicked() {
                    app.stage.display = Some(d.name.clone());
                }
            }
        });
    });
    if displays.len() < 2 {
        w::caption(ui, crate::t!("present-connect-second-display"));
    }
}

/// The Playback options window (Present ▸ Playback Options…).
pub fn playback_window(app: &mut ZebraApp, ctx: &egui::Context) {
    if !app.stage.show_playback {
        return;
    }
    let mut open = true;
    egui::Window::new(crate::t!("present-playback-window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(300.0)
        .anchor(Align2::CENTER_CENTER, EVec2::ZERO)
        .show(ctx, |ui| playback_ui(app, ui));
    app.stage.show_playback = open;
}

// ----------------------------------------------------------------------
// Displays and the borderless play window

#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub name: String,
    /// Frame in points, top-left origin (the main display's top-left = 0,0).
    pub frame: Rect,
    pub main: bool,
}

/// Connected displays. On macOS from `NSScreen`; elsewhere just the
/// window's own monitor.
pub fn displays(ctx: &egui::Context) -> Vec<Display> {
    #[cfg(target_os = "macos")]
    {
        let v = crate::presenting::mac::screens();
        if !v.is_empty() {
            return v;
        }
    }
    let size = ctx.input(|i| i.viewport().monitor_size).unwrap_or(EVec2::new(1440.0, 900.0));
    vec![Display { name: crate::t!("present-built-in-display"), frame: Rect::from_min_size(egui::Pos2::ZERO, size), main: true }]
}

/// The display to play on, if the person chose one that is connected.
fn chosen_display(app: &ZebraApp, ctx: &egui::Context) -> Option<Display> {
    let want = app.stage.display.as_ref()?;
    displays(ctx).into_iter().find(|d| &d.name == want)
}

/// The presenter view goes on another display than the one played on.
fn presenter_display(app: &ZebraApp, ctx: &egui::Context) -> Option<Display> {
    let ds = displays(ctx);
    let play = chosen_display(app, ctx).or_else(|| ds.iter().find(|d| d.main).cloned());
    ds.into_iter().find(|d| Some(d) != play.as_ref())
}

/// Enter or leave the play window. With a chosen display: a borderless,
/// always-on-top window covering it (no new Space, so the presenter view
/// and other apps stay reachable). Otherwise: full screen, as before.
pub fn sync_stage(app: &mut ZebraApp, ctx: &egui::Context, presenting: bool) {
    commit_notes(app);
    if presenting {
        app.stage.timer_start = app.now();
        app.stage.timer_paused = None;
    }
    if app.wgpu.is_none() {
        return; // UI tests: no real window.
    }
    match (presenting, chosen_display(app, ctx)) {
        (true, Some(d)) => {
            app.stage.saved_frame = ctx.input(|i| i.viewport().outer_rect);
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(d.frame.min));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(d.frame.size()));
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            #[cfg(target_os = "macos")]
            mac::hide_menu_bar(d.main, true);
        }
        (true, None) => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true)),
        (false, _) => {
            if let Some(r) = app.stage.saved_frame.take() {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(r.min));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(r.size()));
                #[cfg(target_os = "macos")]
                mac::hide_menu_bar(true, false);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
        }
    }
}

// ----------------------------------------------------------------------
// Autoplay and present-mode extras

/// Run autoplay; schedule the next repaint when it will advance.
pub fn tick_autoplay(app: &mut ZebraApp, ctx: &egui::Context) {
    if let Some(left) = app.editor.autoplay_tick() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left.clamp(0.01, 600.0)));
    }
}

/// Present-mode keys and HUD added by this module: P pauses autoplay; a
/// thin progress line shows time left on the step while autoplaying.
pub fn present_extras(app: &mut ZebraApp, ui: &mut Ui, rect: Rect) {
    let ctx = ui.ctx().clone();
    if ctx.input(|i| i.key_pressed(Key::P)) {
        let paused = !app.editor.view.present.autoplay_paused;
        let _ = app.editor.view(ViewCommand::PauseAutoplay { paused }, H);
        app.toast(if paused { crate::t!("present-toast-autoplay-paused", key = "P") } else { crate::t!("present-toast-autoplay-resumed") });
    }
    let Some(interval) = app.editor.autoplay_interval() else { return };
    if app.editor.view.present.autoplay_paused || app.editor.view.flight.is_some() || app.editor.at_end() && !app.editor.doc.play.looping {
        return;
    }
    let t = ((app.editor.view.now - app.editor.view.present.arrived_at) / interval).clamp(0.0, 1.0) as f32;
    let y = rect.bottom() - 2.0;
    ui.painter().line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.left() + rect.width() * t, y)], egui::Stroke::new(3.0, Color32::from_white_alpha(70)));
    // Smooth progress at a low rate; the advance itself is scheduled exactly.
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
}

// ----------------------------------------------------------------------
// Presenter view

const PRESENTER: &str = "zebra-presenter";

/// Show the presenter window while presenting (if switched on).
pub fn presenter_window(app: &mut ZebraApp, ctx: &egui::Context) {
    if app.editor.view.mode != Mode::Present || !app.stage.presenter_view {
        return;
    }
    let pos = presenter_display(app, ctx);
    let mut builder = egui::ViewportBuilder::default().with_title(crate::t!("present-presenter-window-title")).with_min_inner_size([640.0, 400.0]);
    match &pos {
        Some(d) => builder = builder.with_position(d.frame.min).with_inner_size(d.frame.size()),
        None => builder = builder.with_inner_size([1100.0, 700.0]),
    }
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(PRESENTER), builder, |ui, class| {
        // Embedded (no multi-window support): the main window already
        // handles the keys, so don't handle them twice.
        presenter_ui(app, ui, class != egui::ViewportClass::EmbeddedWindow);
    });
}

/// What the presenter sees: current and next step, notes, timer and clock.
pub fn presenter_ui(app: &mut ZebraApp, ui: &mut Ui, own_keys: bool) {
    let ctx = ui.ctx().clone();
    // The presenter window has its own keyboard focus: same keys as the stage.
    if own_keys {
        presenter_keys(app, &ctx);
    }
    if ctx.input(|i| i.viewport().close_requested()) {
        app.stage.presenter_view = false;
    }
    let now = app.now();
    let ed = &app.editor;
    let (step, build) = (ed.view.present.step, ed.view.present.build);
    let n = ed.doc.path.len();
    let next = next_position(ed);
    app.stage.slides.request(ed, [Some((step, build)), next.map(|(s, b)| (Some(s), b))]);

    let bg = Color32::from_rgb(0x16, 0x17, 0x1a);
    let fg = Color32::from_gray(235);
    let weak = Color32::from_gray(150);
    egui::CentralPanel::default().frame(egui::Frame::new().fill(bg).inner_margin(egui::Margin::same(20))).show(ui, |ui| {
        // Dark controls on the dark presenter display (Keynote's), whatever
        // the system appearance: light bezels sat on near-black before.
        ui.style_mut().visuals = egui::Visuals::dark();
        ui.style_mut().spacing.item_spacing.x = 8.0;
        // Top bar: position, timer, clock.
        ui.horizontal(|ui| {
            let pos = match step {
                Some(s) => crate::t!("present-presenter-step-of", step = ((s + 1) as u64), count = n),
                None => crate::t!("present-presenter-overview", count = n),
            };
            let pos = jump_text(app, true).unwrap_or(pos);
            let typing = app.stage.jump.is_some();
            let mut text = RichText::new(pos).size(16.0).color(fg);
            if typing {
                text = text.underline();
            }
            let label = ui
                .add(egui::Label::new(text).sense(egui::Sense::click()))
                .on_hover_text(crate::t!("present-go-to-tip"))
                .on_hover_cursor(egui::CursorIcon::Text);
            if label.clicked() && n > 0 {
                app.stage.jump = Some(String::new());
            }
            if let Some(a) = app.editor.autoplay_interval() {
                let s = if app.editor.view.present.autoplay_paused { crate::t!("present-presenter-autoplay-paused") } else { crate::t!("present-presenter-autoplay", time = fmt_secs(a)) };
                ui.label(RichText::new(s).size(13.0).color(weak));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(clock_text()).size(26.0).color(fg).monospace());
                ui.add_space(24.0);
                if w::push_button(ui, &crate::t!("present-timer-reset")).on_hover_text(crate::t!("present-timer-reset-tip")).clicked() {
                    app.stage.timer_start = now;
                    if app.stage.timer_paused.is_some() {
                        app.stage.timer_paused = Some(0.0);
                    }
                }
                let paused = app.stage.timer_paused.is_some();
                if w::push_button(ui, &if paused { crate::t!("present-timer-resume") } else { crate::t!("present-timer-pause") }).on_hover_text(if paused { crate::t!("present-timer-resume-tip") } else { crate::t!("present-timer-pause-tip") }).clicked() {
                    match app.stage.timer_paused.take() {
                        Some(e) => app.stage.timer_start = now - e,
                        None => app.stage.timer_paused = Some(now - app.stage.timer_start),
                    }
                }
                let e = app.stage.elapsed(now) as u64;
                ui.label(RichText::new(format!("{:02}:{:02}:{:02}", e / 3600, e / 60 % 60, e % 60)).size(26.0).color(ACCENT.gamma_multiply(1.3)).monospace());
            });
        });
        ui.add_space(12.0);

        let avail = ui.available_rect_before_wrap();
        let top_h = (avail.height() * 0.58).max(160.0);
        let cur_w = (avail.width() * 0.6).min(top_h * 1.6);
        // Both slides start on one line, under the row that holds the
        // "Next" caption (the current slide used to sit 22 pt higher).
        let cur = Rect::from_min_size(avail.min + EVec2::new(0.0, 22.0), EVec2::new(cur_w, cur_w / 1.6));
        let nxt_w = (avail.width() - cur_w - 24.0).max(120.0);
        let nxt = Rect::from_min_size(egui::pos2(cur.right() + 24.0, avail.top() + 22.0), EVec2::new(nxt_w, nxt_w / 1.6));
        let painter = ui.painter().clone();
        let blank = app.editor.view.present.blank;
        draw_slide(&painter, cur, app.stage.slides.get(step, build), blank, "");
        let next_label = match next {
            Some((s, b)) if b > 0 && Some(s) == step => crate::t!("present-next-build", build = b, count = app.editor.doc.path.get(s).map_or(0, |st| st.builds.len())),
            Some((s, _)) => crate::t!("present-next-step", step = ((s + 1) as u64)),
            None => crate::t!("present-next"),
        };
        painter.text(egui::pos2(nxt.left(), avail.top()), Align2::LEFT_TOP, next_label, FontId::proportional(14.0), weak);
        match next {
            Some((s, b)) => draw_slide(&painter, nxt, app.stage.slides.get(Some(s), b), None, ""),
            None => {
                painter.rect_filled(nxt, 6.0, Color32::from_gray(30));
                let msg = if app.editor.doc.play.looping { crate::t!("present-back-to-start") } else { crate::t!("present-end-of-presentation") };
                painter.text(nxt.center(), Align2::CENTER_CENTER, msg, FontId::proportional(16.0), weak);
            }
        }

        // Notes.
        let notes_top = cur.bottom().max(nxt.bottom()) + 16.0;
        let notes_rect = Rect::from_min_max(egui::pos2(avail.left(), notes_top), egui::pos2(avail.right(), avail.bottom() - 44.0));
        let notes = step.and_then(|s| app.editor.doc.path.get(s)).map(|s| s.notes.clone()).unwrap_or_default();
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(notes_rect));
        egui::ScrollArea::vertical().id_salt("presenter-notes").auto_shrink([false, false]).show(&mut child, |ui| {
            if notes.trim().is_empty() {
                ui.label(RichText::new(crate::t!("present-no-notes")).size(app.stage.notes_size * 0.8).color(weak).italics());
            } else {
                ui.label(RichText::new(notes).size(app.stage.notes_size).color(fg));
            }
        });

        // Bottom bar.
        // One button size (regular, 22 pt) across the bar; they were 30 pt
        // egui buttons next to 19 pt ones.
        let bar = Rect::from_min_max(egui::pos2(avail.left(), avail.bottom() - w::H_REGULAR), avail.max);
        let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
        bar_ui.horizontal_centered(|ui| {
            if w::icon_text_button(ui, crate::icons::Icon::ChevronLeft, &crate::t!("present-previous"), true).clicked() {
                let _ = app.editor.view(ViewCommand::Prev, H);
            }
            if w::icon_text_button(ui, crate::icons::Icon::ChevronRight, &crate::t!("present-next"), true).clicked() {
                let _ = app.editor.view(ViewCommand::Next, H);
            }
            ui.add_space(12.0);
            if w::push_button_ext(ui, &crate::t!("canvas-text-smaller-glyph"), w::Size::Regular, true, Some(36.0)).on_hover_text(crate::t!("present-notes-smaller")).clicked() {
                app.stage.notes_size = (app.stage.notes_size - 2.0).max(12.0);
            }
            if w::push_button_ext(ui, &crate::t!("canvas-text-bigger-glyph"), w::Size::Regular, true, Some(36.0)).on_hover_text(crate::t!("present-notes-larger")).clicked() {
                app.stage.notes_size = (app.stage.notes_size + 2.0).min(48.0);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if w::push_button(ui, &crate::t!("present-end-show")).on_hover_text(crate::i18n::key_text(egui::Modifiers::NONE, egui::Key::Escape)).clicked() {
                    let _ = app.editor.view(ViewCommand::ExitPresent, H);
                }
            });
        });
    });
    // The clock and timer tick once a second.
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}

fn draw_slide(painter: &egui::Painter, r: Rect, tex: Option<&TextureHandle>, blank: Option<Blank>, label: &str) {
    match (blank, tex) {
        (Some(Blank::Black), _) => {
            painter.rect_filled(r, 6.0, Color32::BLACK);
        }
        (Some(Blank::White), _) => {
            painter.rect_filled(r, 6.0, Color32::WHITE);
        }
        (None, Some(t)) => {
            painter.image(t.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        (None, None) => {
            painter.rect_filled(r, 6.0, Color32::from_gray(30));
            painter.text(r.center(), Align2::CENTER_CENTER, crate::t!("present-rendering"), FontId::proportional(13.0), Color32::from_gray(120));
        }
    }
    painter.rect_stroke(r, 6.0, egui::Stroke::new(1.0, Color32::from_gray(60)), egui::StrokeKind::Outside);
    if !label.is_empty() {
        painter.text(r.left_bottom() + EVec2::new(8.0, -8.0), Align2::LEFT_BOTTOM, label, FontId::proportional(12.0), Color32::from_gray(200));
    }
}

fn presenter_keys(app: &mut ZebraApp, ctx: &egui::Context) {
    let events = ctx.input(|i| i.events.clone());
    for ev in events {
        if jump_key(app, &ev) {
            continue;
        }
        let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { continue };
        let cmd = match key {
            Key::ArrowRight | Key::ArrowDown | Key::Space | Key::PageDown | Key::Enter if !modifiers.command => Some(ViewCommand::Next),
            Key::ArrowLeft | Key::ArrowUp | Key::PageUp | Key::Backspace => Some(ViewCommand::Prev),
            Key::Escape => Some(ViewCommand::ExitPresent),
            Key::B => Some(ViewCommand::Blank { blank: if app.blank() == Some(Blank::Black) { None } else { Some(Blank::Black) } }),
            Key::W => Some(ViewCommand::Blank { blank: if app.blank() == Some(Blank::White) { None } else { Some(Blank::White) } }),
            Key::P => Some(ViewCommand::PauseAutoplay { paused: !app.editor.view.present.autoplay_paused }),
            _ => None,
        };
        if let Some(c) = cmd {
            let _ = app.editor.view(c, H);
        }
    }
}

/// Typing a step number while presenting, as in Keynote: digits open the
/// "Go to step" field, Return jumps, Esc or Delete-to-empty closes it.
/// Returns true when the event was used (so Return doesn't also step on and
/// Esc doesn't leave the show).
pub fn jump_key(app: &mut ZebraApp, ev: &egui::Event) -> bool {
    let n = app.editor.doc.path.len();
    match ev {
        egui::Event::Text(t) if n > 0 && t.chars().all(|c| c.is_ascii_digit()) && !t.is_empty() => {
            let buf = app.stage.jump.get_or_insert_with(String::new);
            for c in t.chars() {
                if buf.len() < n.to_string().len() {
                    buf.push(c);
                }
            }
            true
        }
        egui::Event::Key { key, pressed: true, modifiers, .. } if app.stage.jump.is_some() && !modifiers.command => {
            match key {
                Key::Enter => {
                    let typed = app.stage.jump.take().and_then(|b| b.parse::<usize>().ok());
                    if let Some(k) = typed.filter(|k| (1..=n).contains(k)) {
                        let _ = app.editor.view(ViewCommand::GotoStep { step: k - 1 }, H);
                    }
                }
                Key::Escape => app.stage.jump = None,
                Key::Backspace => {
                    if let Some(b) = app.stage.jump.as_mut() {
                        b.pop();
                    }
                }
                _ => {}
            }
            // Digits arrive as Text; their Key events are swallowed too.
            true
        }
        _ => false,
    }
}

/// The step position for the HUD and the presenter view: "4 of 12", or
/// the step being typed.
pub fn jump_text(app: &ZebraApp, long: bool) -> Option<String> {
    let typed = app.stage.jump.as_ref()?;
    let n = app.editor.doc.path.len();
    let step = if typed.is_empty() { "–".to_string() } else { typed.clone() };
    Some(if long { crate::t!("present-presenter-go-to", step = step, count = n) } else { crate::t!("present-hud-go-to", step = step, count = n) })
}

/// Where Next will go: the next build, else the next step. None at the end.
pub fn next_position(ed: &Editor) -> Option<(usize, usize)> {
    let p = &ed.view.present;
    let n = ed.doc.path.len();
    match p.step {
        None => (n > 0).then_some((0, 0)),
        Some(i) => {
            let builds = ed.doc.path.get(i).map(|s| s.builds.len()).unwrap_or(0);
            if p.build < builds && !p.off_path {
                Some((i, p.build + 1))
            } else if i + 1 < n {
                Some((i + 1, 0))
            } else if ed.doc.play.looping {
                Some((0, 0))
            } else {
                None
            }
        }
    }
}

/// Wall clock, local time, "14:05".
fn clock_text() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    #[cfg(unix)]
    {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: localtime_r writes into `tm`, which we own.
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return format!("{:02}:{:02}", tm.tm_hour, tm.tm_min);
        }
    }
    format!("{:02}:{:02}", secs / 3600 % 24, secs / 60 % 60)
}

// ----------------------------------------------------------------------
// Presenter slide images, rendered off the UI thread

pub const SLIDE_W: u32 = 960;
pub const SLIDE_H: u32 = 600;
/// Slide textures kept (current + next, plus the two just shown).
const SLIDES_KEPT: usize = 4;

type SlideKey = (u64, Option<usize>, usize);

/// Large step images for the presenter view, by (revision, step, build).
/// Rendered on a worker with the CPU renderer, a few at a time.
pub struct Slides {
    tx: Sender<(SlideKey, Option<Document>)>,
    rx: Receiver<(SlideKey, Option<ColorImage>)>,
    textures: HashMap<SlideKey, TextureHandle>,
    order: Vec<SlideKey>,
    pending: Vec<SlideKey>,
    rev: u64,
    ctx: egui::Context,
}

impl Slides {
    fn new(ctx: egui::Context, renderer: std::sync::Arc<zebra_render::Renderer>) -> Self {
        let (tx, jrx) = channel::<(SlideKey, Option<Document>)>();
        let (rtx, rx) = channel();
        let wake = ctx.clone();
        std::thread::Builder::new()
            .name("zebra-presenter".into())
            .spawn(move || {
                let mut ed: Option<Editor> = None;
                while let Ok((key, doc)) = jrx.recv() {
                    if let Some(doc) = doc {
                        let mut e = Editor::new(Document::default());
                        e.measure = renderer.measure();
                        e.load(doc, None);
                        e.view.viewport = zebra_core::Vec2::new(SLIDE_W as f64, SLIDE_H as f64);
                        ed = Some(e);
                    }
                    let Some(e) = &ed else { continue };
                    let target = match key.1 {
                        Some(i) => zebra_render::CaptureTarget::Step { index: i, build: Some(key.2) },
                        None => zebra_render::CaptureTarget::Overview,
                    };
                    let img = renderer.capture(e, &target, SLIDE_W, SLIDE_H).ok().and_then(|png| {
                        let im = image::load_from_memory(&png).ok()?.to_rgba8();
                        Some(ColorImage::from_rgba_unmultiplied([im.width() as usize, im.height() as usize], im.as_raw()))
                    });
                    if rtx.send((key, img)).is_err() {
                        break;
                    }
                    wake.request_repaint();
                }
            })
            .expect("spawn presenter thread");
        Slides { tx, rx, textures: HashMap::new(), order: vec![], pending: vec![], rev: u64::MAX, ctx }
    }

    /// Ask for these positions at the editor's revision.
    fn request(&mut self, ed: &Editor, positions: [Option<(Option<usize>, usize)>; 2]) {
        while let Ok((key, img)) = self.rx.try_recv() {
            self.pending.retain(|k| *k != key);
            if let Some(img) = img {
                let t = self.ctx.load_texture(format!("slide-{:?}", key), img, TextureOptions::LINEAR);
                self.textures.insert(key, t);
                self.order.retain(|k| *k != key);
                self.order.push(key);
            }
        }
        let mut doc_sent = false;
        for (s, b) in positions.into_iter().flatten() {
            let key = (ed.revision, s, b);
            if self.textures.contains_key(&key) || self.pending.contains(&key) {
                continue;
            }
            let doc = (self.rev != ed.revision && !doc_sent).then(|| ed.doc.clone());
            if doc.is_some() {
                self.rev = ed.revision;
                doc_sent = true;
            }
            if self.tx.send((key, doc)).is_ok() {
                self.pending.push(key);
            }
        }
        // Keep only the newest few textures (GPU memory budget).
        while self.order.len() > SLIDES_KEPT {
            let k = self.order.remove(0);
            self.textures.remove(&k);
        }
    }

    /// The newest image of a position (any revision, newest preferred).
    fn get(&self, step: Option<usize>, build: usize) -> Option<&TextureHandle> {
        self.order.iter().rev().find(|k| k.1 == step && k.2 == build).and_then(|k| self.textures.get(k))
    }

    /// Bytes held by slide textures (perf query).
    pub fn texture_bytes(&self) -> u64 {
        self.textures.len() as u64 * SLIDE_W as u64 * SLIDE_H as u64 * 4
    }
}

// ----------------------------------------------------------------------
// macOS: screens and menu bar

#[cfg(target_os = "macos")]
mod mac {
    use super::Display;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationPresentationOptions, NSScreen};

    pub fn screens() -> Vec<Display> {
        let Some(mtm) = MainThreadMarker::new() else { return vec![] };
        let screens = NSScreen::screens(mtm);
        let Some(main_h) = screens.firstObject().map(|s| s.frame().size.height) else { return vec![] };
        screens
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let f = s.frame();
                // AppKit: bottom-left origin, y up. egui/winit: top-left, y down.
                let top = main_h - (f.origin.y + f.size.height);
                let name = s.localizedName().to_string();
                Display {
                    name: if name.is_empty() { crate::t!("present-display-n", n = ((i + 1) as u64)) } else { name },
                    frame: egui::Rect::from_min_size(egui::pos2(f.origin.x as f32, top as f32), egui::vec2(f.size.width as f32, f.size.height as f32)),
                    main: i == 0,
                }
            })
            .collect()
    }

    /// Hide the menu bar and Dock while playing on the main display.
    pub fn hide_menu_bar(on_main: bool, hide: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        if !on_main {
            return;
        }
        let app = NSApplication::sharedApplication(mtm);
        let opts = if hide {
            NSApplicationPresentationOptions::HideDock | NSApplicationPresentationOptions::HideMenuBar
        } else {
            NSApplicationPresentationOptions::Default
        };
        app.setPresentationOptions(opts);
    }
}
