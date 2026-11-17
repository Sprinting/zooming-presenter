//! Editor chrome: top bar, path sidebar, bottom tool bar, zoom stack,
//! properties / agent activity panel, dialogs. Plain direct manipulation:
//! nothing here requires knowing a command.
use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2 as EVec2};
use zebra_core::{
    Command, ElementKind, ElementPatch, FrameStyle, Mode, Origin, ShapeKind, StepTarget, TextStyle, Theme, Tool,
};

use crate::actions::{frame_name, tool_name, Action};
use crate::app::{ScrollMode, ZebraApp};
use crate::style::{self, Palette, ACCENT, AGENT, PRESENT};

const H: Origin = Origin::Human;

pub fn layout(app: &mut ZebraApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let p = Palette::of(&ctx);

    #[cfg(not(target_os = "macos"))]
    egui::Panel::top("menubar")
        .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(8, 2)))
        .show(ui, |ui| crate::menus::egui_menu_bar(app, ui));

    egui::Panel::top("topbar")
        .exact_size(50.0)
        .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(12, 8)).stroke(Stroke::new(1.0, p.border)))
        .show(ui, |ui| top_bar(app, ui, &p));

    if app.show_sidebar {
        egui::Panel::left("path")
            .exact_size(212.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(10, 10)).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| crate::sidebar::ui(app, ui, &p));
    }

    let has_sel = !app.editor.view.selection.is_empty();
    if app.show_activity || (app.show_props && has_sel) {
        egui::Panel::right("props")
            .exact_size(256.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(12, 10)).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(228.0);
                    if app.show_activity {
                        activity_panel(app, ui, &p);
                    } else {
                        properties(app, ui, &p);
                    }
                });
            });
    }

    egui::CentralPanel::no_frame().show(ui, |ui| {
        crate::canvas::edit_canvas(app, ui);
    });
    let rect = app.canvas.rect;
    floating(app, &ctx, rect, &p);
    dialogs(app, &ctx, &p);
}

// ----------------------------------------------------------------------
// Top bar

fn top_bar(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    ui.horizontal_centered(|ui| {
        // Logo: zebra-striped disc.
        let (r, _) = ui.allocate_exact_size(EVec2::splat(24.0), Sense::hover());
        crate::canvas::draw_zebra_disc(ui.painter(), r.center(), 10.0);
        ui.add_space(4.0);

        // Editable document title.
        let mut title = app.title_buf.clone().unwrap_or_else(|| app.editor.doc.title.clone());
        let title_w = ui.fonts_mut(|f| f.layout_no_wrap(title.clone(), style::bold(15.0), p.text).size().x + 12.0).clamp(60.0, 320.0);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut title)
                .font(style::bold(15.0))
                .frame(egui::Frame::NONE)
                .desired_width(title_w)
                .id(egui::Id::unique("doc-title")),
        );
        if resp.has_focus() {
            app.title_buf = Some(title.clone());
        }
        if resp.lost_focus() {
            app.title_buf = None;
            let t = title.trim().to_string();
            if !t.is_empty() && t != app.editor.doc.title {
                app.apply(Command::SetTitle { title: t });
            }
        }
        resp.on_hover_text("Rename presentation");
        let status = if app.editor.dirty() { "Edited" } else if app.editor.file_path.is_some() { "Saved" } else { "Not saved yet" };
        ui.label(RichText::new(status).size(12.0).color(p.weak));

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        let undo_tip = app.editor.undo_label().map(|l| format!("Undo {l}  ⌘Z")).unwrap_or_else(|| "Nothing to undo".into());
        if icon_button(ui, Icon::Undo, app.editor.can_undo(), false, p).on_hover_text(undo_tip).clicked() {
            app.queued.push(Action::Undo);
        }
        if icon_button(ui, Icon::Redo, app.editor.can_redo(), false, p).on_hover_text("Redo  ⇧⌘Z").clicked() {
            app.queued.push(Action::Redo);
        }

        if app.editor.view.mode == Mode::PathEdit {
            ui.add_space(12.0);
            egui::Frame::new().fill(style::PATH).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
                ui.label(RichText::new("Editing path — click objects to add steps · Esc to finish").size(12.0).color(Color32::WHITE));
            });
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Present (primary).
            let present = egui::Button::new(RichText::new("▶  Present").size(14.0).color(Color32::WHITE).strong())
                .fill(PRESENT)
                .corner_radius(CornerRadius::same(8))
                .min_size(EVec2::new(104.0, 32.0));
            let r = ui.add(present).on_hover_text("Present from the start  F5 / ⌘↵\nFrom the current step  ⇧F5 / ⌥⌘↵");
            if r.clicked() {
                app.queued.push(Action::PresentStart);
            }
            egui::Popup::context_menu(&r).show(|ui| {
                if ui.button("Present from Start").clicked() {
                    app.queued.push(Action::PresentStart);
                }
                if ui.button("Present from Current Step").clicked() {
                    app.queued.push(Action::PresentCurrent);
                }
                ui.separator();
                if ui.button("Playback Options…").clicked() {
                    app.queued.push(Action::PlaybackOptions);
                }
                let mut pv = app.stage.presenter_view;
                if ui.checkbox(&mut pv, "Presenter View").clicked() {
                    app.queued.push(Action::TogglePresenterView);
                }
            });
            ui.add_space(4.0);
            let ex = ui.add(egui::Button::new("Export").min_size(EVec2::new(0.0, 30.0))).on_hover_text("Export as PDF or PNG");
            egui::Popup::menu(&ex).show(|ui| {
                if ui.button("PDF — one page per step…").clicked() {
                    app.queued.push(Action::ExportPdf);
                }
                if ui.button("PNG of current view…").clicked() {
                    app.queued.push(Action::ExportPng);
                }
            });
            let th = ui.add(egui::Button::new(format!("Theme: {}", app.editor.theme().name)).min_size(EVec2::new(0.0, 30.0))).on_hover_text("Change the presentation's colours and fonts");
            egui::Popup::menu(&th).width(220.0).show(|ui| theme_menu(app, ui));
            ui.add_space(4.0);
            agent_pill(app, ui, p);
        });
    });
}

