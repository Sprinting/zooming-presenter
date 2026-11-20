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
        if crate::presenting::jump_key(app, &ev) {
            continue;
        }
        let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { continue };
        let cmd = match key {
            Key::ArrowRight | Key::ArrowDown | Key::Space | Key::PageDown | Key::Enter if !modifiers.command => Some(ViewCommand::Next),
            Key::ArrowLeft | Key::ArrowUp | Key::PageUp | Key::Backspace => Some(ViewCommand::Prev),
            Key::Home => Some(ViewCommand::Overview),
            Key::O if !modifiers.command => Some(ViewCommand::Overview),
            Key::End => app.editor.doc.path.len().checked_sub(1).map(|s| ViewCommand::GotoStep { step: s }),
            Key::B | Key::Period if !modifiers.command => Some(ViewCommand::Blank { blank: toggle(app.blank(), Blank::Black) }),
            Key::W if !modifiers.command => Some(ViewCommand::Blank { blank: toggle(app.blank(), Blank::White) }),
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
        deck_mask(app, &ui.painter_at(rect), rect);
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
                let link = app.editor.elements_at(w, 0.0).into_iter().rev().filter(|e| e.bounds().contains(w)).find_map(|e| app.renderer.link_at(&app.editor, e, w));
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

        // The audience sees the show only: an agent's cursor, its name and
        // the flashes of its edits are for the editor, never this screen.
        let painter = ui.painter_at(rect);
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
        // Typing a step number keeps the bezel up with the number in it.
        if app.stage.jump.is_some() {
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
            let txt = crate::presenting::jump_text(app, false).unwrap_or(txt);
            let a = ((2.0 - idle) / 0.4).clamp(0.0, 1.0) as f32;
            let white = Color32::WHITE.gamma_multiply(a);
            let font = crate::style::ui_font(13.0);
            let galley = painter.layout_no_wrap(txt, font.clone(), white);
            let hint = if app.stage.jump.is_some() { crate::t!("present-hud-go-to-hint") } else { crate::t!("present-hud-exit", key = "Esc") };
            let esc = painter.layout_no_wrap(hint, crate::style::ui_font(11.0), Color32::from_gray(200).gamma_multiply(a));
            // Bezel metrics: 36 pt high, 8 pt corners (a rounded rect, not a
            // capsule), 6 pt from the edge to the 28 pt step buttons (their
            // glyph is inset 8 pt, so 14 pt to the ink), 14 pt after the
            // exit hint. Both texts sit on one baseline.
            const HUD_H: f32 = 36.0;
            const EDGE: f32 = 6.0;
            const GAP: f32 = 6.0;
            const SEP: f32 = 12.0;
            const END: f32 = 14.0;
            let btn = egui::vec2(28.0, 28.0);
            let w = EDGE + btn.x + GAP + galley.size().x + GAP + btn.x + SEP + 1.0 + SEP + esc.size().x + END;
            let hud = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.bottom() - 44.0), egui::vec2(w.round(), HUD_H));
            painter.rect_filled(hud, 8.0, Color32::from_black_alpha((170.0 * a) as u8));
            // One baseline for the step count and the exit hint, with the
            // step count's capitals centred in the bezel.
            let base = |g: &egui::Galley| g.rows.first().and_then(|r| r.row.glyphs.first().map(|gl| r.pos.y + gl.pos.y)).unwrap_or(g.size().y * 0.8);
            let baseline = (hud.center().y + 13.0 * 0.71 / 2.0).round();
            let mut x = hud.left() + EDGE;
            let hud_id = app.uid("present-hud");
            let step_btn = |painter: &egui::Painter, ui: &mut egui::Ui, x: f32, icon: Icon, id: &str, enabled: bool| -> bool {
                let r = egui::Rect::from_min_size(egui::pos2(x, hud.center().y - btn.y / 2.0), btn);
                let resp = ui.interact(r, hud_id.with(id), Sense::click());
                let label = if id == "prev" { crate::t!("present-hud-previous-step") } else { crate::t!("present-hud-next-step") };
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, enabled, label.clone()));
                if resp.hovered() && enabled {
                    painter.rect_filled(r, 5.0, Color32::from_white_alpha((40.0 * a) as u8));
                }
                let col = if enabled { white } else { white.gamma_multiply(0.35) };
                crate::icons::paint_icon(painter, r.shrink(8.0), icon, col);
                enabled && resp.clicked()
            };
            let at_end = app.editor.view.present.step.is_some_and(|s| s + 1 >= n && app.editor.view.present.build >= app.editor.doc.path.get(s).map_or(0, |st| st.builds.len()));
            if step_btn(&painter, ui, x, Icon::ChevronLeft, "prev", app.editor.view.present.step.is_some()) {
                let _ = app.editor.view(ViewCommand::Prev, H);
            }
            x += btn.x + GAP;
            let at = egui::pos2(x, baseline - base(&galley));
            let pos_rect = egui::Rect::from_min_size(at, galley.size()).expand(4.0);
            let pos_resp = ui.interact(pos_rect, hud_id.with("go-to"), Sense::click()).on_hover_text(crate::t!("present-go-to-tip"));
            if pos_resp.clicked() && n > 0 {
                app.stage.jump = Some(String::new());
            }
            if app.stage.jump.is_some() {
                painter.line_segment([pos_rect.left_bottom() + egui::vec2(4.0, -2.0), pos_rect.right_bottom() + egui::vec2(-4.0, -2.0)], egui::Stroke::new(1.0, white));
            }
            painter.galley(at, galley.clone(), white);
            x += galley.size().x + GAP;
            if step_btn(&painter, ui, x, Icon::ChevronRight, "next", !at_end) {
                let _ = app.editor.view(ViewCommand::Next, H);
            }
            x += btn.x + SEP;
            let sx = x.round() + 0.5;
            painter.line_segment([egui::pos2(sx, hud.top() + 9.0), egui::pos2(sx, hud.bottom() - 9.0)], egui::Stroke::new(1.0, Color32::from_white_alpha((60.0 * a) as u8)));
            x += 1.0 + SEP;
            painter.galley(egui::pos2(x, baseline - base(&esc)), esc, white);
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

