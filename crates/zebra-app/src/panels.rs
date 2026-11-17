//! Editor chrome, classic Mac layout (Keynote is the reference):
//! - a unified title bar + toolbar across the top (icon over label);
//! - the path sidebar on the left (opaque source list);
//! - the Format inspector on the right, persistent while shown (⌥⌘I);
//! - the canvas in the middle, with nothing floating over it except the
//!   empty-document hint, "Back to Content" and the HUD bezel.
//!
//! Nothing here requires knowing a command; every control queues an
//! [`Action`] or applies a core command, like the menus do.
use egui::{Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2 as EVec2};
use zebra_core::{FrameStyle, Mode, Origin, Theme, Tool};

use crate::actions::{frame_name, Action};
use crate::app::ZebraApp;
use crate::icons::Icon;
use crate::style::{self, accent, Palette, AGENT};
use crate::widgets::{self as w, ToolbarState};

pub use crate::icons::paint_icon;

const H: Origin = Origin::Human;

/// Height of the title row the native title bar draws into (macOS only:
/// the window uses a full-size content view with a transparent title bar,
/// so the traffic lights, title, edited dot and proxy icon are AppKit's).
pub const TITLE_ROW: f32 = if cfg!(target_os = "macos") { 28.0 } else { 0.0 };
/// Toolbar row under it: 44 pt items plus padding.
pub const TOOLBAR_ROW: f32 = 50.0;
/// Path sidebar width.
pub const SIDEBAR_W: f32 = 220.0;

pub fn layout(app: &mut ZebraApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let p = Palette::of(&ctx);

    #[cfg(not(target_os = "macos"))]
    egui::Panel::top("menubar")
        .frame(egui::Frame::new().fill(p.toolbar).inner_margin(egui::Margin::symmetric(8, 2)))
        .show(ui, |ui| crate::menus::egui_menu_bar(app, ui));

    egui::Panel::top("toolbar")
        .exact_size(TITLE_ROW + TOOLBAR_ROW)
        .frame(egui::Frame::new().fill(p.toolbar))
        .show(ui, |ui| toolbar(app, ui, &p));

    if app.show_sidebar {
        egui::Panel::left("path")
            .exact_size(SIDEBAR_W)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin { left: 10, right: 10, top: 8, bottom: 0 }).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| crate::sidebar::ui(app, ui, &p));
    }

    if app.show_props {
        egui::Panel::right("inspector")
            .exact_size(crate::inspector::WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin { left: 16, right: 12, top: 0, bottom: 0 }).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| crate::inspector::ui(app, ui));
    }
    if app.show_activity {
        egui::Panel::right("activity")
            .exact_size(260.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(14, 10)).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| activity_panel(app, ui, &p));
            });
    }

    egui::CentralPanel::no_frame().show(ui, |ui| {
        crate::canvas::edit_canvas(app, ui);
    });
    let rect = app.canvas.rect;
    floating(app, &ctx, rect, &p);
    crate::sheets::ui(app, &ctx);
    crate::shape_library::ui(app, &ctx, &p);
    dialogs(app, &ctx, &p);
}

// ----------------------------------------------------------------------
// Toolbar (Keynote order: View · Zoom · Add Step | Play | Text · Frame ·
// Shape · Line · Media | Theme · Export · AI · Format)