fn theme_menu(app: &mut ZebraApp, ui: &mut Ui) {
    let all: Vec<Theme> = Theme::builtins().into_iter().chain(app.editor.doc.themes.iter().cloned()).collect();
    for t in all {
        let selected = t.id == app.editor.doc.theme;
        let (r, resp) = ui.allocate_exact_size(EVec2::new(210.0, 40.0), Sense::click());
        let painter = ui.painter();
        if resp.hovered() || selected {
            painter.rect_filled(r, 6.0, ui.visuals().widgets.hovered.bg_fill);
        }
        let sw = Rect::from_min_size(r.min + EVec2::new(6.0, 5.0), EVec2::new(46.0, 30.0));
        painter.rect(sw, 4.0, style::c32(t.color(&"theme:background".into())), Stroke::new(1.0, Color32::from_gray(200)), egui::StrokeKind::Inside);
        // bracket + title line
        let fc = style::c32(t.color(&"theme:frame".into()));
        let st = Stroke::new(1.5, fc);
        painter.add(Shape::line(vec![sw.left_top() + EVec2::new(10.0, 5.0), sw.left_top() + EVec2::new(6.0, 5.0), sw.left_bottom() + EVec2::new(6.0, -5.0), sw.left_bottom() + EVec2::new(10.0, -5.0)], st));
        painter.add(Shape::line(vec![sw.right_top() + EVec2::new(-10.0, 5.0), sw.right_top() + EVec2::new(-6.0, 5.0), sw.right_bottom() + EVec2::new(-6.0, -5.0), sw.right_bottom() + EVec2::new(-10.0, -5.0)], st));
        painter.line_segment([sw.center() + EVec2::new(-12.0, -3.0), sw.center() + EVec2::new(12.0, -3.0)], Stroke::new(3.0, style::c32(t.color(&"theme:title".into()))));
        painter.line_segment([sw.center() + EVec2::new(-9.0, 4.0), sw.center() + EVec2::new(9.0, 4.0)], Stroke::new(1.5, style::c32(t.color(&"theme:subtitle".into()))));
        painter.text(r.min + EVec2::new(62.0, 20.0), Align2::LEFT_CENTER, &t.name, FontId::proportional(13.5), ui.visuals().text_color());
        if selected {
            painter.text(r.right_center() - EVec2::new(10.0, 0.0), Align2::RIGHT_CENTER, "✓", FontId::proportional(14.0), ACCENT);
        }
        if resp.clicked() {
            app.queued.push(Action::SetTheme(t.id.clone()));
            ui.close();
        }
    }    let mine = crate::library::list_themes();
    if !mine.is_empty() {
        ui.separator();
        ui.label(RichText::new("My Themes").size(11.0).weak());
        for (_, doc, t) in mine {
            if ui.button(&t.name).clicked() {
                crate::theme_editor::apply_library_theme(app, &doc, &t);
                ui.close();
            }
        }
    }
    ui.separator();
    if ui.button("Customize Theme…").on_hover_text("Colours, fonts, background picture and 3D background").clicked() {
        app.queued.push(Action::ThemeEditor);
        ui.close();
    }
}

fn agent_pill(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let on = app.api.is_some() || app.demo_agent.is_some();
    let connected = app.agent_connected();
    let paused = app.editor.view.agent_paused;
    let (dot, text) = if let Some(e) = &app.api_error {
        (Color32::from_rgb(220, 60, 60), format!("AI: error · {}", crate::files::short(e, 32)))
    } else if !on {
        (p.weak, "AI: Off".to_string())
    } else if !connected {
        (Color32::from_rgb(70, 170, 90), "AI: On · waiting".to_string())
    } else if paused {
        (Color32::from_rgb(230, 160, 40), format!("{} · paused", app.agent_name()))
    } else {
        let busy = app.editor.activity.back().is_some_and(|a| a.origin.is_agent() && app.editor.view.now - a.time < 2.0);
        (AGENT, format!("{} connected · {}", app.agent_name(), if busy { "editing" } else { "idle" }))
    };
    let fill = if connected && !paused { AGENT.gamma_multiply(0.12) } else { p.hover };
    let tc = if connected { AGENT } else { p.weak };
    let galley = ui.painter().layout_no_wrap(text.clone(), FontId::proportional(12.5), tc);
    let size = EVec2::new(galley.size().x + 34.0, 28.0);
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &text));
    ui.painter().rect_filled(rect, 14.0, if r.hovered() { fill.gamma_multiply(1.6) } else { fill });
    ui.painter().circle_filled(Pos2::new(rect.min.x + 14.0, rect.center().y), 4.0, dot);
    ui.painter().galley(Pos2::new(rect.min.x + 24.0, rect.center().y - galley.size().y / 2.0), galley, tc);
    let r = r.on_hover_text(if on { "AI control is on. Click for agent controls" } else { "Let an AI agent work in Zebra" });
    let mut popup = egui::Popup::menu(&r);
    if std::mem::take(&mut app.open_ai_panel) {
        popup = popup.open_memory(egui::SetOpenCommand::Bool(true));
    }
    popup.width(320.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| ai_panel(app, ui, on, connected, paused));
}

/// The AI popover: the human-only switch, how to connect an agent, and
/// (when on) the agent controls.
fn ai_panel(app: &mut ZebraApp, ui: &mut Ui, on: bool, connected: bool, paused: bool) {
    let weak = ui.visuals().weak_text_color();
    let mut want = on;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Let AI agents control Zebra").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if toggle_switch(ui, &mut want).on_hover_text("Only you can switch this on. An agent can switch it off.").changed() {
                app.queued.push(Action::ToggleAiControl);
            }
        });
    });
    ui.label(
        RichText::new(if on {
            "Agents on this Mac that can run a command (Claude Code, Codex, Cursor, Gemini CLI, your own scripts) can now see and edit this window. You can watch, undo, pause or stop them at any time."
        } else {
            "Off: nothing outside Zebra can see or change your work. Switch it on to let an AI agent on this Mac build or edit presentations with you."
        })
        .small()
        .color(weak),
    );
    if let Some(e) = &app.api_error {
        ui.label(RichText::new(e).small().color(Color32::from_rgb(220, 60, 60)));
    }
    if !on {
        return;
    }

    ui.separator();
    ui.label(RichText::new("Connect an agent").strong());
    ui.label(RichText::new("Paste this into your agent's chat:").small().color(weak));
    let ctl = crate::cli::zebractl_command();
    let prompt = crate::cli::agent_prompt(&ctl);
    copy_box(ui, &prompt, "agent-prompt");
    ui.horizontal_wrapped(|ui| {
        if let Some(installed) = crate::cli::install_state() {
            if installed {
                ui.label(RichText::new("✓ zebractl is installed on this Mac").small().color(weak));
            } else if ui.button("Install command-line tool…").on_hover_text("Adds `zebractl` to /usr/local/bin so agents can type just `zebractl`. macOS asks for your password.").clicked() {
                match crate::cli::install() {
                    Ok(()) => app.toast("Installed zebractl. Agents can now run `zebractl --help`."),
                    Err(e) => app.toast(format!("Couldn't install zebractl: {e}")),
                }
            }
        }
        ui.menu_button("Use with MCP", |ui| {
            ui.set_max_width(320.0);
            ui.label(RichText::new("Optional. For apps that take an MCP server config (Claude Desktop, Cursor, and others):").small().color(weak));
            copy_box(ui, &crate::cli::mcp_config(&ctl), "mcp-json");
        });
    });

    ui.separator();
    if !connected {
        ui.label(RichText::new("No agent connected yet.").small().color(weak));
    }
    if connected {
        if ui.button(if paused { "Resume agent" } else { "Pause agent" }).on_hover_text("Temporarily stop the agent from making changes").clicked() {
            app.queued.push(Action::PauseAgents);
        }
        let mut follow = app.editor.view.follow_agent;
        if ui.checkbox(&mut follow, "Follow agent's camera").on_hover_text("Move your view to wherever the agent is working").changed() {
            app.queued.push(Action::FollowAgent);
        }
        if ui.button("Revert agent's changes").on_hover_text("Undo everything the agent did since your last change").clicked() {
            app.queued.push(Action::RevertAgent);
        }
    }
    if ui.button("Activity…  (⌘J)").clicked() {
        app.queued.push(Action::ToggleActivity);
    }
    ui.add_space(2.0);
    ui.label(RichText::new("What agents may do").small().color(weak));
    let cur = app.editor.view.agent_permission;
    for (perm, label) in [
        (zebra_core::view::Permission::Observe, "Look only"),
        (zebra_core::view::Permission::Edit, "Edit the presentation (default)"),
        (zebra_core::view::Permission::Full, "Full control (also your view, files, presenting)"),
    ] {
        if ui.radio(cur == perm, label).clicked() {
            let _ = app.editor.view(zebra_core::ViewCommand::SetAgentPermission { permission: perm }, H);
        }
    }
}

