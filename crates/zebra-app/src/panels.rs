//! Editor chrome, classic Mac layout (Keynote is the reference):
//! - a unified title bar + toolbar across the top (icon over label);
//! - the path sidebar on the left (opaque source list);
//! - the Format inspector on the right, persistent while shown (⌥⌘I);
//! - the canvas in the middle, with nothing floating over it except the
//!   empty-document hint, &crate::t!("canvas-back-to-content") and the HUD bezel.
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
/// Status bar at the bottom while AI control is on (Finder-style).
pub const STATUS_ROW: f32 = 22.0;
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
        .show(ui, |ui| {
                let t = std::time::Instant::now();
                toolbar(app, ui, &p);
                app.editor.perf.record_part("toolbar", t.elapsed().as_secs_f32() * 1000.0);
            });

    // Full window width, like Finder's status bar.
    if app.api.is_some() || app.demo_agent.is_some() {
        egui::Panel::bottom("agent-status")
            .exact_size(STATUS_ROW)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.toolbar).inner_margin(egui::Margin::symmetric(10, 0)).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| status_bar(app, ui, &p));
    }

    if app.show_sidebar {
        egui::Panel::left("path")
            .exact_size(SIDEBAR_W)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin { left: 10, right: 10, top: 8, bottom: 0 }).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| {
                let t = std::time::Instant::now();
                crate::sidebar::ui(app, ui, &p);
                app.editor.perf.record_part("sidebar", t.elapsed().as_secs_f32() * 1000.0);
            });
    }

    if app.show_props {
        egui::Panel::right("inspector")
            .exact_size(crate::inspector::WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin { left: 16, right: 12, top: 0, bottom: 0 }).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| {
                let t = std::time::Instant::now();
                crate::inspector::ui(app, ui);
                app.editor.perf.record_part("inspector", t.elapsed().as_secs_f32() * 1000.0);
            });
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
        let t = std::time::Instant::now();
        crate::canvas::edit_canvas(app, ui);
        app.editor.perf.record_part("canvas", t.elapsed().as_secs_f32() * 1000.0);
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
    let add_tip = crate::i18n::with_shortcut(if app.editor.view.selection.is_empty() { crate::t!("toolbar-add-step-tip-view") } else { crate::t!("toolbar-add-step-tip-selection") }, &Action::AddToPath);
    if w::toolbar_item(&mut left, Icon::AddStep, &crate::t!("toolbar-add-step"), ToolbarState { enabled: !app.editor.doc.elements.is_empty(), ..st() }).on_hover_text(&add_tip).clicked() {
        app.queued.push(Action::AddToPath);
    }
    let left_end = left.min_rect().right();

    // Right group (right to left)
    let mut right = ui.new_child(egui::UiBuilder::new().max_rect(row.shrink2(EVec2::new(inset, 0.0))).layout(Layout::right_to_left(Align::Center)));
    right.spacing_mut().item_spacing.x = 2.0;
    if w::toolbar_item(&mut right, Icon::Format, &crate::t!("toolbar-format"), ToolbarState { active: app.show_props, ..st() }).on_hover_text(crate::i18n::with_shortcut(if app.show_props { crate::t!("toolbar-format-tip-hide") } else { crate::t!("toolbar-format-tip-show") }, &Action::ToggleProperties)).clicked() {
        app.queued.push(Action::ToggleProperties);
    }
    ai_item(app, &mut right, p);
    let ex = w::toolbar_item(&mut right, Icon::Share, &crate::t!("toolbar-export"), ToolbarState { menu: true, enabled: !app.editor.doc.elements.is_empty(), ..st() }).on_hover_text(&crate::t!("toolbar-export-tip"));
    egui::Popup::menu(&ex).width(220.0).show(|ui| {
        if w::menu_item(ui, &crate::t!("toolbar-export-pdf"), Action::ExportPdf.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ExportPdf);
        }
        if w::menu_item(ui, &crate::t!("toolbar-export-png"), None).clicked() {
            app.queued.push(Action::ExportPng);
        }
    });
    let th = w::toolbar_item(&mut right, Icon::Theme, &crate::t!("toolbar-theme"), ToolbarState { menu: true, ..st() }).on_hover_text(&crate::t!("toolbar-theme-tip"));
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
    let r = w::toolbar_item(ui, Icon::SidebarLeft, &crate::t!("toolbar-view"), ToolbarState { menu: true, ..st() }).on_hover_text(&crate::t!("toolbar-view-tip"));
    egui::Popup::menu(&r).width(230.0).show(|ui| {
        if w::menu_check(ui, app.show_sidebar, &crate::t!("toolbar-view-path-sidebar"), Action::ToggleSidebar.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ToggleSidebar);
        }
        if w::menu_check(ui, app.show_props, &crate::t!("toolbar-view-inspector"), Action::ToggleProperties.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ToggleProperties);
        }
        if w::menu_check(ui, app.show_activity, &crate::t!("toolbar-view-agent-activity"), Action::ToggleActivity.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ToggleActivity);
        }
        w::menu_separator(ui);
        if w::menu_check(ui, app.editor.view.mode == Mode::PathEdit, &crate::t!("toolbar-view-edit-path"), Action::TogglePathEdit.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::TogglePathEdit);
        }
    });
}