fn toolbar(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let full = ui.max_rect();
    // Bottom hairline (the toolbar's separator from content).
    ui.painter().line_segment([full.left_bottom() - EVec2::new(0.0, 0.5), full.right_bottom() - EVec2::new(0.0, 0.5)], Stroke::new(1.0, p.border));
    // Empty toolbar space drags the window, as in AppKit.
    let bg = ui.interact(full, egui::Id::unique("toolbar-bg"), Sense::click_and_drag());
    if bg.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if bg.double_clicked() && cfg!(target_os = "macos") {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
    }

    let row = Rect::from_min_max(Pos2::new(full.left(), full.top() + TITLE_ROW), Pos2::new(full.right(), full.top() + TITLE_ROW + TOOLBAR_ROW - 3.0));
    let inset = if cfg!(target_os = "macos") { 12.0 } else { 10.0 };

    // Left group
    let mut left = ui.new_child(egui::UiBuilder::new().max_rect(row.shrink2(EVec2::new(inset, 0.0))).layout(Layout::left_to_right(Align::Center)));
    left.spacing_mut().item_spacing.x = 2.0;
    view_item(app, &mut left);
    zoom_item(app, &mut left, p);
    let add_tip = if app.editor.view.selection.is_empty() { "Add the current view as a step  ⇧⌘A" } else { "Add the selection as a step  ⇧⌘A" };
    if w::toolbar_item(&mut left, Icon::AddStep, "Add Step", ToolbarState { enabled: !app.editor.doc.elements.is_empty(), ..st() }).on_hover_text(add_tip).clicked() {
        app.queued.push(Action::AddToPath);
    }
    let left_end = left.min_rect().right();

    // Right group (right to left)
    let mut right = ui.new_child(egui::UiBuilder::new().max_rect(row.shrink2(EVec2::new(inset, 0.0))).layout(Layout::right_to_left(Align::Center)));
    right.spacing_mut().item_spacing.x = 2.0;
    if w::toolbar_item(&mut right, Icon::Format, "Format", ToolbarState { active: app.show_props, ..st() }).on_hover_text(if app.show_props { "Hide the inspector  ⌥⌘I" } else { "Show the inspector  ⌥⌘I" }).clicked() {
        app.queued.push(Action::ToggleProperties);
    }
    ai_item(app, &mut right, p);
    let ex = w::toolbar_item(&mut right, Icon::Share, "Export", ToolbarState { menu: true, enabled: !app.editor.doc.elements.is_empty(), ..st() }).on_hover_text("Export as PDF or PNG");
    egui::Popup::menu(&ex).width(220.0).show(|ui| {
        if w::menu_item(ui, "PDF…", Action::ExportPdf.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ExportPdf);
        }
        if w::menu_item(ui, "PNG of Current View…", None).clicked() {
            app.queued.push(Action::ExportPng);
        }
    });
    let th = w::toolbar_item(&mut right, Icon::Theme, "Theme", ToolbarState { menu: true, ..st() }).on_hover_text("Change the presentation's colours and fonts");
    egui::Popup::menu(&th).width(230.0).show(|ui| theme_menu(app, ui));
    let right_start = right.min_rect().left();

    // Centre group, centred in the window when it fits between the others.
    let cw_id = egui::Id::unique("toolbar-centre-w");
    let cw: f32 = ui.data(|d| d.get_temp(cw_id)).unwrap_or(380.0);
    let mut cx = full.center().x - cw / 2.0;
    cx = cx.max(left_end + 16.0).min(right_start - 16.0 - cw);
    let crect = Rect::from_min_max(Pos2::new(cx, row.top()), Pos2::new(cx + cw + 40.0, row.bottom()));
    let mut centre = ui.new_child(egui::UiBuilder::new().max_rect(crect).layout(Layout::left_to_right(Align::Center)));
    centre.spacing_mut().item_spacing.x = 2.0;
    centre_group(app, &mut centre, p);
    let used = centre.min_rect().width();
    if (used - cw).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(cw_id, used));
        ui.ctx().request_repaint();
    }
}

fn st() -> ToolbarState {
    ToolbarState { enabled: true, ..Default::default() }
}

fn view_item(app: &mut ZebraApp, ui: &mut Ui) {
    let r = w::toolbar_item(ui, Icon::SidebarLeft, "View", ToolbarState { menu: true, ..st() }).on_hover_text("Show or hide the path sidebar and inspector");
    egui::Popup::menu(&r).width(230.0).show(|ui| {
        if w::menu_check(ui, app.show_sidebar, "Path Sidebar", Action::ToggleSidebar.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ToggleSidebar);
        }
        if w::menu_check(ui, app.show_props, "Inspector", Action::ToggleProperties.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ToggleProperties);
        }
        if w::menu_check(ui, app.show_activity, "Agent Activity", Action::ToggleActivity.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ToggleActivity);
        }
        w::menu_separator(ui);
        if w::menu_check(ui, app.editor.view.mode == Mode::PathEdit, "Edit Path", Some("P")).clicked() {
            app.queued.push(Action::TogglePathEdit);
        }
    });
}