/// A read-only text block with a Copy button.
fn copy_box(ui: &mut Ui, text: &str, id: &str) {
    egui::Frame::new().fill(ui.visuals().extreme_bg_color).corner_radius(6.0).inner_margin(8.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(text).monospace().size(11.0)).wrap());
    });
    let copied_id = ui.scope_id().with(("copied", id));
    let copied_at: Option<f64> = ui.data(|d| d.get_temp(copied_id));
    let now = ui.input(|i| i.time);
    let label = if copied_at.is_some_and(|t| now - t < 1.5) { "Copied ✓" } else { "Copy" };
    if ui.small_button(label).clicked() {
        ui.ctx().copy_text(text.to_string());
        ui.data_mut(|d| d.insert_temp(copied_id, now));
    }
}

/// iOS/macOS-style on/off switch.
fn toggle_switch(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = EVec2::new(36.0, 20.0);
    let (rect, mut r) = ui.allocate_exact_size(size, Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, ui.is_enabled(), *on, "Let AI agents control Zebra"));
    let t = ui.ctx().animate_bool_responsive(r.id, *on);
    let bg = Color32::from_gray(190).lerp_to_gamma(Color32::from_rgb(52, 199, 89), t);
    ui.painter().rect_filled(rect, 10.0, bg);
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    ui.painter().circle(Pos2::new(x, rect.center().y), 8.0, Color32::WHITE, Stroke::new(0.5, Color32::from_gray(150)));
    r
}

// ----------------------------------------------------------------------
// Icons (painted, so the chrome needs no icon font)

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Select,
    Hand,
    Text,
    Frame(FrameStyle),
    Rect,
    Ellipse,
    Line,
    Arrow,
    Pen,
    Image,
    Undo,
    Redo,
    Plus,
    Minus,
    Home,
}

pub fn paint_icon(painter: &egui::Painter, r: Rect, icon: Icon, c: Color32) {
    let s = Stroke::new(1.6, c);
    let m = r.center();
    let u = r.width().min(r.height()) / 20.0; // unit
    let p = |x: f32, y: f32| m + EVec2::new(x * u, y * u);
    match icon {
        Icon::Select => {
            painter.add(Shape::convex_polygon(vec![p(-4.0, -7.0), p(-4.0, 6.0), p(-0.8, 3.0), p(1.6, 8.0), p(3.6, 7.0), p(1.2, 2.2), p(5.4, 2.0)], Color32::TRANSPARENT, s));
            painter.add(Shape::closed_line(vec![p(-4.0, -7.0), p(-4.0, 6.0), p(-0.8, 3.0), p(1.6, 8.0), p(3.6, 7.0), p(1.2, 2.2), p(5.4, 2.0)], s));
        }
        Icon::Hand => {
            for (x, top) in [(-3.6, -4.0), (-1.2, -6.5), (1.2, -6.0), (3.6, -4.0)] {
                painter.line_segment([p(x, top), p(x, 1.0)], Stroke::new(1.8, c));
            }
            painter.add(Shape::line(vec![p(-6.5, 0.0), p(-4.6, 4.5), p(-2.0, 7.5), p(3.0, 7.5), p(4.8, 3.0), p(4.8, -2.5)], s));
            painter.line_segment([p(-6.5, 0.0), p(-4.6, 1.5)], s);
        }
        Icon::Text => {
            painter.text(m, Align2::CENTER_CENTER, "T", style::bold(16.0 * u), c);
        }
        Icon::Frame(st) => match st {
            FrameStyle::Circle => {
                painter.circle_stroke(m, 7.0 * u, s);
            }
            FrameStyle::Rectangle => {
                painter.rect(Rect::from_center_size(m, EVec2::new(15.0 * u, 11.0 * u)), 1.0, c.gamma_multiply(0.25), s, egui::StrokeKind::Middle);
            }
            FrameStyle::Invisible => {
                let pts = vec![p(-7.5, -5.5), p(7.5, -5.5), p(7.5, 5.5), p(-7.5, 5.5), p(-7.5, -5.5)];
                painter.extend(Shape::dashed_line(&pts, s, 2.5 * u, 2.0 * u));
            }
            FrameStyle::Bracket => {
                painter.add(Shape::line(vec![p(-4.0, -6.5), p(-7.0, -6.5), p(-7.0, 6.5), p(-4.0, 6.5)], Stroke::new(2.0, c)));
                painter.add(Shape::line(vec![p(4.0, -6.5), p(7.0, -6.5), p(7.0, 6.5), p(4.0, 6.5)], Stroke::new(2.0, c)));
            }
        },
        Icon::Rect => {
            painter.rect_stroke(Rect::from_center_size(m, EVec2::new(14.0 * u, 11.0 * u)), 2.0, s, egui::StrokeKind::Middle);
        }
        Icon::Ellipse => {
            painter.circle_stroke(m, 6.8 * u, s);
        }
        Icon::Line => {
            painter.line_segment([p(-6.5, 6.5), p(6.5, -6.5)], s);
        }
        Icon::Arrow => {
            painter.line_segment([p(-6.5, 6.5), p(6.0, -6.0)], s);
            painter.add(Shape::line(vec![p(0.0, -6.5), p(6.5, -6.5), p(6.5, 0.0)], s));
        }
        Icon::Pen => {
            painter.add(Shape::closed_line(vec![p(-6.5, 6.5), p(-5.5, 2.8), p(3.8, -6.5), p(6.5, -3.8), p(-2.8, 5.5)], s));
            painter.line_segment([p(1.8, -4.5), p(4.5, -1.8)], s);
        }
        Icon::Image => {
            let rr = Rect::from_center_size(m, EVec2::new(15.0 * u, 12.0 * u));
            painter.rect_stroke(rr, 2.0, s, egui::StrokeKind::Middle);
            painter.add(Shape::line(vec![p(-7.0, 4.5), p(-2.5, -0.5), p(1.0, 2.5), p(3.0, 0.8), p(7.0, 4.5)], s));
            painter.circle_filled(p(3.0, -2.8), 1.4 * u, c);
        }
        Icon::Undo | Icon::Redo => {
            let flip = if icon == Icon::Redo { -1.0 } else { 1.0 };
            let q = |x: f32, y: f32| p(x * flip, y);
            let arc: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 - i as f32 / 12.0) * 1.0 - std::f32::consts::FRAC_PI_2 * 0.0;
                    q(-5.0 * a.cos() * -1.0 + 0.0, -5.5 * a.sin() + 3.0)
                })
                .collect();
            let _ = arc;
            // simple hook arrow
            let path: Vec<Pos2> = (0..=14)
                .map(|i| {
                    let t = i as f32 / 14.0;
                    let a = std::f32::consts::PI * (0.5 + t);
                    q(1.5 + 5.0 * -a.cos(), 0.5 - 4.5 * a.sin())
                })
                .collect();
            let start = path[0];
            let mut full = vec![q(-6.5, 5.0)];
            full.extend(path.into_iter().rev());
            let _ = start;
            painter.add(Shape::line(vec![q(-6.0, -4.0), q(2.0, -4.0)], s));
            let curve: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 12.0;
                    q(2.0 + 4.0 * a.cos(), 0.0 + 4.0 * a.sin())
                })
                .collect();
            painter.add(Shape::line(curve, s));
            painter.add(Shape::line(vec![q(2.0, 4.0), q(-3.0, 4.0)], s));
            painter.add(Shape::line(vec![q(-3.0, -7.0), q(-6.0, -4.0), q(-3.0, -1.0)], s));
            let _ = full;
        }
        Icon::Plus => {
            painter.line_segment([p(-6.0, 0.0), p(6.0, 0.0)], s);
            painter.line_segment([p(0.0, -6.0), p(0.0, 6.0)], s);
        }
        Icon::Minus => {
            painter.line_segment([p(-6.0, 0.0), p(6.0, 0.0)], s);
        }
        Icon::Home => {
            painter.add(Shape::closed_line(vec![p(-6.5, -0.5), p(0.0, -7.0), p(6.5, -0.5), p(4.5, -0.5), p(4.5, 6.5), p(-4.5, 6.5), p(-4.5, -0.5)], s));
        }
    }
}