/// Zoom: a small pop-up showing the zoom level, labelled &crate::t!("toolbar-zoom").
fn zoom_item(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let zoom = app.editor.view.camera.zoom;
    let txt = if zoom >= 100.0 || zoom < 0.01 { format!("{:.0e}%", zoom * 100.0) } else { format!("{:.0}%", zoom * 100.0) };
    let (rect, resp) = ui.allocate_exact_size(EVec2::new(64.0, 44.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, crate::t!("toolbar-zoom-level", level = txt.as_str())));
    let bez = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 14.0), EVec2::new(58.0, 21.0));
    w::paint_bezel(ui, bez, resp.is_pointer_button_down_on(), true);
    let painter = ui.painter();
    painter.text(Pos2::new(bez.left() + 8.0, bez.center().y), Align2::LEFT_CENTER, &txt, style::ui_font(12.0), p.text);
    paint_icon(painter, Rect::from_center_size(Pos2::new(bez.right() - 9.0, bez.center().y), EVec2::splat(10.0)), Icon::UpDown, p.weak);
    let galley = painter.layout_no_wrap(crate::t!("toolbar-zoom"), style::ui_font(11.0), p.text.gamma_multiply(0.9));
    painter.galley(Pos2::new(rect.center().x - galley.size().x / 2.0, rect.bottom() - galley.size().y - 1.0), galley, p.text);
    let resp = resp.on_hover_text(&crate::t!("toolbar-zoom"));
    egui::Popup::menu(&resp).width(200.0).show(|ui| {
        if w::menu_item(ui, &crate::t!("toolbar-zoom-to-fit"), Action::FitAll.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::FitAll);
        }
        let has_sel = !app.editor.view.selection.is_empty();
        if w::menu_row(ui, None, &crate::t!("toolbar-zoom-to-selection"), Action::ZoomToSelection.shortcut_text().as_deref(), has_sel).clicked() {
            app.queued.push(Action::ZoomToSelection);
        }
        w::menu_separator(ui);
        let reset_key = Action::ZoomReset.shortcut_text();
        for (z, l) in [(0.5, "50%"), (1.0, "100%"), (2.0, "200%")] {
            if w::menu_check(ui, (zoom - z).abs() < 0.005, l, if z == 1.0 { reset_key.as_deref() } else { None }).clicked() {
                let mut c = app.editor.view.target_camera();
                c.zoom = z;
                app.view_cmd(zebra_core::ViewCommand::SetCamera { camera: c, animate: true });
            }
        }
        w::menu_separator(ui);
        if w::menu_item(ui, &crate::t!("toolbar-zoom-in"), Action::ZoomIn.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ZoomIn);
        }
        if w::menu_item(ui, &crate::t!("toolbar-zoom-out"), Action::ZoomOut.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::ZoomOut);
        }
    });
}