/// Zoom: a small pop-up showing the zoom level, labelled "Zoom".
fn zoom_item(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let zoom = app.editor.view.camera.zoom;
    let txt = if zoom >= 100.0 || zoom < 0.01 { format!("{:.0e}%", zoom * 100.0) } else { format!("{:.0}%", zoom * 100.0) };
    let (rect, resp) = ui.allocate_exact_size(EVec2::new(64.0, 44.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, format!("Zoom {txt}")));
    let bez = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 14.0), EVec2::new(58.0, 21.0));
    w::paint_bezel(ui, bez, resp.is_pointer_button_down_on(), true);
    let painter = ui.painter();
    painter.text(Pos2::new(bez.left() + 8.0, bez.center().y), Align2::LEFT_CENTER, &txt, style::ui_font(12.0), p.text);
    paint_icon(painter, Rect::from_center_size(Pos2::new(bez.right() - 9.0, bez.center().y), EVec2::splat(10.0)), Icon::UpDown, p.weak);
    let galley = painter.layout_no_wrap("Zoom".into(), style::ui_font(11.0), p.text.gamma_multiply(0.9));
    painter.galley(Pos2::new(rect.center().x - galley.size().x / 2.0, rect.bottom() - galley.size().y - 1.0), galley, p.text);
    let resp = resp.on_hover_text("Zoom");
    egui::Popup::menu(&resp).width(200.0).show(|ui| {
        if w::menu_item(ui, "Zoom to Fit", Some("⇧1")).clicked() {
            app.queued.push(Action::FitAll);
        }
        let has_sel = !app.editor.view.selection.is_empty();
        if w::menu_row(ui, None, "Zoom to Selection", Some("⇧2"), has_sel).clicked() {
            app.queued.push(Action::ZoomToSelection);
        }
        w::menu_separator(ui);
        for (z, l) in [(0.5, "50%"), (1.0, "100%"), (2.0, "200%")] {
            if w::menu_check(ui, (zoom - z).abs() < 0.005, l, if z == 1.0 { Some("⌘0") } else { None }).clicked() {
                let mut c = app.editor.view.target_camera();
                c.zoom = z;
                app.view_cmd(zebra_core::ViewCommand::SetCamera { camera: c, animate: true });
            }
        }
        w::menu_separator(ui);
        if w::menu_item(ui, "Zoom In", Some("⌘=")).clicked() {
            app.queued.push(Action::ZoomIn);
        }
        if w::menu_item(ui, "Zoom Out", Some("⌘−")).clicked() {
            app.queued.push(Action::ZoomOut);
        }
    });
}

fn centre_group(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let tool = app.editor.view.tool;
    let has = !app.editor.doc.elements.is_empty();
    let play = w::toolbar_item(ui, Icon::Play, "Play", ToolbarState { enabled: has, tint: Some(accent()), ..st() })
        .on_hover_text("Play from the start  ⌥⌘P\nRight-click to play from the current step");
    if play.clicked() {
        app.queued.push(Action::PresentStart);
    }
    egui::Popup::context_menu(&play).show(|ui| {
        if w::menu_item(ui, "Play from Start", Action::PresentStart.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::PresentStart);
        }
        if w::menu_item(ui, "Play from Current Step", Action::PresentCurrent.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::PresentCurrent);
        }
        w::menu_separator(ui);
        if w::menu_check(ui, app.stage.presenter_view, "Presenter View", None).clicked() {
            app.queued.push(Action::TogglePresenterView);
        }
        if w::menu_item(ui, "Playback Options…", None).clicked() {
            app.queued.push(Action::PlaybackOptions);
        }
    });
    w::toolbar_separator(ui);

    if w::toolbar_item(ui, Icon::Text, "Text", ToolbarState { active: tool == Tool::Text, ..st() }).on_hover_text("Text  T\nOr double-click anywhere on the canvas").clicked() {
        app.queued.push(Action::SetTool(Tool::Text));
    }
    let fs = app.editor.view.frame_style;
    let fr = w::toolbar_item(ui, Icon::Frame(fs), "Frame", ToolbarState { active: tool == Tool::Frame, menu: true, ..st() }).on_hover_text("Frame  F\nPress F again to change the style");
    egui::Popup::menu(&fr).width(190.0).show(|ui| {
        for s in FrameStyle::ALL {
            if menu_icon_row(ui, Icon::Frame(s), frame_name(s), fs == s && tool == Tool::Frame).clicked() {
                app.queued.push(Action::SetFrameStyle(s));
            }
        }
        w::menu_separator(ui);
        let has_sel = !app.editor.view.selection.is_empty();
        if w::menu_row(ui, None, "Frame Selection", Action::WrapInFrame.shortcut_text().as_deref(), has_sel).clicked() {
            app.queued.push(Action::WrapInFrame);
        }
    });
    let shape_active = matches!(tool, Tool::Rectangle | Tool::Ellipse | Tool::Triangle);
    let sh = w::toolbar_item(ui, Icon::Shapes, "Shape", ToolbarState { active: shape_active, menu: true, ..st() }).on_hover_text("Shapes and icons");
    egui::Popup::menu(&sh).width(200.0).show(|ui| {
        for (i, t, l) in [(Icon::Rect, Tool::Rectangle, "Rectangle"), (Icon::Ellipse, Tool::Ellipse, "Ellipse")] {
            if menu_icon_row(ui, i, l, tool == t).clicked() {
                app.queued.push(Action::SetTool(t));
            }
        }
        w::menu_separator(ui);
        if w::menu_item(ui, "Shapes and Icons…", None).clicked() {
            app.queued.push(Action::ShapeLibrary);
        }
    });
    let line_active = matches!(tool, Tool::Line | Tool::Arrow | Tool::Pen | Tool::Highlighter);
    let line_icon = match tool {
        Tool::Line => Icon::Line,
        Tool::Pen | Tool::Highlighter => Icon::Pen,
        _ => Icon::Arrow,
    };
    let ln = w::toolbar_item(ui, line_icon, "Line", ToolbarState { active: line_active, menu: true, ..st() }).on_hover_text("Lines, arrows and drawing");
    egui::Popup::menu(&ln).width(190.0).show(|ui| {
        for (i, t, l) in [(Icon::Line, Tool::Line, "Line"), (Icon::Arrow, Tool::Arrow, "Arrow"), (Icon::Pen, Tool::Pen, "Draw")] {
            if menu_icon_row(ui, i, l, tool == t).clicked() {
                app.queued.push(Action::SetTool(t));
            }
        }
    });
    let md = w::toolbar_item(ui, Icon::Media, "Media", ToolbarState { menu: true, ..st() }).on_hover_text("Images, video and audio\nOr drop files on the canvas");
    egui::Popup::menu(&md).width(200.0).show(|ui| {
        if menu_icon_row(ui, Icon::Image, "Image…", false).clicked() {
            app.queued.push(Action::InsertImage);
        }
        if menu_icon_row(ui, Icon::Video, "Video…", false).clicked() {
            app.queued.push(Action::InsertVideo);
        }
        if menu_icon_row(ui, Icon::Note, "Audio…", false).clicked() {
            app.queued.push(Action::InsertAudio);
        }
    });
    let _ = p;
}

