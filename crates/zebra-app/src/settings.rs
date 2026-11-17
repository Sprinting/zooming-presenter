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
    Agents,
}

pub fn show(app: &mut ZebraApp, ctx: &egui::Context) {
    if !app.show_settings {
        ctx.data_mut(|d| d.remove::<u32>(egui::Id::unique("zebra-settings-frames")));
        return;
    }
    let title = match app.settings_pane {
        Pane::General => "General",
        Pane::Canvas => "Canvas",
        Pane::Presenting => "Presenting",
        Pane::Agents => "Agents",
    };
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
        if ctx.input(|i| i.viewport().close_requested() || (i.modifiers.command && i.key_pressed(egui::Key::W)) || i.key_pressed(egui::Key::Escape)) {
            app.show_settings = false;
        }
        if class == egui::ViewportClass::EmbeddedWindow {
            // No multi-window support (tests): draw as an egui window.
            let mut open = true;
            egui::Window::new("Settings").open(&mut open).collapsible(false).resizable(false).default_width(520.0).show(&ctx, |ui| contents(app, ui));
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
            let total = 4.0 * 66.0;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            for (pane, icon, label) in [(Pane::General, Icon::Gear, "General"), (Pane::Canvas, Icon::Select, "Canvas"), (Pane::Presenting, Icon::Play, "Presenting"), (Pane::Agents, Icon::Sparkles, "Agents")] {
                ui.allocate_ui(Vec2::new(64.0, 44.0), |ui| {
                    let st = ToolbarState { enabled: true, active: app.settings_pane == pane, ..Default::default() };
                    if w::toolbar_item(ui, icon, label, st).clicked() {
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
    srow(ui, "Theme for new presentations:", |ui| {
        let name = Theme::resolve(&app.default_theme).name;
        w::popup_button(ui, "default-theme", &name, 200.0, |ui| {
            for t in Theme::builtins() {
                if w::menu_check(ui, t.id == app.default_theme, &t.name, None).clicked() {
                    app.default_theme = t.id.clone();
                }
            }
        });
    });
    srow(ui, "Appearance:", |ui| {
        w::caption(ui, "Zebra follows the system's Light or Dark appearance and accent colour. The canvas uses the presentation's theme.");
    });
}

fn canvas(app: &mut ZebraApp, ui: &mut Ui) {
    srow(ui, "Scrolling:", |ui| {
        for (m, l) in [(ScrollMode::Auto, "Trackpad pans, mouse wheel zooms"), (ScrollMode::Zoom, "Always zoom (classic Prezi)"), (ScrollMode::Pan, "Always pan; ⌘-scroll zooms")] {
            if w::radio(ui, app.scroll_mode == m, l).clicked() {
                app.scroll_mode = m;
            }
        }
    });
    srow(ui, "Snapping:", |ui| {
        w::checkbox(ui, &mut app.snapping, "Snap to other objects while moving");
        w::caption(ui, "Hold ⌥ while dragging to move freely.");
    });
}

fn presenting(app: &mut ZebraApp, ui: &mut Ui) {
    srow(ui, "Reduce motion:", |ui| {
        let label = match app.motion_override {
            None => "Follow System Setting",
            Some(true) => "On",
            Some(false) => "Off",
        };
        let mut pick: Option<Option<bool>> = None;
        w::popup_button(ui, "motion", label, 200.0, |ui| {
            for (v, l) in [(None, "Follow System Setting"), (Some(true), "On"), (Some(false), "Off")] {
                if w::menu_check(ui, app.motion_override == v, l, None).clicked() {
                    pick = Some(v);
                }
            }
        });
        if let Some(v) = pick {
            app.motion_override = v;
            let on = v.unwrap_or_else(crate::mac::system_reduce_motion);
            let _ = app.editor.view(zebra_core::ViewCommand::SetReduceMotion { on }, zebra_core::Origin::Human);
        }
        w::caption(ui, "On: steps cross-fade instead of flying and rotating.");
    });
}

fn agents(app: &mut ZebraApp, ui: &mut Ui) {
    srow(ui, "AI control:", |ui| {
        let mut on = app.ai_enabled;
        ui.horizontal(|ui| {
            if w::switch(ui, &mut on, "Let AI agents control Zebra").changed() {
                app.queued.push(Action::ToggleAiControl);
            }
            ui.label("Let AI agents control Zebra");
        });
        w::caption(ui, "Only you can switch this on. An agent can switch it off.");
    });
    srow(ui, "Agents may:", |ui| {
        let cur = app.editor.view.agent_permission;
        for (perm, label) in [
            (zebra_core::view::Permission::Observe, "Look only"),
            (zebra_core::view::Permission::Edit, "Edit the presentation"),
            (zebra_core::view::Permission::Full, "Also move the view, open and save files, and present"),
        ] {
            if w::radio(ui, cur == perm, label).clicked() && cur != perm {
                app.queued.push(Action::SetAgentPermission(perm));
            }
        }
    });
    if let Some(installed) = crate::cli::install_state() {
        srow(ui, "Command-line tool:", |ui| {
            if installed {
                w::caption(ui, "zebractl is installed in /usr/local/bin.");
            } else if w::push_button(ui, "Install zebractl…").clicked() {
                match crate::cli::install() {
                    Ok(()) => app.toast("Installed zebractl."),
                    Err(e) => app.toast(format!("Couldn't install zebractl: {e}")),
                }
            }
        });
    }
}