fn centre_group(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let tool = app.editor.view.tool;
    let has = !app.editor.doc.elements.is_empty();
    let play = w::toolbar_item(ui, Icon::Play, &crate::t!("toolbar-play"), ToolbarState { enabled: has, tint: Some(accent()), ..st() })
        .on_hover_text(format!("{}\n{}", crate::i18n::with_shortcut(crate::t!("toolbar-play-tip"), &Action::PresentStart), crate::t!("toolbar-play-tip-detail")));
    if play.clicked() {
        app.queued.push(Action::PresentStart);
    }
    egui::Popup::context_menu(&play).show(|ui| {
        if w::menu_item(ui, &crate::t!("toolbar-play-from-start"), Action::PresentStart.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::PresentStart);
        }
        if w::menu_item(ui, &crate::t!("toolbar-play-from-current"), Action::PresentCurrent.shortcut_text().as_deref()).clicked() {
            app.queued.push(Action::PresentCurrent);
        }
        w::menu_separator(ui);
        if w::menu_check(ui, app.stage.presenter_view, &crate::t!("toolbar-presenter-view"), None).clicked() {
            app.queued.push(Action::TogglePresenterView);
        }
        if w::menu_item(ui, &crate::t!("toolbar-playback-options"), None).clicked() {
            app.queued.push(Action::PlaybackOptions);
        }
    });
    w::toolbar_separator(ui);

    if w::toolbar_item(ui, Icon::Text, &crate::t!("toolbar-text"), ToolbarState { active: tool == Tool::Text, ..st() }).on_hover_text(format!("{}\n{}", crate::i18n::with_shortcut(crate::t!("toolbar-text"), &Action::SetTool(Tool::Text)), crate::t!("toolbar-text-tip-detail"))).clicked() {
        app.queued.push(Action::SetTool(Tool::Text));
    }
    let fs = app.editor.view.frame_style;
    let fr = w::toolbar_item(ui, Icon::Frame(fs), &crate::t!("toolbar-frame"), ToolbarState { active: tool == Tool::Frame, menu: true, ..st() }).on_hover_text({ let k = Action::FrameTool.shortcut_text().unwrap_or_default(); format!("{}\n{}", crate::i18n::with_shortcut(crate::t!("toolbar-frame"), &Action::FrameTool), crate::t!("toolbar-frame-tip-detail", key = k.as_str())) });
    egui::Popup::menu(&fr).width(190.0).show(|ui| {
        for s in FrameStyle::ALL {
            if menu_icon_row(ui, Icon::Frame(s), frame_name(s), fs == s && tool == Tool::Frame).clicked() {
                app.queued.push(Action::SetFrameStyle(s));
            }
        }
        w::menu_separator(ui);
        let has_sel = !app.editor.view.selection.is_empty();
        if w::menu_row(ui, None, &crate::t!("toolbar-frame-selection"), Action::WrapInFrame.shortcut_text().as_deref(), has_sel).clicked() {
            app.queued.push(Action::WrapInFrame);
        }
    });
    let shape_active = matches!(tool, Tool::Rectangle | Tool::Ellipse | Tool::Triangle);
    let sh = w::toolbar_item(ui, Icon::Shapes, &crate::t!("toolbar-shape"), ToolbarState { active: shape_active, menu: true, ..st() }).on_hover_text(&crate::t!("toolbar-shape-tip"));
    egui::Popup::menu(&sh).width(200.0).show(|ui| {
        for (i, t, l) in [(Icon::Rect, Tool::Rectangle, &crate::t!("toolbar-shape-rectangle")), (Icon::Ellipse, Tool::Ellipse, &crate::t!("toolbar-shape-ellipse"))] {
            if menu_icon_row(ui, i, l, tool == t).clicked() {
                app.queued.push(Action::SetTool(t));
            }
        }
        w::menu_separator(ui);
        if w::menu_item(ui, &crate::t!("toolbar-shape-library"), None).clicked() {
            app.queued.push(Action::ShapeLibrary);
        }
    });
    let line_active = matches!(tool, Tool::Line | Tool::Arrow | Tool::Pen | Tool::Highlighter);
    let line_icon = match tool {
        Tool::Line => Icon::Line,
        Tool::Pen | Tool::Highlighter => Icon::Pen,
        _ => Icon::Arrow,
    };
    let ln = w::toolbar_item(ui, line_icon, &crate::t!("toolbar-line"), ToolbarState { active: line_active, menu: true, ..st() }).on_hover_text(&crate::t!("toolbar-line-tip"));
    egui::Popup::menu(&ln).width(190.0).show(|ui| {
        for (i, t, l) in [(Icon::Line, Tool::Line, &crate::t!("toolbar-line")), (Icon::Arrow, Tool::Arrow, &crate::t!("toolbar-line-arrow")), (Icon::Pen, Tool::Pen, &crate::t!("toolbar-line-draw"))] {
            if menu_icon_row(ui, i, l, tool == t).clicked() {
                app.queued.push(Action::SetTool(t));
            }
        }
    });
    let md = w::toolbar_item(ui, Icon::Media, &crate::t!("toolbar-media"), ToolbarState { menu: true, ..st() }).on_hover_text(&crate::t!("toolbar-media-tip"));
    egui::Popup::menu(&md).width(200.0).show(|ui| {
        if menu_icon_row(ui, Icon::Image, &crate::t!("toolbar-media-image"), false).clicked() {
            app.queued.push(Action::InsertImage);
        }
        if menu_icon_row(ui, Icon::Video, &crate::t!("toolbar-media-video"), false).clicked() {
            app.queued.push(Action::InsertVideo);
        }
        if menu_icon_row(ui, Icon::Note, &crate::t!("toolbar-media-audio"), false).clicked() {
            app.queued.push(Action::InsertMusic);
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
        ui.label(RichText::new(&crate::t!("toolbar-theme-my-themes")).size(11.0).weak());
        for (_, doc, t) in mine {
            if ui.button(&t.name).clicked() {
                crate::theme_editor::apply_library_theme(app, &doc, &t);
                ui.close();
            }
        }
    }
    ui.separator();
    if ui.button(&crate::t!("toolbar-theme-customize")).on_hover_text(&crate::t!("toolbar-theme-customize-tip")).clicked() {
        app.queued.push(Action::ThemeEditor);
        ui.close();
    }
}

/// The AI toolbar item. Off: a plain &crate::t!("ai-toolbar") item. On: a violet glyph, with
/// the agent's name as the label while one is connected.
fn ai_item(app: &mut ZebraApp, ui: &mut Ui, _p: &Palette) {
    let on = app.api.is_some() || app.demo_agent.is_some();
    let connected = app.agent_connected();
    let paused = app.editor.view.agent_paused;
    // The agent's name and what it is doing live in the status bar.
    let label = if app.api_error.is_some() { &crate::t!("ai-toolbar-error").to_string() } else { &crate::t!("ai-toolbar").to_string() };
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
        &crate::t!("ai-tip-off")
    } else if !connected {
        &crate::t!("ai-tip-waiting")
    } else if paused {
        &crate::t!("ai-tip-paused")
    } else {
        &crate::t!("ai-tip-connected")
    };
    let r = w::toolbar_item(ui, Icon::Sparkles, &label, ToolbarState { tint, active: on && !connected, ..st() });
    if connected && !paused {
        // a small live dot on the glyph, like a status item
        const BUSY: f64 = 2.0;
        let since = app.editor.activity.back().filter(|a| a.origin.is_agent()).map(|a| app.editor.view.now - a.time);
        let busy = since.is_some_and(|s| s < BUSY);
        if let Some(s) = since.filter(|_| busy) {
            // Turn the dot back to green when the agent goes quiet.
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(BUSY - s + 0.05));
        }
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
        ui.label(RichText::new(&crate::t!("ai-control-switch")).font(style::semibold(13.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if w::switch(ui, &mut want, &crate::t!("ai-control-switch")).on_hover_text(&crate::t!("ai-control-switch-tip")).changed() {
                app.queued.push(Action::ToggleAiControl);
            }
        });
    });
    w::caption(
        ui,
        &if on { crate::t!("ai-caption-on") } else { crate::t!("ai-caption-off") },
    );
    if let Some(e) = &app.api_error {
        ui.label(RichText::new(e).font(style::ui_font(11.0)).color(Color32::from_rgb(0xff, 0x3b, 0x30)));
    }
    if !on {
        return;
    }

    w::separator(ui);
    ui.label(RichText::new(&crate::t!("ai-connect-title")).font(style::semibold(13.0)));
    w::caption(ui, &crate::t!("ai-connect-paste"));
    let ctl = crate::cli::zebractl_command();
    let prompt = crate::cli::agent_prompt(&ctl);
    copy_box(ui, &prompt, "agent-prompt");
    ui.horizontal_wrapped(|ui| {
        if let Some(installed) = crate::cli::install_state() {
            if installed {
                w::caption(ui, &crate::t!("ai-zebractl-installed"));
            } else if w::push_button(ui, &crate::t!("ai-install-cli")).on_hover_text(&crate::t!("ai-install-cli-tip")).clicked() {
                match crate::cli::install() {
                    Ok(()) => app.toast(&crate::t!("ai-toast-cli-installed")),
                    Err(e) => app.toast(crate::t!("ai-toast-cli-install-failed", error = e.to_string())),
                }
            }
        }
        let mcp = w::push_button(ui, &crate::t!("ai-mcp-setup"));
        egui::Popup::menu(&mcp).width(330.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            egui::Frame::new().inner_margin(8).show(ui, |ui| {
                w::caption(ui, &crate::t!("ai-mcp-caption"));
                copy_box(ui, &crate::cli::mcp_config(&ctl), "mcp-json");
            });
        });
    });

    w::separator(ui);
    if !connected {
        w::caption(ui, &crate::t!("ai-no-agent"));
    } else {
        ui.horizontal(|ui| {
            if w::push_button(ui, &if paused { crate::t!("ai-resume") } else { crate::t!("ai-pause") }).on_hover_text(&crate::t!("ai-pause-tip")).clicked() {
                app.queued.push(Action::PauseAgents);
            }
            if w::push_button(ui, &crate::t!("ai-revert-changes")).on_hover_text(&crate::t!("ai-revert-tip")).clicked() {
                app.queued.push(Action::RevertAgent);
            }
        });
        let mut follow = app.editor.view.follow_agent;
        if w::checkbox(ui, &mut follow, &crate::t!("ai-follow-camera")).on_hover_text(&crate::t!("ai-follow-tip")).changed() {
            app.queued.push(Action::FollowAgent);
        }
    }
    if w::push_button(ui, &crate::t!("ai-show-activity")).on_hover_text(Action::ToggleActivity.shortcut_text().unwrap_or_default()).clicked() {
        app.queued.push(Action::ToggleActivity);
    }
    ui.add_space(2.0);
    ui.label(RichText::new(&crate::t!("ai-permission-title")).font(style::semibold(11.0)).color(p.weak));
    let cur = app.editor.view.agent_permission;
    for (perm, label) in [
        (zebra_core::view::Permission::Observe, &crate::t!("ai-permission-observe")),
        (zebra_core::view::Permission::Edit, &crate::t!("ai-permission-edit")),
        (zebra_core::view::Permission::Full, &crate::t!("ai-permission-full")),
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
        // Left-aligned and never justified: a long path that drops to the
        // next line no longer spreads the line before it.
        let mut job = egui::text::LayoutJob::simple(text.to_string(), egui::FontId::monospace(11.0), ui.visuals().text_color(), ui.available_width());
        job.justify = false;
        job.halign = egui::Align::LEFT;
        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| ui.add(egui::Label::new(job)));
    });
    let copied_id = ui.scope_id().with(("copied", id));
    let copied_at: Option<f64> = ui.data(|d| d.get_temp(copied_id));
    let now = ui.input(|i| i.time);
    let copied = copied_at.is_some_and(|t| now - t < 1.5);
    if w::push_button_ext(ui, &if copied { crate::t!("copy-box-copied") } else { crate::t!("copy-box-copy") }, w::Size::Small, true, None).clicked() {
        ui.ctx().copy_text(text.to_string());
        ui.data_mut(|d| d.insert_temp(copied_id, now));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(1600));
    }
}