/// A menu row with a leading icon and a trailing ✓ when on.
fn menu_icon_row(ui: &mut Ui, icon: Icon, label: &str, on: bool) -> egui::Response {
    let p = Palette::of(ui.ctx());
    let wdt = ui.available_width().max(150.0);
    let (rect, resp) = ui.allocate_exact_size(EVec2::new(wdt, 24.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
    let hot = resp.hovered();
    let painter = ui.painter();
    if hot {
        painter.rect_filled(rect, 4.0, accent());
    }
    let c = if hot { w::on_accent() } else { p.text };
    paint_icon(painter, Rect::from_center_size(Pos2::new(rect.left() + 16.0, rect.center().y), EVec2::splat(16.0)), icon, c);
    painter.text(Pos2::new(rect.left() + 32.0, rect.center().y), Align2::LEFT_CENTER, label, style::ui_font(13.0), c);
    if on {
        paint_icon(painter, Rect::from_center_size(Pos2::new(rect.right() - 12.0, rect.center().y), EVec2::splat(11.0)), Icon::Check, c);
    }
    if resp.clicked() {
        ui.close();
    }
    resp
}

fn theme_menu(app: &mut ZebraApp, ui: &mut Ui) {
    let p = Palette::of(ui.ctx());
    let all: Vec<Theme> = Theme::builtins().into_iter().chain(app.editor.doc.themes.iter().cloned()).collect();
    for t in all {
        let selected = t.id == app.editor.doc.theme;
        let (r, resp) = ui.allocate_exact_size(EVec2::new(ui.available_width().max(214.0), 40.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &t.name));
        let painter = ui.painter();
        let hot = resp.hovered();
        if hot {
            painter.rect_filled(r, 4.0, accent());
        }
        let sw = Rect::from_min_size(r.min + EVec2::new(22.0, 5.0), EVec2::new(46.0, 30.0));
        painter.rect(sw, 3.0, style::c32(t.color(&"theme:background".into())), Stroke::new(0.6, Color32::from_black_alpha(50)), egui::StrokeKind::Inside);
        let fc = style::c32(t.color(&"theme:frame".into()));
        let stv = Stroke::new(1.5, fc);
        painter.add(egui::Shape::line(vec![sw.left_top() + EVec2::new(10.0, 5.0), sw.left_top() + EVec2::new(6.0, 5.0), sw.left_bottom() + EVec2::new(6.0, -5.0), sw.left_bottom() + EVec2::new(10.0, -5.0)], stv));
        painter.add(egui::Shape::line(vec![sw.right_top() + EVec2::new(-10.0, 5.0), sw.right_top() + EVec2::new(-6.0, 5.0), sw.right_bottom() + EVec2::new(-6.0, -5.0), sw.right_bottom() + EVec2::new(-10.0, -5.0)], stv));
        painter.line_segment([sw.center() + EVec2::new(-12.0, -3.0), sw.center() + EVec2::new(12.0, -3.0)], Stroke::new(3.0, style::c32(t.color(&"theme:title".into()))));
        painter.line_segment([sw.center() + EVec2::new(-9.0, 4.0), sw.center() + EVec2::new(9.0, 4.0)], Stroke::new(1.5, style::c32(t.color(&"theme:subtitle".into()))));
        let tc = if hot { w::on_accent() } else { p.text };
        if selected {
            paint_icon(painter, Rect::from_center_size(Pos2::new(r.left() + 11.0, r.center().y), EVec2::splat(11.0)), Icon::Check, tc);
        }
        painter.text(r.min + EVec2::new(78.0, 20.0), Align2::LEFT_CENTER, &t.name, style::ui_font(13.0), tc);
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

/// The AI toolbar item. Off: a plain "AI" item. On: a violet glyph, with
/// the agent's name as the label while one is connected.
fn ai_item(app: &mut ZebraApp, ui: &mut Ui, _p: &Palette) {
    let on = app.api.is_some() || app.demo_agent.is_some();
    let connected = app.agent_connected();
    let paused = app.editor.view.agent_paused;
    let label = if app.api_error.is_some() {
        "AI: Error".to_string()
    } else if connected {
        crate::files::short(&app.agent_name(), 12)
    } else {
        "AI".to_string()
    };
    let tint = if app.api_error.is_some() {
        Some(Color32::from_rgb(0xff, 0x3b, 0x30))
    } else if connected && !paused {
        Some(AGENT)
    } else if connected && paused {
        Some(Color32::from_rgb(0xff, 0x9f, 0x0a))
    } else {
        None
    };
    let tip = if !on {
        "Let an AI agent work in Zebra"
    } else if !connected {
        "AI control is on, waiting for an agent"
    } else if paused {
        "Agent paused. Click for agent controls"
    } else {
        "Agent connected. Click for agent controls"
    };
    let r = w::toolbar_item(ui, Icon::Sparkles, &label, ToolbarState { tint, active: on && !connected, ..st() });
    if connected && !paused {
        // a small live dot on the glyph, like a status item
        let busy = app.editor.activity.back().is_some_and(|a| a.origin.is_agent() && app.editor.view.now - a.time < 2.0);
        let c = Pos2::new(r.rect.center().x + 10.0, r.rect.top() + 6.0);
        ui.painter().circle(c, 3.5, if busy { AGENT } else { Color32::from_rgb(0x34, 0xc7, 0x59) }, Stroke::new(1.0, Palette::of(ui.ctx()).toolbar));
    }
    let r = r.on_hover_text(tip);
    let mut popup = egui::Popup::menu(&r);
    if std::mem::take(&mut app.open_ai_panel) {
        popup = popup.open_memory(egui::SetOpenCommand::Bool(true));
    }
    popup.width(330.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| ai_panel(app, ui, on, connected, paused));
    });
}

/// The AI popover: the human-only switch, how to connect an agent, and
/// (when on) the agent controls.
fn ai_panel(app: &mut ZebraApp, ui: &mut Ui, on: bool, connected: bool, paused: bool) {
    let p = Palette::of(ui.ctx());
    let mut want = on;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Let AI agents control Zebra").font(style::semibold(13.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if w::switch(ui, &mut want, "Let AI agents control Zebra").on_hover_text("Only you can switch this on. An agent can switch it off.").changed() {
                app.queued.push(Action::ToggleAiControl);
            }
        });
    });
    w::caption(
        ui,
        if on {
            "Agents on this Mac that can run a command (Claude Code, Codex, Cursor, Gemini CLI, your own scripts) can now see and edit this window. You can watch, undo, pause or stop them at any time."
        } else {
            "Off: nothing outside Zebra can see or change your work. Switch it on to let an AI agent on this Mac build or edit presentations with you."
        },
    );
    if let Some(e) = &app.api_error {
        ui.label(RichText::new(e).font(style::ui_font(11.0)).color(Color32::from_rgb(0xff, 0x3b, 0x30)));
    }
    if !on {
        return;
    }

    w::separator(ui);
    ui.label(RichText::new("Connect an Agent").font(style::semibold(13.0)));
    w::caption(ui, "Paste this into your agent's chat:");
    let ctl = crate::cli::zebractl_command();
    let prompt = crate::cli::agent_prompt(&ctl);
    copy_box(ui, &prompt, "agent-prompt");
    ui.horizontal_wrapped(|ui| {
        if let Some(installed) = crate::cli::install_state() {
            if installed {
                w::caption(ui, "✓ zebractl is installed on this Mac");
            } else if w::push_button(ui, "Install Command-Line Tool…").on_hover_text("Adds `zebractl` to /usr/local/bin so agents can type just `zebractl`. macOS asks for your password.").clicked() {
                match crate::cli::install() {
                    Ok(()) => app.toast("Installed zebractl. Agents can now run `zebractl --help`."),
                    Err(e) => app.toast(format!("Couldn't install zebractl: {e}")),
                }
            }
        }
        let mcp = w::push_button(ui, "MCP Setup…");
        egui::Popup::menu(&mcp).width(330.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            egui::Frame::new().inner_margin(8).show(ui, |ui| {
                w::caption(ui, "Optional. For apps that take an MCP server config (Claude Desktop, Cursor, and others):");
                copy_box(ui, &crate::cli::mcp_config(&ctl), "mcp-json");
            });
        });
    });

    w::separator(ui);
    if !connected {
        w::caption(ui, "No agent connected yet.");
    } else {
        ui.horizontal(|ui| {
            if w::push_button(ui, if paused { "Resume" } else { "Pause" }).on_hover_text("Temporarily stop the agent from making changes").clicked() {
                app.queued.push(Action::PauseAgents);
            }
            if w::push_button(ui, "Revert Changes…").on_hover_text("Undo everything the agent did since your last change").clicked() {
                app.queued.push(Action::RevertAgent);
            }
        });
        let mut follow = app.editor.view.follow_agent;
        if w::checkbox(ui, &mut follow, "Follow the agent's camera").on_hover_text("Move your view to wherever the agent is working").changed() {
            app.queued.push(Action::FollowAgent);
        }
    }
    if w::push_button(ui, "Show Activity").on_hover_text("⌘J").clicked() {
        app.queued.push(Action::ToggleActivity);
    }
    ui.add_space(2.0);
    ui.label(RichText::new("What agents may do").font(style::semibold(11.0)).color(p.weak));
    let cur = app.editor.view.agent_permission;
    for (perm, label) in [
        (zebra_core::view::Permission::Observe, "Look only"),
        (zebra_core::view::Permission::Edit, "Edit the presentation (default)"),
        (zebra_core::view::Permission::Full, "Full control (also your view, files, presenting)"),
    ] {
        if w::radio(ui, cur == perm, label).clicked() && cur != perm {
            app.queued.push(Action::SetAgentPermission(perm));
        }
    }
}