/// The current step's imported page (a PDF page or PowerPoint slide,
/// [`zebra_core::Element::is_page`]) and how strongly to hide what is
/// around it (0..1): a page step shows only the page, like a slide deck.
/// It fades in over the end of the flight there; off the path, in the
/// overview and on every other step the canvas shows as usual.
pub fn deck_page(ed: &zebra_core::Editor) -> Option<([zebra_core::Vec2; 4], f32)> {
    let p = &ed.view.present;
    if ed.view.mode != zebra_core::Mode::Present || p.off_path {
        return None;
    }
    let step = ed.doc.path.get(p.step?)?;
    let zebra_core::StepTarget::Element { id } = &step.target else { return None };
    let e = ed.doc.get(id)?;
    if !e.is_page() {
        return None;
    }
    let a = match &ed.view.flight {
        Some(f) => (((f.progress(ed.view.now) - 0.7) / 0.3).clamp(0.0, 1.0)) as f32,
        None => 1.0,
    };
    let (w, h) = e.visual_size();
    let (hw, hh) = (w / 2.0, h / 2.0);
    let c = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].map(|(x, y)| e.transform.apply(zebra_core::Vec2::new(x, y)));
    (a > 0.0).then_some((c, a))
}

/// Black over everything but the page (see [`deck_page`]).
fn deck_mask(app: &ZebraApp, painter: &egui::Painter, rect: egui::Rect) {
    let Some((corners, a)) = deck_page(&app.editor) else { return };
    let inner = corners.map(|w| app.canvas.w2s(&app.editor, w));
    let outer = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    let col = Color32::BLACK.gamma_multiply(a);
    // A frame of four quads between the screen's edge and the page's.
    let mut mesh = egui::Mesh::default();
    for p in outer.iter().chain(inner.iter()) {
        mesh.colored_vertex(*p, col);
    }
    for i in 0..4u32 {
        let j = (i + 1) % 4;
        mesh.add_triangle(i, j, 4 + j);
        mesh.add_triangle(i, 4 + j, 4 + i);
    }
    painter.add(egui::Shape::mesh(mesh));
}
