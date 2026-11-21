//! Settings window (⌘,): a separate window with the classic preferences
//! toolbar (icon over label, one pane per tab), drawn in egui.
use egui::{Align, Layout, RichText, Ui, Vec2};
use zebra_core::Theme;

use crate::actions::Action;
use crate::app::{ScrollMode, ZebraApp};
use crate::icons::Icon;
use crate::style::{self, Palette};
use crate::widgets::{self as w, ToolbarState};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Pane {
    #[default]
    General,
    Canvas,
    Presenting,
    Keyboard,
    Agents,
}

impl Pane {
    /// The pane's name, shown in its toolbar item and as the window title.
    pub fn label(self) -> String {
        match self {
            Pane::General => crate::t!("settings-general"),
            Pane::Canvas => crate::t!("settings-canvas"),
            Pane::Presenting => crate::t!("settings-presenting"),
            Pane::Keyboard => crate::t!("settings-keyboard"),
            Pane::Agents => crate::t!("settings-agents"),
        }
    }

    /// The window's content size for this pane. Like Mac preference
    /// windows, the window grows for a pane that needs the room.
    pub fn size(self) -> Vec2 {
        match self {
            Pane::Keyboard => Vec2::new(620.0, 520.0),
            // The third permission's label needs the room (it ran past the
            // window's edge at 520).
            Pane::Agents => Vec2::new(600.0, 380.0),
            _ => Vec2::new(520.0, 380.0),
        }
    }
}

/// The Settings window's viewport (hashed into its id).
pub const VIEWPORT: &str = "zebra-settings";

pub fn show(app: &mut ZebraApp, ctx: &egui::Context) {
    if !app.show_settings {
        ctx.data_mut(|d| d.remove::<u32>(egui::Id::unique("zebra-settings-frames")));
        app.key_capture = None;
        return;
    }
    let title = app.settings_pane.label();
    let vb = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(app.settings_pane.size())
        .with_resizable(false)
        .with_minimize_button(false)
        .with_maximize_button(false);
    let vid = egui::ViewportId::from_hash_of(VIEWPORT);
    if !crate::panels::own_window_ready(ctx, vid, &vb) {
        return;
    }
    ctx.show_viewport_immediate(vid, vb, |ui, class| {
        let ctx = ui.ctx().clone();
        // A new native window can come up after our first paint (another
        // Space, not focused yet) and then sits blank: keep painting it
        // until it has been shown for a moment.
        let frames: u32 = ctx.data_mut(|d| {
            let n = d.get_temp_mut_or(egui::Id::unique("zebra-settings-frames"), 0u32);
            *n += 1;
            *n
        });
        if frames < 30 {
            ctx.request_repaint_of(vid);
        }
        // While a shortcut is being typed, Esc and ⌘W belong to it.
        let typing_keys = app.key_capture.is_some();
        // Esc in a text field (the shortcut search) is the field's.
        let in_field = ctx.egui_wants_keyboard_input();
        if ctx.input(|i| i.viewport().close_requested() || (!typing_keys && ((i.modifiers.command && i.key_pressed(egui::Key::W)) || (!in_field && i.key_pressed(egui::Key::Escape))))) {
            app.show_settings = false;
        }
        if class == egui::ViewportClass::EmbeddedWindow {
            // No multi-window support (tests): draw as an egui window.
            let mut open = true;
            egui::Window::new(crate::t!("settings-title")).open(&mut open).collapsible(false).resizable(false).fixed_size(app.settings_pane.size()).show(&ctx, |ui| contents(app, ui));
            if !open {
                app.show_settings = false;
            }
        } else {
            let p = Palette::of(&ctx);
            egui::CentralPanel::default().frame(egui::Frame::new().fill(p.panel)).show(ui, |ui| contents(app, ui));
        }
    });
}