pub fn icon_button(ui: &mut Ui, icon: Icon, enabled: bool, active: bool, p: &Palette) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(EVec2::splat(32.0), if enabled { Sense::click() } else { Sense::hover() });
    let painter = ui.painter();
    if active {
        painter.rect_filled(r, 7.0, ACCENT.gamma_multiply(if p.dark { 0.45 } else { 0.14 }));
    } else if resp.hovered() && enabled {
        painter.rect_filled(r, 7.0, p.hover);
    }
    let c = if !enabled { p.weak.gamma_multiply(0.5) } else if active { ACCENT } else { p.text };
    paint_icon(painter, r.shrink(6.0), icon, c);
    resp
}

// ----------------------------------------------------------------------
// Floating tool bar, zoom stack, hints

fn floating(app: &mut ZebraApp, ctx: &egui::Context, rect: Rect, p: &Palette) {
    // Bottom-centre tool bar.
    egui::Area::new(egui::Id::unique("toolbar"))
        .anchor(Align2::CENTER_BOTTOM, EVec2::new(rect.center().x - ctx.content_rect().center().x, -(ctx.content_rect().bottom() - rect.bottom()) - 16.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            style::float_frame(p).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let cur = app.editor.view.tool;
                    let fs = app.editor.view.frame_style;
                    let items: [(Icon, Option<Tool>, &str, Action); 10] = [
                        (Icon::Select, Some(Tool::Select), "Select  V", Action::SetTool(Tool::Select)),
                        (Icon::Hand, Some(Tool::Hand), "Hand — drag to pan  H  (or hold Space)", Action::SetTool(Tool::Hand)),
                        (Icon::Text, Some(Tool::Text), "Text  T  (or double-click anywhere)", Action::SetTool(Tool::Text)),
                        (Icon::Frame(fs), Some(Tool::Frame), "Frame  F  (press F again to change style)", Action::SetTool(Tool::Frame)),
                        (Icon::Rect, Some(Tool::Rectangle), "Rectangle  R", Action::SetTool(Tool::Rectangle)),
                        (Icon::Ellipse, Some(Tool::Ellipse), "Ellipse  O", Action::SetTool(Tool::Ellipse)),
                        (Icon::Line, Some(Tool::Line), "Line  L", Action::SetTool(Tool::Line)),
                        (Icon::Arrow, Some(Tool::Arrow), "Arrow  A", Action::SetTool(Tool::Arrow)),
                        (Icon::Pen, Some(Tool::Pen), "Pen  D", Action::SetTool(Tool::Pen)),
                        (Icon::Image, None, "Insert image…  I  (or drop files on the canvas)", Action::InsertImage),
                    ];
                    for (icon, tool, tip, action) in items {
                        let active = tool == Some(cur);
                        let r = icon_button(ui, icon, true, active, p).on_hover_text(tip);
                        if r.clicked() {
                            app.queued.push(action);
                        }
                        if tool == Some(Tool::Frame) {
                            // small style picker caret
                            let (cr, cresp) = ui.allocate_exact_size(EVec2::new(12.0, 32.0), Sense::click());
                            let c = cr.center();
                            ui.painter().add(Shape::convex_polygon(vec![c + EVec2::new(-3.5, -2.0), c + EVec2::new(3.5, -2.0), c + EVec2::new(0.0, 2.5)], p.weak, Stroke::NONE));
                            egui::Popup::menu(&cresp).show(|ui| {
                                for s in FrameStyle::ALL {
                                    ui.horizontal(|ui| {
                                        let (ir, _) = ui.allocate_exact_size(EVec2::splat(20.0), Sense::hover());
                                        paint_icon(ui.painter(), ir, Icon::Frame(s), ui.visuals().text_color());
                                        if ui.selectable_label(fs == s, frame_name(s)).clicked() {
                                            app.queued.push(Action::SetFrameStyle(s));
                                            ui.close();
                                        }
                                    });
                                }
                            });
                        }
                        if tool == Some(Tool::Text) {
                            ui.add_space(2.0);
                            ui.separator();
                            ui.add_space(2.0);
                        }
                    }
                });
            });
        });

    // Right-edge zoom stack (+ − ⌂), Prezi style.
    egui::Area::new(egui::Id::unique("zoomstack"))
        .fixed_pos(Pos2::new(rect.right() - 54.0, rect.center().y - 60.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            style::float_frame(p).inner_margin(egui::Margin::same(3)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if icon_button(ui, Icon::Plus, true, false, p).on_hover_text("Zoom in  ⌘=").clicked() {
                    app.queued.push(Action::ZoomIn);
                }
                if icon_button(ui, Icon::Minus, true, false, p).on_hover_text("Zoom out  ⌘−").clicked() {
                    app.queued.push(Action::ZoomOut);
                }
                if icon_button(ui, Icon::Home, true, false, p).on_hover_text("Show everything  ⇧1").clicked() {
                    app.queued.push(Action::FitAll);
                }
            });
        });

    // Bottom-right zoom %.
    let zoom = app.editor.view.camera.zoom;
    egui::Area::new(egui::Id::unique("zoompct")).fixed_pos(Pos2::new(rect.right() - 74.0, rect.bottom() - 42.0)).order(egui::Order::Middle).show(ctx, |ui| {
        let txt = if zoom >= 100.0 || zoom < 0.01 { format!("{:.0e}%", zoom * 100.0) } else { format!("{:.0}%", zoom * 100.0) };
        let r = ui.add(egui::Button::new(RichText::new(txt).size(12.0).color(p.weak)).fill(p.float).stroke(Stroke::new(1.0, p.border)).corner_radius(8).min_size(EVec2::new(58.0, 26.0)));
        if r.on_hover_text("Click: 100%   ⌘0").clicked() {
            app.queued.push(Action::ZoomReset);
        }
    });

    // Empty-document hint.
    if app.editor.doc.elements.is_empty() && app.canvas.text.is_none() {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("hint")));
        let painter = painter.with_clip_rect(rect);
        painter.text(rect.center() - EVec2::new(0.0, 14.0), Align2::CENTER_CENTER, "Double-click anywhere to write", style::bold(22.0), p.weak.gamma_multiply(0.85));
        painter.text(rect.center() + EVec2::new(0.0, 18.0), Align2::CENTER_CENTER, "F for frames  ·  drop images here  ·  File › New from Sample to see an example", FontId::proportional(14.0), p.weak.gamma_multiply(0.75));
    } else if app.editor.doc.bounds().is_some_and(|b| !b.intersects(&app.editor.view.camera.visible_bounds(app.editor.view.viewport))) {
        // "Back to content" pill when lost in empty space.
        egui::Area::new(egui::Id::unique("backtocontent")).anchor(Align2::CENTER_TOP, EVec2::new(rect.center().x - ctx.content_rect().center().x, rect.top() + 16.0)).show(ctx, |ui| {
            if ui.add(egui::Button::new("Back to content").fill(p.float).stroke(Stroke::new(1.0, p.border)).corner_radius(14)).clicked() {
                app.queued.push(Action::FitAll);
            }
        });
    }

    // Toast.
    if let Some(t) = app.toast.clone() {
        egui::Area::new(egui::Id::unique("toast")).anchor(Align2::CENTER_BOTTOM, EVec2::new(rect.center().x - ctx.content_rect().center().x, -(ctx.content_rect().bottom() - rect.bottom()) - 76.0)).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
            egui::Frame::new().fill(Color32::from_rgba_unmultiplied(30, 32, 36, 235)).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 7)).show(ui, |ui| {
                ui.label(RichText::new(t.text).color(Color32::WHITE).size(13.0));
            });
        });
    }

    // Autosave recovery banner.
    if let Some(rp) = app.recovered.clone() {
        egui::Area::new(egui::Id::unique("recover")).anchor(Align2::CENTER_TOP, EVec2::new(rect.center().x - ctx.content_rect().center().x, rect.top() + 12.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            style::float_frame(p).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Unsaved changes from an earlier session were found.");
                    if ui.button("Restore").clicked() {
                        if let Ok(doc) = zebra_core::io::load(&rp) {
                            let path = app.editor.file_path.clone();
                            app.editor.load(doc, path);
                            app.toast("Restored — save to keep it");
                        }
                        app.recovered = None;
                    }
                    if ui.button("Dismiss").clicked() {
                        app.recovered = None;
                    }
                });
            });
        });
    }
}

