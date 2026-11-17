//! Present mode: full screen, chrome hidden, the camera flies the path.
use egui::{Color32, CursorIcon, FontId, Key, PointerButton, Sense, Ui};
use zebra_core::view::Blank;
use zebra_core::{Origin, ViewCommand};

use crate::app::ZebraApp;

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
        crate::canvas::sync_viewport(app, rect);
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        crate::canvas::paint_content(app, ui, rect, true);

        crate::canvas::handle_scroll(app, ui, resp.contains_pointer());
        if resp.dragged_by(PointerButton::Primary) || resp.dragged_by(PointerButton::Middle) {
            let d = resp.drag_delta();
            let _ = app.editor.view(ViewCommand::Pan { dx: d.x as f64, dy: d.y as f64 }, H);
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                // Click an object to zoom to it (off-path); click empty space = next.
                match crate::canvas::hit(app, p, false) {
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
        crate::canvas::draw_agent_presence(app, &painter);
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
            let n = app.editor.doc.path.len();
            let txt = match app.editor.view.present.step {
                Some(s) if app.editor.view.present.off_path => format!("{} / {n}  ·  off path — → to return", s + 1),
                Some(s) => format!("{} / {n}", s + 1),
                None => format!("Overview  ·  {n} steps"),
            };
            let a = ((2.0 - idle) / 0.4).clamp(0.0, 1.0) as f32;
            let galley = painter.layout_no_wrap(txt, FontId::proportional(13.0), Color32::WHITE.gamma_multiply(a));
            let r = egui::Rect::from_min_size(rect.right_bottom() - galley.size() - egui::vec2(36.0, 30.0), galley.size() + egui::vec2(20.0, 10.0));
            painter.rect_filled(r, 8.0, Color32::from_black_alpha((150.0 * a) as u8));
            painter.galley(r.min + egui::vec2(10.0, 5.0), galley, Color32::WHITE);
            let hint = painter.layout_no_wrap("Esc to exit".into(), FontId::proportional(12.0), Color32::WHITE.gamma_multiply(a * 0.9));
            let hr = egui::Rect::from_min_size(rect.left_bottom() + egui::vec2(24.0, -30.0 - hint.size().y - 10.0), hint.size() + egui::vec2(20.0, 10.0));
            painter.rect_filled(hr, 8.0, Color32::from_black_alpha((120.0 * a) as u8));
            painter.galley(hr.min + egui::vec2(10.0, 5.0), hint, Color32::WHITE);
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if resp.contains_pointer() {
            ctx.set_cursor_icon(CursorIcon::None);
        }
    });
}

fn toggle(cur: Option<Blank>, b: Blank) -> Option<Blank> {
    if cur == Some(b) { None } else { Some(b) }
}
