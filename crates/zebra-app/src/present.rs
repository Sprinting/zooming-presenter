//! Present mode: full screen, chrome hidden, the camera flies the path.
use egui::{Color32, CursorIcon, Key, PointerButton, Sense, Ui};
use zebra_core::view::Blank;
use zebra_core::{Origin, ViewCommand};

use crate::app::ZebraApp;
use crate::icons::Icon;

const H: Origin = Origin::Human;

pub fn ui(app: &mut ZebraApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let now = app.editor.view.now;

    // Keys.
    let events = ctx.input(|i| i.events.clone());
    for ev in events {
        let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { continue };
        let cmd = match key {
            Key::ArrowRight | Key::ArrowDown | Key::Space | Key::PageDown | Key::Enter if !modifiers.command => Some(ViewCommand::Next),
            Key::ArrowLeft | Key::ArrowUp | Key::PageUp | Key::Backspace => Some(ViewCommand::Prev),
            Key::Home | Key::O => Some(ViewCommand::Overview),
            Key::End => app.editor.doc.path.len().checked_sub(1).map(|s| ViewCommand::GotoStep { step: s }),
            Key::B | Key::Period if !modifiers.command => Some(ViewCommand::Blank { blank: toggle(app.blank(), Blank::Black) }),
            Key::W => Some(ViewCommand::Blank { blank: toggle(app.blank(), Blank::White) }),
            Key::Escape => Some(ViewCommand::ExitPresent),
            Key::Period if modifiers.command => None,
            _ => None,
        };
        if let Some(c) = cmd {
            let _ = app.editor.view(c, H);
        }
        if modifiers.command && key == Key::Period {
            app.queued.push(crate::actions::Action::StopAgents);
        }
    }

    // Track fade-in reveals (~300 ms).
    let hidden = app.editor.hidden_builds();
    for id in app.prev_hidden.difference(&hidden) {
        app.revealed.insert(id.clone(), now);
    }
    app.revealed.retain(|_, t| now - *t < 0.4);
    if !app.revealed.is_empty() {
        ctx.request_repaint();
    }
    app.prev_hidden = hidden;

    egui::CentralPanel::no_frame().show(ui, |ui| {
        let rect = ui.max_rect();
        crate::canvas::sync_viewport(app, rect, ui.ctx().content_rect().size());
        let resp = ui.interact(rect, app.uid("zebra-canvas"), Sense::click_and_drag());
        crate::canvas::paint_content(app, ui, rect, true);
        crate::presenting::present_extras(app, ui, rect);

        crate::canvas::handle_scroll(app, ui, resp.contains_pointer());
        if resp.dragged_by(PointerButton::Primary) || resp.dragged_by(PointerButton::Middle) {
            let d = resp.drag_delta();
            let _ = app.editor.view(ViewCommand::Pan { dx: d.x as f64, dy: d.y as f64 }, H);
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                // A link in text opens; an object zooms (off-path); empty space = next.
                let w = app.canvas.s2w(&app.editor, p);
                let link = app.editor.doc.ordered().rev().filter(|e| e.bounds().contains(w)).find_map(|e| app.renderer.link_at(&app.editor, e, w));
                if let Some(url) = link {
                    crate::app::open_url(&url);
                    app.toast(crate::t!("present-toast-opened-link", url = url.as_str()));
                    return;
                }
                match crate::canvas::hit(app, p, false) {
                    // Click a video to play or pause it.
                    Some(id) if matches!(app.editor.doc.get(&id).map(|e| &e.kind), Some(zebra_core::ElementKind::Video(_))) => {
                        crate::media_ui::toggle(app, &id);
                    }
                    Some(id) => {
                        let _ = app.editor.view(ViewCommand::ZoomTo { id, animate: true }, H);
                    }
                    None => {
                        let _ = app.editor.view(ViewCommand::Next, H);
                    }
                }
            }
        }
        if resp.secondary_clicked() {
            let _ = app.editor.view(ViewCommand::Prev, H);
        }

        let painter = ui.painter_at(rect);
        let _ = crate::canvas::draw_agent_presence(app, &painter);
        match app.blank() {
            Some(Blank::Black) => {
                painter.rect_filled(rect, 0.0, Color32::BLACK);
            }
            Some(Blank::White) => {
                painter.rect_filled(rect, 0.0, Color32::WHITE);
            }
            None => {}
        }

        // Pointer activity: show the step counter, hide the cursor when idle.
        let pos = ctx.input(|i| i.pointer.hover_pos());
        if pos != app.last_pointer {
            app.last_pointer = pos;
            app.last_pointer_move = app.now();
        }
        let idle = app.now() - app.last_pointer_move;
        if idle < 2.0 {
            // A dark HUD bezel at the bottom centre: ‹ position › and how to
            // leave, so mouse users can step without the keyboard (critique 21).
            let n = app.editor.doc.path.len();
            let txt = match app.editor.view.present.step {
                Some(s) if app.editor.view.present.off_path => crate::t!("present-hud-off-path", step = ((s + 1) as u64), count = n, key = "→"),
                Some(s) => crate::t!("present-hud-step-of", step = ((s + 1) as u64), count = n),
                None => crate::t!("present-hud-overview", count = n),
            };
            let a = ((2.0 - idle) / 0.4).clamp(0.0, 1.0) as f32;
            let white = Color32::WHITE.gamma_multiply(a);
            let font = crate::style::ui_font(13.0);
            let galley = painter.layout_no_wrap(txt, font.clone(), white);
            let esc = painter.layout_no_wrap(crate::t!("present-hud-exit", key = "Esc"), crate::style::ui_font(11.0), Color32::from_gray(200).gamma_multiply(a));
            let btn = egui::vec2(30.0, 30.0);
            let w = btn.x * 2.0 + galley.size().x + 24.0 + 1.0 + esc.size().x + 28.0;
            let hud = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.bottom() - 44.0), egui::vec2(w, 38.0));
            painter.rect_filled(hud, 10.0, Color32::from_black_alpha((170.0 * a) as u8));
            let mut x = hud.left() + 4.0;
            let hud_id = app.uid("present-hud");
            let step_btn = |painter: &egui::Painter, ui: &mut egui::Ui, x: f32, icon: Icon, id: &str, enabled: bool| -> bool {
                let r = egui::Rect::from_min_size(egui::pos2(x, hud.center().y - btn.y / 2.0), btn);
                let resp = ui.interact(r, hud_id.with(id), Sense::click());
                let label = if id == "prev" { crate::t!("present-hud-previous-step") } else { crate::t!("present-hud-next-step") };
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, enabled, label.clone()));
                if resp.hovered() && enabled {
                    painter.rect_filled(r, 6.0, Color32::from_white_alpha((40.0 * a) as u8));
                }
                let col = if enabled { white } else { white.gamma_multiply(0.35) };
                crate::icons::paint_icon(painter, r.shrink(8.0), icon, col);
                enabled && resp.clicked()
            };
            let at_end = app.editor.view.present.step.is_some_and(|s| s + 1 >= n && app.editor.view.present.build >= app.editor.doc.path[s].builds.len());
            if step_btn(&painter, ui, x, Icon::ChevronLeft, "prev", app.editor.view.present.step.is_some()) {
                let _ = app.editor.view(ViewCommand::Prev, H);
            }
            x += btn.x + 12.0;
            painter.galley(egui::pos2(x, hud.center().y - galley.size().y / 2.0), galley.clone(), white);
            x += galley.size().x + 12.0;
            if step_btn(&painter, ui, x, Icon::ChevronRight, "next", !at_end) {
                let _ = app.editor.view(ViewCommand::Next, H);
            }
            x += btn.x + 8.0;
            painter.line_segment([egui::pos2(x, hud.top() + 9.0), egui::pos2(x, hud.bottom() - 9.0)], egui::Stroke::new(1.0, Color32::from_white_alpha((60.0 * a) as u8)));
            x += 12.0;
            painter.galley(egui::pos2(x, hud.center().y - esc.size().y / 2.0), esc, white);
            // Repaint only for the fade, not while the bezel just sits there.
            if idle < 1.6 {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.6 - idle));
            } else {
                ctx.request_repaint();
            }
        } else if resp.contains_pointer() {
            ctx.set_cursor_icon(CursorIcon::None);
        }
    });
}

fn toggle(cur: Option<Blank>, b: Blank) -> Option<Blank> {
    if cur == Some(b) { None } else { Some(b) }
}