/// A read-only text block with a Copy button.
fn copy_box(ui: &mut Ui, text: &str, id: &str) {
    let p = Palette::of(ui.ctx());
    egui::Frame::new().fill(p.field).stroke(Stroke::new(0.6, p.control_edge)).corner_radius(4.0).inner_margin(8.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(text).monospace().size(11.0)).wrap());
    });
    let copied_id = ui.scope_id().with(("copied", id));
    let copied_at: Option<f64> = ui.data(|d| d.get_temp(copied_id));
    let now = ui.input(|i| i.time);
    let copied = copied_at.is_some_and(|t| now - t < 1.5);
    if w::push_button_ext(ui, if copied { "Copied" } else { "Copy" }, w::Size::Small, true, None).clicked() {
        ui.ctx().copy_text(text.to_string());
        ui.data_mut(|d| d.insert_temp(copied_id, now));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(1600));
    }
}

// ----------------------------------------------------------------------
// Over the canvas: empty-document hint, Back to Content, HUD, recovery

fn floating(app: &mut ZebraApp, ctx: &egui::Context, rect: Rect, p: &Palette) {
    // Empty-document hint (full-strength secondary text, 13 pt minimum).
    if app.editor.doc.elements.is_empty() && app.canvas.text.is_none() {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("hint"))).with_clip_rect(rect);
        let bg = style::c32(app.editor.theme().color(&"theme:background".into()));
        let dark_bg = (bg.r() as u32 + bg.g() as u32 + bg.b() as u32) < 380;
        let (c1, c2) = if dark_bg { (Color32::from_gray(0xd0), Color32::from_gray(0xa8)) } else { (Color32::from_gray(0x55), Color32::from_gray(0x6b)) };
        painter.text(rect.center() - EVec2::new(0.0, 14.0), Align2::CENTER_CENTER, "Double-click anywhere to write", style::semibold(20.0), c1);
        painter.text(rect.center() + EVec2::new(0.0, 16.0), Align2::CENTER_CENTER, "Press F for a frame, drop images here, or choose File › New from Sample", FontId::proportional(13.0), c2);
    } else if app.editor.doc.bounds().is_some_and(|b| !b.intersects(&app.editor.view.camera.visible_bounds(app.editor.view.viewport))) {
        egui::Area::new(egui::Id::unique("backtocontent"))
            .anchor(Align2::CENTER_TOP, EVec2::new(rect.center().x - ctx.content_rect().center().x, rect.top() + 14.0))
            .show(ctx, |ui| {
                if w::push_button(ui, "Back to Content").on_hover_text("Zoom to fit everything  ⇧1").clicked() {
                    app.queued.push(Action::FitAll);
                }
            });
    }

    // HUD bezel for the few messages that can't be shown in place.
    if let Some(t) = app.toast.clone() {
        let left = t.until - app.now();
        let a = (left / 0.3).clamp(0.0, 1.0) as f32;
        w::hud(ctx, rect, &t.text, a);
    }

    // Autosave recovery: an opaque info bar under the toolbar.
    if let Some(rp) = app.recovered.clone() {
        egui::Area::new(egui::Id::unique("recover"))
            .anchor(Align2::CENTER_TOP, EVec2::new(rect.center().x - ctx.content_rect().center().x, rect.top() + 12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                style::float_frame(p).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        paint_icon(ui.painter(), Rect::from_min_size(ui.cursor().min + EVec2::new(0.0, 2.0), EVec2::splat(18.0)), Icon::Warning, Color32::from_rgb(0xff, 0x9f, 0x0a));
                        ui.add_space(24.0);
                        ui.label("Unsaved changes from an earlier session were found.");
                        if w::push_button(ui, "Dismiss").clicked() {
                            app.recovered = None;
                        }
                        if w::default_button(ui, "Restore").clicked() {
                            if let Ok(doc) = zebra_core::io::load(&rp) {
                                let path = app.editor.file_path.clone();
                                app.editor.load(doc, path);
                                // Restored work is unsaved: mark the document edited.
                                app.editor.revision += 1;
                            }
                            app.recovered = None;
                        }
                    });
                });
            });
    }
}