fn contents(app: &mut ZebraApp, ui: &mut Ui) {
    let p = Palette::of(ui.ctx());
    // Preferences toolbar.
    egui::Frame::new().fill(p.toolbar).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let total = 5.0 * 66.0;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            for (pane, icon) in [(Pane::General, Icon::Gear), (Pane::Canvas, Icon::Select), (Pane::Presenting, Icon::Play), (Pane::Keyboard, Icon::Keyboard), (Pane::Agents, Icon::Sparkles)] {
                let label = pane.label();
                ui.allocate_ui(Vec2::new(64.0, 44.0), |ui| {
                    let st = ToolbarState { enabled: true, active: app.settings_pane == pane, ..Default::default() };
                    if w::toolbar_item(ui, icon, &label, st).clicked() {
                        app.settings_pane = pane;
                    }
                });
            }
        });
    });
    w::separator(ui);
    let margin = if app.settings_pane == Pane::Keyboard { egui::Margin::symmetric(20, 14) } else { egui::Margin::symmetric(28, 16) };
    egui::Frame::new().inner_margin(margin).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        match app.settings_pane {
            Pane::General => general(app, ui),
            Pane::Canvas => canvas(app, ui),
            Pane::Presenting => presenting(app, ui),
            Pane::Keyboard => crate::keys_pane::show(app, ui),
            Pane::Agents => agents(app, ui),
        }
    });
}

/// Width of the label column in a settings pane.
const LABEL_COL: f32 = 150.0;

/// A settings row, laid out like a Mac Settings form: the label right-
/// aligned in a fixed column (so every colon lines up, whatever the label's
/// length), 8 pt, then the controls, top-aligned with the label's row.
/// The label column used to shrink to its text, so each row's controls
/// started at a different x.
fn srow(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let p = Palette::of(ui.ctx());
    // One key for every row of this pane (the pane's own Ui).
    let key = ui.scope_id().with("settings-label-col");
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        // The column is as wide as the pane's widest label (at least
        // LABEL_COL), remembered per pane so every row uses the same x.
        let lw = ui.painter().layout_no_wrap(label.to_string(), style::ui_font(13.0), p.text).size().x.ceil();
        let known: f32 = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(LABEL_COL);
        let width = known.max(lw);
        if width > known {
            ui.ctx().data_mut(|d| d.insert_temp(key, width));
            ui.ctx().request_repaint();
        }
        let (col, _) = ui.allocate_exact_size(Vec2::new(width, w::H_REGULAR), egui::Sense::hover());
        ui.scope_builder(egui::UiBuilder::new().max_rect(col).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.label(RichText::new(label).font(style::ui_font(13.0)).color(p.text));
        });
        let width = ui.available_width();
        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 4.0;
            add(ui)
        });
    });
}

fn general(app: &mut ZebraApp, ui: &mut Ui) {
    srow(ui, &crate::t!("settings-default-theme"), |ui| {
        let name = crate::words::theme_name(&Theme::resolve(&app.default_theme));
        w::popup_button(ui, "default-theme", &name, 200.0, |ui| {
            for t in Theme::builtins() {
                if w::menu_check(ui, t.id == app.default_theme, &crate::words::theme_name(&t), None).clicked() {
                    app.default_theme = t.id.clone();
                }
            }
        });
    });
    srow(ui, &crate::t!("settings-language"), |ui| {
        let chosen = crate::i18n::chosen_language();
        let system = crate::t!("settings-language-system", language = crate::i18n::language_name(&crate::i18n::system_language()));
        let label = chosen.as_deref().map(crate::i18n::language_name).unwrap_or_else(|| system.clone());
        let mut pick = None;
        w::popup_button(ui, "language", &label, 200.0, |ui| {
            if w::menu_check(ui, chosen.is_none(), &system, None).clicked() {
                pick = Some(None);
            }
            w::menu_separator(ui);
            for (id, name) in crate::i18n::available() {
                if w::menu_check(ui, chosen.as_deref() == Some(id.as_str()), &name, None).clicked() {
                    pick = Some(Some(id));
                }
            }
        });
        if let Some(p) = pick {
            app.set_language(p.as_deref());
        }
    });
    srow(ui, &crate::t!("settings-appearance"), |ui| {
        w::caption(ui, crate::t!("settings-appearance-caption"));
    });
    // Only the person turns this on or off; agents can only read it.
    srow(ui, &crate::t!("settings-beta"), |ui| {
        let mut on = app.editor.view.beta_features;
        if w::checkbox(ui, &mut on, &crate::t!("settings-beta-switch")).changed() {
            crate::beta_ui::set(app, on);
        }
        w::caption(ui, crate::t!("settings-beta-caption"));
    });
}