// ----------------------------------------------------------------------
// Properties panel

fn section(ui: &mut Ui, title: &str, p: &Palette) {
    ui.add_space(6.0);
    ui.label(RichText::new(title.to_uppercase()).size(10.5).color(p.weak).strong());
}

fn swatch(ui: &mut Ui, c: Color32, selected: bool, tip: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(EVec2::splat(22.0), Sense::click());
    let painter = ui.painter();
    if c == Color32::TRANSPARENT {
        painter.circle_stroke(r.center(), 8.5, Stroke::new(1.0, Color32::from_gray(160)));
        painter.line_segment([r.center() + EVec2::new(-6.0, 6.0), r.center() + EVec2::new(6.0, -6.0)], Stroke::new(1.2, Color32::from_rgb(220, 60, 60)));
    } else {
        painter.circle(r.center(), 8.5, c, Stroke::new(1.0, Color32::from_black_alpha(40)));
    }
    if selected {
        painter.circle_stroke(r.center(), 11.0, Stroke::new(2.0, ACCENT));
    }
    resp.on_hover_text(tip).clicked()
}

fn properties(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let sel = app.selection();
    let theme = app.editor.theme();
    let mut patch_all: Option<ElementPatch> = None;
    let mut gesture_active = false;

    if sel.len() > 1 {
        ui.heading(format!("{} objects", sel.len()));
        section(ui, "Arrange", p);
        ui.horizontal_wrapped(|ui| {
            for (l, edge) in [("Left", zebra_core::command::AlignEdge::Left), ("Centre", zebra_core::command::AlignEdge::CenterX), ("Right", zebra_core::command::AlignEdge::Right), ("Top", zebra_core::command::AlignEdge::Top), ("Middle", zebra_core::command::AlignEdge::CenterY), ("Bottom", zebra_core::command::AlignEdge::Bottom)] {
                if ui.button(l).on_hover_text(format!("Align {l}")).clicked() {
                    app.apply(Command::Align { ids: sel.clone(), edge });
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (l, layout) in [("Row", zebra_core::command::Layout::Row), ("Column", zebra_core::command::Layout::Column), ("Grid", zebra_core::command::Layout::Grid), ("Circle", zebra_core::command::Layout::Circle)] {
                if ui.button(l).on_hover_text(format!("Arrange in a {l}")).clicked() {
                    app.apply(Command::Arrange { ids: sel.clone(), layout, gap: None });
                }
            }
        });
        common_actions(app, ui, p);
        return;
    }
    let Some(e) = sel.first().and_then(|id| app.editor.doc.get(id)).cloned() else { return };
    let kind_name = match &e.kind {
        ElementKind::Frame(f) => format!("{} frame", frame_name(f.style)),
        ElementKind::Text(t) => format!("{:?} text", t.style),
        ElementKind::Shape(s) => format!("{:?}", s.shape),
        ElementKind::Image(_) => "Image".into(),
        ElementKind::Ink(_) => "Drawing".into(),
    };
    ui.horizontal(|ui| {
        ui.heading(kind_name);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(&e.id).size(11.0).color(p.weak)).on_hover_text("Element id (what an agent uses to refer to it)");
        });
    });

    // Name (frames: shown in the path sidebar)
    if e.is_frame() {
        let mut name = e.name.clone();
        ui.horizontal(|ui| {
            ui.label("Name");
            let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text(e.label(&app.editor.doc)).desired_width(170.0));
            if r.changed() {
                patch_all = Some(ElementPatch { name: Some(name.clone()), ..Default::default() });
            }
            gesture_active |= r.has_focus();
        });
    }

    section(ui, "Position", p);
    let mut x = e.transform.x;
    let mut y = e.transform.y;
    let mut scale = e.transform.scale;
    let mut rot = e.transform.rotation;
    egui::Grid::new("xform").num_columns(4).spacing([6.0, 6.0]).show(ui, |ui| {
        let sp = (1.0 / app.editor.view.camera.zoom).max(1e-6);
        ui.label("X");
        let r1 = ui.add(egui::DragValue::new(&mut x).speed(sp).max_decimals(1));
        ui.label("Y");
        let r2 = ui.add(egui::DragValue::new(&mut y).speed(sp).max_decimals(1));
        ui.end_row();
        ui.label("Scale");
        let r3 = ui.add(egui::DragValue::new(&mut scale).speed(e.transform.scale * 0.01).range(1e-6..=1e6).max_decimals(3));
        ui.label("Rot");
        let r4 = ui.add(egui::DragValue::new(&mut rot).speed(0.5).suffix("°").max_decimals(1));
        ui.end_row();
        for r in [&r1, &r2, &r3, &r4] {
            gesture_active |= r.dragged() || r.has_focus();
        }
        if r1.changed() || r2.changed() || r3.changed() || r4.changed() {
            patch_all = Some(ElementPatch { x: Some(x), y: Some(y), scale: Some(scale), rotation: Some(rot), ..Default::default() });
        }
    });

    match &e.kind {
        ElementKind::Text(t) => {
            section(ui, "Text", p);
            ui.horizontal(|ui| {
                for s in [TextStyle::Title, TextStyle::Subtitle, TextStyle::Body] {
                    if ui.selectable_label(t.style == s, format!("{s:?}")).on_hover_text(format!("{s:?} text style")).clicked() {
                        patch_all = Some(ElementPatch { style: Some(s), ..Default::default() });
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.selectable_label(t.bold, RichText::new(" B ").strong()).on_hover_text("Bold").clicked() {
                    patch_all = Some(ElementPatch { bold: Some(!t.bold), ..Default::default() });
                }
                if ui.selectable_label(t.italic, RichText::new(" I ").italics()).on_hover_text("Italic").clicked() {
                    patch_all = Some(ElementPatch { italic: Some(!t.italic), ..Default::default() });
                }
                if ui.selectable_label(t.bullets, "• List").on_hover_text("Bulleted list").clicked() {
                    patch_all = Some(ElementPatch { bullets: Some(!t.bullets), ..Default::default() });
                }
            });
            ui.horizontal(|ui| {
                ui.label("Align");
                for (a, l) in [(zebra_core::Align::Left, "⇤"), (zebra_core::Align::Center, "↔"), (zebra_core::Align::Right, "⇥")] {
                    if ui.selectable_label(t.align == a, l).on_hover_text(format!("Align {a:?}")).clicked() {
                        patch_all = Some(ElementPatch { align: Some(a), ..Default::default() });
                    }
                }
            });
            if ui.button("Edit text  ↵").on_hover_text("Edit the text in place  ↵ / double-click").clicked() {
                crate::canvas::begin_edit_existing(app, e.id.clone());
            }
            section(ui, "Colour", p);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let cur = t.color.clone().unwrap_or_default();
                if swatch(ui, style::c32(theme.color(&format!("theme:{}", format!("{:?}", t.style).to_lowercase()))), cur.is_empty(), "Theme default") {
                    patch_all = Some(ElementPatch { color: Some(String::new()), ..Default::default() });
                }
                for slot in ["theme:title", "theme:subtitle", "theme:body", "theme:accent", "theme:shape1", "theme:shape2", "theme:shape3", "theme:shape4"] {
                    if swatch(ui, style::c32(theme.color(&slot.to_string())), cur == slot, slot) {
                        patch_all = Some(ElementPatch { color: Some(slot.into()), ..Default::default() });
                    }
                }
            });
        }
        ElementKind::Shape(s) => {
            section(ui, "Shape", p);
            ui.horizontal_wrapped(|ui| {
                for k in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Triangle, ShapeKind::Line, ShapeKind::Arrow] {
                    if ui.selectable_label(s.shape == k, format!("{k:?}")).clicked() {
                        patch_all = Some(ElementPatch { shape: Some(k), ..Default::default() });
                    }
                }
            });
            for (label, cur, is_fill) in [("Fill", s.fill.clone(), true), ("Stroke", s.stroke.clone(), false)] {
                section(ui, label, p);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let cur = cur.unwrap_or_default();
                    if swatch(ui, Color32::TRANSPARENT, cur.is_empty(), "None") {
                        patch_all = Some(if is_fill { ElementPatch { fill: Some(String::new()), ..Default::default() } } else { ElementPatch { stroke: Some(String::new()), ..Default::default() } });
                    }
                    for slot in ["theme:shape1", "theme:shape2", "theme:shape3", "theme:shape4", "theme:accent", "theme:frame", "theme:title", "theme:background"] {
                        if swatch(ui, style::c32(theme.color(&slot.to_string())), cur == slot, slot) {
                            patch_all = Some(if is_fill { ElementPatch { fill: Some(slot.into()), ..Default::default() } } else { ElementPatch { stroke: Some(slot.into()), ..Default::default() } });
                        }
                    }
                });
            }
            let mut sw = s.stroke_width;
            ui.horizontal(|ui| {
                ui.label("Stroke width");
                let r = ui.add(egui::DragValue::new(&mut sw).speed(0.2).range(0.0..=200.0));
                gesture_active |= r.dragged() || r.has_focus();
                if r.changed() {
                    patch_all = Some(ElementPatch { stroke_width: Some(sw), ..Default::default() });
                }
            });
        }
        ElementKind::Frame(f) => {
            section(ui, "Frame style", p);
            ui.horizontal(|ui| {
                for s in FrameStyle::ALL {
                    let r = icon_button(ui, Icon::Frame(s), true, f.style == s, p).on_hover_text(frame_name(s));
                    if r.clicked() {
                        patch_all = Some(ElementPatch { frame_style: Some(s), ..Default::default() });
                    }
                }
            });
            section(ui, "Colour", p);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let cur = f.color.clone().unwrap_or_default();
                for (slot, tip) in [("", "Theme frame colour"), ("theme:accent", "Accent"), ("theme:shape1", "Colour 1"), ("theme:shape2", "Colour 2"), ("theme:shape3", "Colour 3"), ("theme:shape4", "Colour 4"), ("theme:title", "Title")] {
                    let c = style::c32(theme.color(&(if slot.is_empty() { "theme:frame" } else { slot }).to_string()));
                    if swatch(ui, c, cur == slot, tip) {
                        patch_all = Some(ElementPatch { color: Some(slot.into()), ..Default::default() });
                    }
                }
            });
            let n = app.editor.doc.frame_contents(&e.id).len();
            ui.label(RichText::new(format!("Contains {n} object{}", if n == 1 { "" } else { "s" })).size(12.0).color(p.weak));
            if ui.button("Zoom into frame").on_hover_text("Move the camera to fit this frame").clicked() {
                let _ = app.editor.view(zebra_core::ViewCommand::ZoomTo { id: e.id.clone(), animate: true }, H);
            }
        }
        ElementKind::Image(i) => {
            section(ui, "Image", p);
            let mut alt = i.alt.clone();
            ui.label("Description (alt text)");
            let r = ui.add(egui::TextEdit::singleline(&mut alt).desired_width(220.0));
            gesture_active |= r.has_focus();
            if r.changed() {
                patch_all = Some(ElementPatch { alt: Some(alt), ..Default::default() });
            }
        }
        ElementKind::Ink(_) => {}
    }

    // The step this element is the target of: notes and timing.
    if let Some(si) = app.editor.doc.path.iter().position(|s| matches!(&s.target, StepTarget::Element { id } if *id == e.id)) {
        crate::presenting::step_section(app, ui, p, si);
    }

    // Fade-in builds: classic Prezi "Animate" within a step.
    let steps: Vec<usize> = (0..app.editor.doc.path.len())
        .filter(|i| match &app.editor.doc.path[*i].target {
            StepTarget::Element { id } => *id != e.id && app.editor.doc.frame_contents(id).contains(&e.id),
            _ => false,
        })
        .collect();
    if let Some(&si) = steps.last() {
        section(ui, "Animation", p);
        let builds = app.editor.doc.path[si].builds.clone();
        let mut on = builds.contains(&e.id);
        if ui.checkbox(&mut on, format!("Fade in on step {}", si + 1)).on_hover_text("Appears with a click after the camera arrives").changed() {
            let mut b = builds;
            if on {
                b.push(e.id.clone());
            } else {
                b.retain(|x| *x != e.id);
            }
            app.apply(Command::UpdateStep { index: si, target: None, builds: Some(b), notes: None, duration: None, dwell: None });
        }
    }

    // Live edits from the panel are one undo step per interaction.
    if let Some(patch) = patch_all {
        if gesture_active && !app.editor.in_transaction() {
            let _ = app.editor.begin("Change properties", H);
        }
        let r = app.editor.apply(Command::Update { id: e.id.clone(), patch }, H);
        app.report(r);
    }
    if !gesture_active && app.editor.human_gesture_active() && matches!(app.canvas.gesture, crate::canvas::Gesture::None) {
        app.editor.commit();
    }

    common_actions(app, ui, p);
}