// ----------------------------------------------------------------------
// Over the canvas: empty-document hint, Back to Content, HUD, recovery

/// The bottom status bar while AI control is on: which agent is connected
/// and its latest change, fading to idle. Click: the Activity pane.
fn status_bar(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    const RECENT: f64 = 30.0;
    let now = app.editor.view.now;
    let connected = app.agent_connected();
    let name = app.agent_name();
    // That agent's own latest change, never another agent's.
    let last = app
        .editor
        .activity
        .iter()
        .rev()
        .find(|a| matches!(&a.origin, Origin::Agent { name: n } if *n == name))
        .map(|a| (a.time, a.label.clone()));
    let (text, live) = match (connected, app.editor.view.agent_paused, &last) {
        (true, true, _) => (crate::t!("status-agent-paused", name = name.as_str()), false),
        (true, false, Some((t, l))) if now - t < RECENT => {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(RECENT - (now - t) + 0.05));
            (crate::t!("status-agent-action", name = name.as_str(), action = l.as_str()), true)
        }
        (true, false, _) => (crate::t!("status-agent-idle", name = name.as_str()), false),
        (false, _, _) => (crate::t!("status-no-agent"), false),
    };
    let rect = ui.max_rect();
    let resp = ui.interact(rect, egui::Id::unique("agent-status"), Sense::click());
    let color = if live { p.text } else { p.weak };
    let icon = Rect::from_center_size(Pos2::new(rect.left() + 7.0, rect.center().y), EVec2::splat(13.0));
    paint_icon(ui.painter(), icon, Icon::Sparkles, if connected { AGENT } else { p.weak });
    let max_w = rect.width() - 30.0;
    let mut job = egui::text::LayoutJob::simple_singleline(text, style::ui_font(11.5), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(Pos2::new(icon.right() + 6.0, rect.center().y - galley.size().y / 2.0), galley, color);
    let tip = crate::i18n::with_shortcut(crate::t!("status-activity"), &Action::ToggleActivity);
    if resp.on_hover_text(tip).clicked() {
        app.show_activity = !app.show_activity;
    }
}