// ----------------------------------------------------------------------
// Agent activity (⌘J): its own pane, beside the inspector.

fn activity_panel(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Agent Activity").font(style::semibold(13.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if w::icon_button(ui, Icon::Close, 20.0, true, false).on_hover_text("Close  ⌘J").clicked() {
                app.show_activity = false;
            }
        });
    });
    w::caption(ui, "Every change an agent makes appears here and on your undo stack.");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let paused = app.editor.view.agent_paused;
        if w::push_button(ui, if paused { "Resume" } else { "Pause" }).on_hover_text(if paused { "Let the agent continue" } else { "Temporarily stop the agent from making changes" }).clicked() {
            app.queued.push(Action::PauseAgents);
        }
        if w::push_button(ui, "Revert…").on_hover_text("Undo everything the agent did since your last change").clicked() {
            app.queued.push(Action::RevertAgent);
        }
    });
    let mut follow = app.editor.view.follow_agent;
    if w::checkbox(ui, &mut follow, "Follow the agent").on_hover_text("Move your view to wherever the agent is working").changed() {
        app.queued.push(Action::FollowAgent);
    }
    ui.add_space(4.0);
    w::separator(ui);
    let now = app.editor.view.now;
    let items: Vec<_> = app.editor.activity.iter().rev().filter(|a| a.origin.is_agent()).take(200).cloned().collect();
    if items.is_empty() {
        ui.add_space(6.0);
        w::caption(ui, "No agent changes yet.");
    }
    for a in items {
        let ago = now - a.time;
        let when = if ago < 60.0 { format!("{:.0} s ago", ago.max(0.0)) } else { format!("{:.0} min ago", ago / 60.0) };
        let r = egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)).corner_radius(5).show(ui, |ui| {
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
            ui.painter().rect_filled(resp.rect, 5.0, p.hover);
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
// Dialogs: ⌘K palette (optional), keyboard shortcuts, about

fn dialogs(app: &mut ZebraApp, ctx: &egui::Context, p: &Palette) {
    if app.show_icon_sheet {
        egui::Area::new(egui::Id::unique("icon-sheet")).order(egui::Order::Foreground).fixed_pos(Pos2::new(240.0, 120.0)).show(ctx, |ui| {
            style::float_frame(p).inner_margin(12).show(ui, |ui| crate::icons::debug_sheet(ui));
        });
    }
    if let Some((mut q, mut sel)) = app.palette.take() {
        let mut keep = true;
        let all = Action::palette();
        let ql = q.to_lowercase();
        let matches: Vec<Action> = all.into_iter().filter(|a| ql.split_whitespace().all(|w| a.label().to_lowercase().contains(w))).collect();
        let m = egui::Modal::new(egui::Id::unique("palette")).backdrop_color(Color32::TRANSPARENT).show(ctx, |ui| {
            ui.set_width(440.0);
            let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Type a command").desired_width(f32::INFINITY).font(FontId::proportional(15.0)));
            r.request_focus();
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                sel = (sel + 1).min(matches.len().saturating_sub(1));
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                sel = sel.saturating_sub(1);
            }
            w::separator(ui);
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, a) in matches.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(EVec2::new(ui.available_width(), 22.0), Sense::click());
                    let hot = i == sel || resp.hovered();
                    if hot {
                        ui.painter().rect_filled(rect, 4.0, accent());
                    }
                    let c = if hot { w::on_accent() } else { p.text };
                    ui.painter().text(rect.left_center() + EVec2::new(8.0, 0.0), Align2::LEFT_CENTER, a.label(), style::ui_font(13.0), c);
                    if let Some(s) = a.shortcut_text() {
                        ui.painter().text(rect.right_center() - EVec2::new(8.0, 0.0), Align2::RIGHT_CENTER, s, style::ui_font(13.0), if hot { c } else { p.weak });
                    }
                    if resp.clicked() {
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
            ui.set_width(540.0);
            ui.label(RichText::new("Keyboard Shortcuts").font(style::semibold(15.0)));
            w::caption(ui, "Every shortcut is also a menu item or a button.");
            ui.add_space(6.0);
            let rows: &[(&str, &str)] = &[
                ("Write text", "Double-click anywhere"),
                ("Select tool · Hand tool", "V · H (or hold Space)"),
                ("Text · Frame (again to change style)", "T · F"),
                ("Rectangle · Ellipse · Line · Arrow · Draw", "R · O · L · A · D"),
                ("Insert image", "I"),
                ("Pan · Zoom", "Two-finger scroll · Pinch, or ⌘-scroll"),
                ("Zoom to Fit · Zoom to Selection · Actual Size", "⇧1 · ⇧2 · ⌘0"),
                ("Undo · Redo", "⌘Z · ⇧⌘Z"),
                ("Duplicate · Delete", "⌘D · ⌫"),
                ("Bring Forward · Send Backward", "] · ["),
                ("Rotate in 15° steps", "Hold ⇧ while rotating"),
                ("Edit Path · Add to Path", "P · ⇧⌘A"),
                ("Play · Play from Current Step", "⌥⌘P · ⇧⌥⌘P"),
                ("While playing", "→ or Space next · ← back · Home overview · B or W blank · Esc stop"),
                ("Show Inspector · Show Sidebar", "⌥⌘I · ⌃⌘S"),
                ("Agent Activity · Stop Agent", "⌘J · ⌘."),
                ("Settings", "⌘,"),
            ];
            egui::Grid::new("sc").num_columns(2).spacing([24.0, 6.0]).striped(true).show(ui, |ui| {
                for (a, k) in rows {
                    ui.label(*a);
                    ui.label(RichText::new(*k).color(p.weak));
                    ui.end_row();
                }
            });
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::default_button(ui, "Done").clicked() {
                    app.show_shortcuts = false;
                }
            });
        });
        if m.should_close() {
            app.show_shortcuts = false;
        }
    }

    if app.show_about {
        let m = egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.vertical_centered(|ui| {
                ui.add_space(6.0);
                let (r, _) = ui.allocate_exact_size(EVec2::splat(64.0), Sense::hover());
                crate::canvas::draw_zebra_disc(ui.painter(), r.center(), 30.0);
                ui.add_space(4.0);
                ui.label(RichText::new("Zebra").font(style::semibold(15.0)));
                w::caption(ui, format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                w::caption(ui, "A zooming canvas for presentations, in the spirit of classic Prezi. Drivable by AI agents through an opt-in local API.");
                ui.add_space(4.0);
            });
        });
        if m.should_close() {
            app.show_about = false;
        }
    }
}