fn common_actions(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let sel = app.selection();
    let locked = !sel.is_empty() && sel.iter().all(|id| app.editor.doc.get(id).is_some_and(|e| e.locked));
    let in_path = sel.iter().any(|id| !app.editor.doc.steps_using(id).is_empty());
    section(ui, "Arrange", p);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Forward").on_hover_text("Bring forward one layer  ]").clicked() {
            app.queued.push(Action::BringForward);
        }
        if ui.button("Backward").on_hover_text("Send backward one layer  [").clicked() {
            app.queued.push(Action::SendBackward);
        }
        if ui.button("To front").on_hover_text("Bring to front  ⌘]").clicked() {
            app.queued.push(Action::BringToFront);
        }
        if ui.button("To back").on_hover_text("Send to back  ⌘[").clicked() {
            app.queued.push(Action::SendToBack);
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button(if locked { "Unlock" } else { "Lock" }).on_hover_text(if locked { "Unlock so it can be moved and edited  ⌘L" } else { "Lock in place  ⌘L" }).clicked() {
            app.queued.push(Action::ToggleLock);
        }
        if ui.button("Duplicate").on_hover_text("Duplicate the selection  ⌘D").clicked() {
            app.queued.push(Action::Duplicate);
        }
        if ui.button("Frame it").on_hover_text("Draw a frame around the selection  ⌘G").clicked() {
            app.queued.push(Action::WrapInFrame);
        }
    });
    section(ui, "Path", p);
    ui.horizontal_wrapped(|ui| {
        if ui.button("+ Add to path").on_hover_text("Add a path step that zooms to the selection  ⇧⌘A").clicked() {
            app.queued.push(Action::AddToPath);
        }
        if in_path {
            ui.label(RichText::new("✓ on the path").size(12.0).color(p.weak));
        }
    });
    ui.add_space(10.0);
    let del = egui::Button::new(RichText::new("Delete").color(Color32::from_rgb(200, 50, 50)));
    if ui.add_enabled(!locked, del).on_hover_text("Delete the selection  ⌫").on_disabled_hover_text("Unlock to delete  ⌘L").clicked() {
        app.queued.push(Action::Delete);
    }
}