fn floating(app: &mut ZebraApp, ctx: &egui::Context, rect: Rect, p: &Palette) {
    // Empty-document hint (full-strength secondary text, 13 pt minimum).
    if app.editor.doc.elements.is_empty() && app.canvas.text.is_none() {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("hint"))).with_clip_rect(rect);
        let bg = style::c32(app.editor.theme().color(&"theme:background".into()));
        let dark_bg = (bg.r() as u32 + bg.g() as u32 + bg.b() as u32) < 380;
        let (c1, c2) = if dark_bg { (Color32::from_gray(0xd0), Color32::from_gray(0xa8)) } else { (Color32::from_gray(0x55), Color32::from_gray(0x6b)) };
        painter.text(rect.center() - EVec2::new(0.0, 14.0), Align2::CENTER_CENTER, &crate::t!("canvas-hint-title"), style::semibold(20.0), c1);
        painter.text(rect.center() + EVec2::new(0.0, 16.0), Align2::CENTER_CENTER, crate::t!("canvas-hint-detail", key = Action::FrameTool.shortcut_text().unwrap_or_default()), FontId::proportional(13.0), c2);
    } else if app.editor.doc.bounds().is_some_and(|b| !b.intersects(&app.editor.view.camera.visible_bounds(app.editor.view.viewport))) {
        egui::Area::new(egui::Id::unique("backtocontent"))
            .anchor(Align2::CENTER_TOP, EVec2::new(rect.center().x - ctx.content_rect().center().x, rect.top() + 14.0))
            .show(ctx, |ui| {
                if w::push_button(ui, &crate::t!("canvas-back-to-content")).on_hover_text(crate::i18n::with_shortcut(crate::t!("canvas-back-to-content-tip"), &Action::FitAll)).clicked() {
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
                        ui.label(&crate::t!("recover-message"));
                        if w::push_button(ui, &crate::t!("recover-dismiss")).clicked() {
                            app.recovered = None;
                        }
                        if w::default_button(ui, &crate::t!("recover-restore")).clicked() {
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
        ui.label(RichText::new(&crate::t!("activity-title")).font(style::semibold(13.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if w::icon_button(ui, Icon::Close, 20.0, true, false).on_hover_text(crate::i18n::with_shortcut(crate::t!("activity-close"), &Action::ToggleActivity)).clicked() {
                app.show_activity = false;
            }
        });
    });
    w::caption(ui, &crate::t!("activity-caption"));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let paused = app.editor.view.agent_paused;
        if w::push_button(ui, &if paused { crate::t!("activity-resume") } else { crate::t!("activity-pause") }).on_hover_text(&if paused { crate::t!("activity-resume-tip") } else { crate::t!("activity-pause-tip") }).clicked() {
            app.queued.push(Action::PauseAgents);
        }
        if w::push_button(ui, &crate::t!("activity-revert")).on_hover_text(&crate::t!("activity-revert-tip")).clicked() {
            app.queued.push(Action::RevertAgent);
        }
    });
    let mut follow = app.editor.view.follow_agent;
    if w::checkbox(ui, &mut follow, &crate::t!("activity-follow")).on_hover_text(&crate::t!("activity-follow-tip")).changed() {
        app.queued.push(Action::FollowAgent);
    }
    ui.add_space(4.0);
    w::separator(ui);
    let now = app.editor.view.now;
    let items: Vec<_> = app.editor.activity.iter().rev().filter(|a| a.origin.is_agent()).take(200).cloned().collect();
    if items.is_empty() {
        ui.add_space(6.0);
        w::caption(ui, &crate::t!("activity-empty"));
    }
    for a in items {
        let ago = now - a.time;
        let when = if ago < 60.0 { crate::t!("activity-seconds-ago", seconds = (ago.max(0.0).round() as u64)) } else { crate::t!("activity-minutes-ago", minutes = ((ago / 60.0).round() as u64)) };
        let r = egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)).corner_radius(5).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(EVec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 3.5, AGENT);
                ui.label(RichText::new(&a.label).size(13.0));
            });
            ui.label(RichText::new(crate::t!("activity-item-meta", origin = a.origin.display(), when = when.as_str(), count = (a.ids.len() as u64))).size(11.0).color(p.weak));
        });
        let resp = r.response.interact(Sense::click()).on_hover_text(&crate::t!("activity-item-tip"));
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
            let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text(&crate::t!("palette-hint")).desired_width(f32::INFINITY).font(FontId::proportional(15.0)));
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
            ui.label(RichText::new(&crate::t!("shortcuts-title")).font(style::semibold(15.0)));
            w::caption(ui, &crate::t!("shortcuts-caption"));
            ui.add_space(6.0);
            let k = |acts: &[Action]| acts.iter().filter_map(|a| a.shortcut_text()).collect::<Vec<_>>().join(" · ");
            use egui::{Key, Modifiers as M};
            let key = |m: M, k: Key| crate::i18n::key_text(m, k);
            let rows: Vec<(String, String)> = vec![
                (crate::t!("shortcuts-write-text"), crate::t!("shortcuts-write-text-keys")),
                (
                    crate::t!("shortcuts-select-hand"),
                    crate::t!(
                        "shortcuts-select-hand-keys",
                        select = k(&[Action::SetTool(Tool::Select)]),
                        hand = k(&[Action::SetTool(Tool::Hand)]),
                        space = key(M::NONE, Key::Space)
                    ),
                ),
                (crate::t!("shortcuts-text-frame"), k(&[Action::SetTool(Tool::Text), Action::FrameTool])),
                (
                    crate::t!("shortcuts-shapes"),
                    k(&[
                        Action::SetTool(Tool::Rectangle),
                        Action::SetTool(Tool::Ellipse),
                        Action::SetTool(Tool::Line),
                        Action::SetTool(Tool::Arrow),
                        Action::SetTool(Tool::Pen),
                    ]),
                ),
                (crate::t!("shortcuts-insert-image"), k(&[Action::InsertImage])),
                (crate::t!("shortcuts-pan-zoom"), crate::t!("shortcuts-pan-zoom-keys", modifier = crate::i18n::modifier_text(M::COMMAND))),
                (crate::t!("shortcuts-zoom"), k(&[Action::FitAll, Action::ZoomToSelection, Action::ZoomReset])),
                (crate::t!("shortcuts-undo-redo"), k(&[Action::Undo, Action::Redo])),
                (crate::t!("shortcuts-duplicate-delete"), k(&[Action::Duplicate, Action::Delete])),
                (crate::t!("shortcuts-forward-backward"), k(&[Action::BringForward, Action::SendBackward])),
                (crate::t!("shortcuts-rotate"), crate::t!("shortcuts-rotate-keys", modifier = crate::i18n::modifier_text(M::SHIFT))),
                (crate::t!("shortcuts-path"), k(&[Action::TogglePathEdit, Action::AddToPath])),
                (crate::t!("shortcuts-play"), k(&[Action::PresentStart, Action::PresentCurrent])),
                (
                    crate::t!("shortcuts-while-playing"),
                    crate::t!(
                        "shortcuts-while-playing-keys",
                        next = key(M::NONE, Key::ArrowRight),
                        space = key(M::NONE, Key::Space),
                        back = key(M::NONE, Key::ArrowLeft),
                        home = key(M::NONE, Key::Home),
                        black = key(M::NONE, Key::B),
                        white = key(M::NONE, Key::W),
                        stop = key(M::NONE, Key::Escape)
                    ),
                ),
                (crate::t!("shortcuts-panels"), k(&[Action::ToggleProperties, Action::ToggleSidebar])),
                (crate::t!("shortcuts-agent"), k(&[Action::ToggleActivity, Action::StopAgents])),
                (crate::t!("shortcuts-settings"), k(&[Action::Settings])),
            ];
            egui::Grid::new("sc").num_columns(2).spacing([24.0, 6.0]).striped(true).show(ui, |ui| {
                for (a, k) in &rows {
                    ui.label(a);
                    ui.label(RichText::new(k).color(p.weak));
                    ui.end_row();
                }
            });
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::default_button(ui, &crate::t!("shortcuts-done")).clicked() {
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
                ui.label(RichText::new(&crate::t!("about-name")).font(style::semibold(15.0)));
                w::caption(ui, crate::t!("about-version", version = env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                w::caption(ui, &crate::t!("about-blurb"));
                ui.add_space(4.0);
            });
        });
        if m.should_close() {
            app.show_about = false;
        }
    }
}