fn canvas(app: &mut ZebraApp, ui: &mut Ui) {
    srow(ui, &crate::t!("settings-scrolling"), |ui| {
        let cmd = crate::i18n::modifier_text(egui::Modifiers::COMMAND);
        for (m, l) in [
            (ScrollMode::Auto, crate::t!("settings-scroll-auto")),
            (ScrollMode::Zoom, crate::t!("settings-scroll-zoom")),
            (ScrollMode::Pan, crate::t!("settings-scroll-pan", modifier = cmd)),
        ] {
            if w::radio(ui, app.scroll_mode == m, &l).clicked() {
                app.scroll_mode = m;
            }
        }
    });
    srow(ui, &crate::t!("settings-snapping"), |ui| {
        w::checkbox(ui, &mut app.snapping, &crate::t!("settings-snap-to-objects"));
        w::caption(ui, crate::t!("settings-snap-caption", modifier = crate::i18n::modifier_text(egui::Modifiers::ALT)));
    });
}

fn presenting(app: &mut ZebraApp, ui: &mut Ui) {
    srow(ui, &crate::t!("settings-reduce-motion"), |ui| {
        let motion_label = |v: Option<bool>| match v {
            None => crate::t!("settings-follow-system"),
            Some(true) => crate::t!("settings-on"),
            Some(false) => crate::t!("settings-off"),
        };
        let label = motion_label(app.motion_override);
        let mut pick: Option<Option<bool>> = None;
        w::popup_button(ui, "motion", &label, 200.0, |ui| {
            for v in [None, Some(true), Some(false)] {
                if w::menu_check(ui, app.motion_override == v, &motion_label(v), None).clicked() {
                    pick = Some(v);
                }
            }
        });
        if let Some(v) = pick {
            app.motion_override = v;
            let on = v.unwrap_or_else(crate::mac::system_reduce_motion);
            let _ = app.editor.view(zebra_core::ViewCommand::SetReduceMotion { on }, zebra_core::Origin::Human);
        }
        w::caption(ui, crate::t!("settings-reduce-motion-caption"));
    });
}

fn agents(app: &mut ZebraApp, ui: &mut Ui) {
    srow(ui, &crate::t!("settings-ai-control"), |ui| {
        let mut on = app.ai_enabled;
        let switch = crate::t!("settings-ai-switch");
        ui.horizontal(|ui| {
            if w::switch(ui, &mut on, &switch).changed() {
                app.queued.push(Action::ToggleAiControl);
            }
            ui.label(&switch);
        });
        w::caption(ui, crate::t!("settings-ai-switch-caption"));
    });
    srow(ui, &crate::t!("settings-agents-may"), |ui| {
        let cur = app.editor.view.agent_permission;
        for (perm, label) in [
            (zebra_core::view::Permission::Observe, crate::t!("settings-permission-observe")),
            (zebra_core::view::Permission::Edit, crate::t!("settings-permission-edit")),
            (zebra_core::view::Permission::Full, crate::t!("settings-permission-full")),
        ] {
            if w::radio(ui, cur == perm, &label).clicked() && cur != perm {
                app.queued.push(Action::SetAgentPermission(perm));
            }
        }
    });
    if let Some(installed) = crate::cli::install_state() {
        srow(ui, &crate::t!("settings-cli"), |ui| {
            if installed {
                w::caption(ui, crate::t!("settings-cli-installed"));
            } else if w::push_button(ui, &crate::t!("settings-cli-install")).clicked() {
                match crate::cli::install() {
                    Ok(()) => app.toast(crate::t!("settings-toast-cli-installed")),
                    Err(e) => app.toast(crate::t!("settings-toast-cli-install-failed", error = e.to_string())),
                }
            }
        });
    }
}