// ----------------------------------------------------------------------
// Agent activity panel (⌘J)

fn activity_panel(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    ui.horizontal(|ui| {
        ui.heading("Agent activity");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Close").on_hover_text("Close the activity panel  ⌘J").clicked() {
                app.show_activity = false;
            }
        });
    });
    ui.label(RichText::new("Every change an agent makes appears here and on your undo stack.").size(12.0).color(p.weak));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let paused = app.editor.view.agent_paused;
        if ui.button(if paused { "Resume" } else { "Pause" }).on_hover_text(if paused { "Let the agent continue" } else { "Temporarily stop the agent from making changes" }).clicked() {
            app.queued.push(Action::PauseAgents);
        }
        let mut follow = app.editor.view.follow_agent;
        if ui.checkbox(&mut follow, "Follow").on_hover_text("Move your view to wherever the agent is working").changed() {
            app.queued.push(Action::FollowAgent);
        }
        if ui.button("Revert all").on_hover_text("Undo everything the agent did since your last change").clicked() {
            app.queued.push(Action::RevertAgent);
        }
    });
    ui.separator();
    let now = app.editor.view.now;
    let items: Vec<_> = app.editor.activity.iter().rev().filter(|a| a.origin.is_agent()).take(200).cloned().collect();
    if items.is_empty() {
        ui.label(RichText::new("No agent changes yet.").color(p.weak));
    }
    for a in items {
        let ago = now - a.time;
        let when = if ago < 60.0 { format!("{:.0}s ago", ago.max(0.0)) } else { format!("{:.0} min ago", ago / 60.0) };
        let r = egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)).corner_radius(6).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(EVec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 3.5, AGENT);
                ui.label(RichText::new(&a.label).size(13.0));
            });
            ui.label(RichText::new(format!("{} · {} · {} object(s)", a.origin.display(), when, a.ids.len())).size(11.0).color(p.weak));
        });
        let resp = r.response.interact(Sense::click()).on_hover_text("Click to show these objects");
        if resp.hovered() {
            ui.painter().rect_stroke(resp.rect, 6.0, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        }
        if resp.clicked() && !a.ids.is_empty() {
            let ids: Vec<_> = a.ids.iter().filter(|id| app.editor.doc.get(id).is_some()).cloned().collect();
            if !ids.is_empty() {
                let b = app.editor.bounds_of(&ids);
                let cam = zebra_core::Camera::fit_rect(&b, app.editor.view.viewport, 0.15);
                let _ = app.editor.view(zebra_core::ViewCommand::SetCamera { camera: cam, animate: true }, H);
            }
        }
    }
}

