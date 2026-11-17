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
}

pub fn show(app: &mut ZebraApp, ctx: &egui::Context) {
    if !app.show_settings {
        ctx.data_mut(|d| d.remove::<u32>(egui::Id::unique("zebra-settings-frames")));
        app.key_capture = None;
        return;
    }
    let title = app.settings_pane.label();
    let vb = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size([520.0, 380.0])
        .with_resizable(false)
        .with_minimize_button(false)
        .with_maximize_button(false);
    let vid = egui::ViewportId::from_hash_of("zebra-settings");
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
        if ctx.input(|i| i.viewport().close_requested() || (!typing_keys && ((i.modifiers.command && i.key_pressed(egui::Key::W)) || i.key_pressed(egui::Key::Escape)))) {
            app.show_settings = false;
        }
        if class == egui::ViewportClass::EmbeddedWindow {
            // No multi-window support (tests): draw as an egui window.
            let mut open = true;
            egui::Window::new(crate::t!("settings-title")).open(&mut open).collapsible(false).resizable(false).default_width(520.0).show(&ctx, |ui| contents(app, ui));
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
    egui::Frame::new().inner_margin(egui::Margin::symmetric(28, 16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        match app.settings_pane {
            Pane::General => general(app, ui),
            Pane::Canvas => canvas(app, ui),
            Pane::Presenting => presenting(app, ui),
            Pane::Keyboard => keyboard(app, ui),
            Pane::Agents => agents(app, ui),
        }
    });
}

/// A settings row: right-aligned label in a 150 pt column, controls right.
fn srow(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let p = Palette::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(150.0, w::H_REGULAR), Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(label).font(style::ui_font(13.0)).color(p.text));
        });
        ui.vertical(|ui| add(ui));
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
    srow(ui, &crate::t!("settings-appearance"), |ui| {
        w::caption(ui, crate::t!("settings-appearance-caption"));
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

/// Settings › Keyboard: every rebindable action and its shortcut. Click a
/// shortcut, then press the new keys; Esc cancels.
fn keyboard(app: &mut ZebraApp, ui: &mut Ui) {
    use crate::keymap;
    let p = Palette::of(ui.ctx());
    // Keys typed while a shortcut is being recorded (the Mac menu bar
    // hands over the ones it owns through `got`).
    let mut recorded_now = false;
    if let Some(cap) = &mut app.key_capture {
        if cap.got.is_none() {
            cap.got = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*modifiers, *key)),
                    _ => None,
                })
            });
        }
        if let Some((m, k)) = cap.got {
            let a = cap.action.clone();
            app.key_capture = None;
            recorded_now = true;
            if k == egui::Key::Escape && m.is_none() {
                // Cancelled.
            } else {
                let keys = crate::actions::fmt_shortcut(m, k);
                match keymap::action_for(m, k) {
                    Some(other) if other != a && !keymap::rebindable().contains(&other) => {
                        app.key_message = Some(crate::t!("settings-keys-taken", keys = keys.as_str(), action = other.label()));
                    }
                    _ => {
                        let before = keymap::set(&a, m, k);
                        app.key_message = before.map(|b| crate::t!("settings-keys-moved", keys = keys.as_str(), action = b.label()));
                    }
                }
            }
        }
    }
    w::caption(ui, crate::t!("settings-keys-caption"));
    let capturing = app.key_capture.as_ref().map(|c| c.action.clone());
    egui::ScrollArea::vertical().max_height(196.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("keymap").num_columns(3).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
            for a in keymap::rebindable() {
                ui.add(egui::Label::new(RichText::new(a.label()).font(style::ui_font(12.0)).color(p.text)).truncate());
                let text = if capturing.as_ref() == Some(&a) {
                    crate::t!("settings-keys-type")
                } else {
                    a.shortcut_text().unwrap_or_else(|| crate::t!("settings-keys-none"))
                };
                // Space or Return just recorded must not press the button again.
                if w::push_button_ext(ui, &text, w::Size::Small, true, Some(110.0)).clicked() && !recorded_now {
                    app.key_message = None;
                    app.key_capture = Some(keymap::Capture { action: a.clone(), got: None });
                }
                let has = keymap::binding_for(&a).is_some();
                if w::icon_button(ui, Icon::Close, 16.0, has, false).on_hover_text(crate::t!("settings-keys-remove")).clicked() {
                    keymap::clear(&a);
                    app.key_message = None;
                }
                ui.end_row();
            }
        });
    });
    if let Some(m) = &app.key_message {
        w::caption(ui, m.as_str());
    }
    ui.horizontal(|ui| {
        let changed = !keymap::overrides().keys.is_empty();
        if w::push_button_ext(ui, &crate::t!("settings-keys-restore"), w::Size::Regular, changed, None).clicked() {
            keymap::reset_all();
            app.key_capture = None;
            app.key_message = None;
        }
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