// ----------------------------------------------------------------------
// Dialogs: unsaved changes, ⌘K palette, shortcuts, about

fn dialogs(app: &mut ZebraApp, ctx: &egui::Context, p: &Palette) {
    if let Some(action) = app.confirm.clone() {
        let m = egui::Modal::new(egui::Id::unique("confirm")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Save changes?");
            ui.label(format!("“{}” has changes that aren't saved.", app.editor.doc.title));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Save").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                    if app.save(false) {
                        app.force_quit = true;
                        app.queued.push(action.clone());
                    }
                    app.confirm = None;
                }
                if ui.button("Don't Save").clicked() {
                    app.force_quit = true;
                    app.queued.push(action.clone());
                    app.confirm = None;
                }
                if ui.button("Cancel").clicked() {
                    app.confirm = None;
                }
            });
        });
        if m.should_close() {
            app.confirm = None;
        }
    }

    if let Some((mut q, mut sel)) = app.palette.take() {
        let mut keep = true;
        let all = Action::palette();
        let ql = q.to_lowercase();
        let matches: Vec<Action> = all.into_iter().filter(|a| ql.split_whitespace().all(|w| a.label().to_lowercase().contains(w))).collect();
        let m = egui::Modal::new(egui::Id::unique("palette")).show(ctx, |ui| {
            ui.set_width(440.0);
            let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Type a command…").desired_width(f32::INFINITY).font(FontId::proportional(16.0)));
            r.request_focus();
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                sel = (sel + 1).min(matches.len().saturating_sub(1));
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                sel = sel.saturating_sub(1);
            }
            ui.separator();
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (i, a) in matches.iter().enumerate() {
                    let resp = ui.horizontal(|ui| {
                        let r = ui.selectable_label(i == sel, a.label());
                        if let Some(s) = a.shortcut_text() {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(s).color(p.weak)));
                        }
                        r
                    });
                    if resp.inner.clicked() {
                        app.queued.push(a.clone());
                        keep = false;
                    }
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(a) = matches.get(sel) {
                    app.queued.push(a.clone());
                }
                keep = false;
            }
        });
        if m.should_close() {
            keep = false;
        }
        if keep {
            app.palette = Some((q, sel));
        }
    }

    if app.show_shortcuts {
        let m = egui::Modal::new(egui::Id::unique("shortcuts")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("Keyboard shortcuts");
            ui.label(RichText::new("You never need these — every one is also a button or menu item.").color(p.weak));
            ui.add_space(6.0);
            let rows: &[(&str, &str)] = &[
                ("Write text", "Double-click anywhere"),
                ("Tools", "V select · H hand · T text · F frame (again = style) · R O L A D I"),
                ("Pan / zoom", "Space-drag or two-finger scroll · pinch or wheel"),
                ("Fit all / selection / 100%", "⇧1 / ⇧2 / ⌘0"),
                ("Undo / redo", "⌘Z / ⇧⌘Z"),
                ("Duplicate / delete", "⌘D / ⌫"),
                ("Copy / paste", "⌘C / ⌘V"),
                ("Forward / backward", "] / ["),
                ("Rotate in 15° steps", "Hold ⇧ while rotating"),
                ("Edit path", "P"),
                ("Add selection to path", "⇧⌘A"),
                ("Present (start / current)", "F5 · ⌘↵ / ⇧F5 · ⌥⌘↵"),
                ("While presenting", "→ Space next · ← back · Home overview · B/W blank · Esc exit"),
                ("Command palette", "⌘K"),
                ("Agent activity / stop agent", "⌘J / ⌘."),
            ];
            egui::Grid::new("sc").num_columns(2).spacing([24.0, 6.0]).striped(true).show(ui, |ui| {
                for (a, k) in rows {
                    ui.label(*a);
                    ui.label(RichText::new(*k).color(p.weak));
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Scrolling:");
                for (m, l) in [(ScrollMode::Auto, "Auto (trackpad pans)"), (ScrollMode::Zoom, "Always zoom"), (ScrollMode::Pan, "Always pan")] {
                    if ui.radio(app.scroll_mode == m, l).clicked() {
                        app.scroll_mode = m;
                    }
                }
            });
        });
        if m.should_close() {
            app.show_shortcuts = false;
        }
    }

    if app.show_about {
        let m = egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(EVec2::splat(56.0), Sense::hover());
                crate::canvas::draw_zebra_disc(ui.painter(), r.center(), 24.0);
                ui.heading("Zebra");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label(RichText::new("A zooming canvas for presentations, in the spirit of 2012 Prezi.\nDrivable by AI agents through an opt-in local API.").color(p.weak));
            });
        });
        if m.should_close() {
            app.show_about = false;
        }
    }

    // macOS-style tool name feedback isn't needed; keep `tool_name` linked.
    let _ = tool_name;
}
