//! Left path sidebar: numbered thumbnails, drag to reorder, delete, add.
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2 as EVec2};
use zebra_core::{Command, Mode, Origin, StepTarget, ViewCommand};

use crate::actions::Action;
use crate::app::ZebraApp;
use crate::style::{self, Palette, ACCENT, PATH};
use crate::thumbs::{THUMB_H, THUMB_W};

const H: Origin = Origin::Human;

pub fn step_label(app: &ZebraApp, i: usize) -> String {
    let doc = &app.editor.doc;
    match &doc.path[i].target {
        StepTarget::Element { id } => doc.get(id).map(|e| {
            let l = e.label(doc);
            if l == "frame" { "Untitled frame".into() } else { l }
        }).unwrap_or_else(|| "(missing)".into()),
        StepTarget::View { .. } => "View".into(),
        StepTarget::Overview => "Overview".into(),
    }
}

pub fn ui(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let n = app.editor.doc.path.len();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Path").font(style::bold(14.0)));
        ui.label(RichText::new(format!("{n} step{}", if n == 1 { "" } else { "s" })).size(12.0).color(p.weak));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let editing = app.editor.view.mode == Mode::PathEdit;
            let b = egui::Button::new(RichText::new(if editing { "Done" } else { "Edit path" }).size(12.0).color(if editing { Color32::WHITE } else { p.text }))
                .fill(if editing { PATH } else { p.hover })
                .corner_radius(6);
            if ui.add(b).on_hover_text("Click objects on the canvas to add them as steps  (P)").clicked() {
                app.queued.push(Action::TogglePathEdit);
            }
        });
    });
    ui.add_space(6.0);

    let footer_h = 74.0;
    let avail = ui.available_height() - footer_h;
    let drag_id = egui::Id::unique("path-drag");
    let mut dragging: Option<usize> = ui.ctx().data(|d| d.get_temp(drag_id));
    let mut rows: Vec<Rect> = vec![];
    let mut drop_to: Option<usize> = None;
    let pointer = ui.ctx().pointer_hover_pos();

    egui::ScrollArea::vertical().max_height(avail.max(60.0)).auto_shrink([false, false]).show(ui, |ui| {
        if n == 0 {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No steps yet").color(p.weak));
                ui.add_space(4.0);
                ui.label(RichText::new("Select a frame and press “+ Add”, or click “Edit path” and click frames in order.").size(12.0).color(p.weak));
            });
        }
        let tw = ui.available_width();
        let th = tw * THUMB_H as f32 / THUMB_W as f32;
        for i in 0..n {
            let (r, resp) = ui.allocate_exact_size(EVec2::new(tw, th + 22.0), Sense::click_and_drag());
            rows.push(r);
            let selected = app.selected_step == Some(i) || (app.editor.view.mode == Mode::Present && app.editor.view.present.step == Some(i));
            let painter = ui.painter();
            let img_r = Rect::from_min_size(r.min + EVec2::new(18.0, 0.0), EVec2::new(tw - 18.0, th));
            // number
            painter.text(Pos2::new(r.min.x + 6.0, r.min.y + 10.0), Align2::CENTER_CENTER, (i + 1).to_string(), style::bold(12.0), if selected { ACCENT } else { p.weak });
            // thumbnail
            let bg = style::c32(app.editor.theme().color(&"theme:background".into()));
            painter.rect_filled(img_r, 6.0, bg);
            if let Some(tex) = app.thumbs.get(i) {
                painter.image(tex.id(), img_r.shrink(1.0), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), if app.thumbs.stale(app.editor.revision) { Color32::from_white_alpha(200) } else { Color32::WHITE });
            } else {
                painter.text(img_r.center(), Align2::CENTER_CENTER, "…", FontId::proportional(14.0), p.weak);
            }
            let stroke = if selected { Stroke::new(2.0, ACCENT) } else if resp.hovered() { Stroke::new(1.0, ACCENT.gamma_multiply(0.6)) } else { Stroke::new(1.0, p.border) };
            painter.rect_stroke(img_r, 6.0, stroke, egui::StrokeKind::Inside);
            // label
            let label = crate::files::short(&step_label(app, i), 26);
            painter.text(Pos2::new(img_r.min.x + 2.0, img_r.max.y + 11.0), Align2::LEFT_CENTER, label, FontId::proportional(12.0), p.text);
            let step = &app.editor.doc.path[i];
            let mut badge_x = img_r.max.x - 4.0;
            if !step.builds.is_empty() {
                let t = format!("+{} fade", step.builds.len());
                painter.text(Pos2::new(badge_x, img_r.max.y + 11.0), Align2::RIGHT_CENTER, &t, FontId::proportional(10.5), p.weak);
                badge_x -= 52.0;
            }
            if !step.notes.is_empty() {
                painter.text(Pos2::new(badge_x, img_r.max.y + 11.0), Align2::RIGHT_CENTER, "notes", FontId::proportional(10.5), p.weak);
            }
            // delete × on hover
            if resp.hovered() && dragging.is_none() {
                let xr = Rect::from_center_size(img_r.right_top() + EVec2::new(-12.0, 12.0), EVec2::splat(18.0));
                painter.circle_filled(xr.center(), 9.0, Color32::from_black_alpha(140));
                painter.text(xr.center(), Align2::CENTER_CENTER, "✕", FontId::proportional(10.0), Color32::WHITE);
                if resp.clicked() && pointer.is_some_and(|pp| xr.contains(pp)) {
                    app.apply(Command::RemoveStep { index: i });
                    app.selected_step = None;
                    return;
                }
            }
            if resp.clicked() {
                app.selected_step = Some(i);
                let _ = app.editor.view(ViewCommand::GotoStep { step: i }, H);
            }
            if resp.drag_started() {
                dragging = Some(i);
            }
            let resp = resp.on_hover_text("Click to fly there · drag to reorder · right-click for more");
            egui::Popup::context_menu(&resp).show(|ui| {
                if ui.button("Go to step").clicked() {
                    let _ = app.editor.view(ViewCommand::GotoStep { step: i }, H);
                }
                if ui.button("Present from here").clicked() {
                    let _ = app.editor.view(ViewCommand::Present { step: Some(i) }, H);
                }
                if ui.button("Select target").clicked() {
                    if let StepTarget::Element { id } = &app.editor.doc.path[i].target {
                        app.select(vec![id.clone()]);
                    }
                }
                if ui.button("Duplicate step").clicked() {
                    let s = app.editor.doc.path[i].clone();
                    app.apply(Command::AddStep { target: s.target, index: Some(i + 1), builds: vec![], notes: s.notes });
                }
                if ui.button("Move to start").clicked() && i > 0 {
                    app.apply(Command::MoveStep { from: i, to: 0 });
                }
                if ui.button("Move to end").clicked() && i + 1 < n {
                    app.apply(Command::MoveStep { from: i, to: n - 1 });
                }
                ui.separator();
                if ui.button("Remove from path").clicked() {
                    app.apply(Command::RemoveStep { index: i });
                }
            });
            ui.add_space(6.0);
        }
    });

    // Drag-to-reorder
    if let Some(from) = dragging {
        if let Some(pp) = pointer {
            let mut to = rows.len().saturating_sub(1);
            for (k, r) in rows.iter().enumerate() {
                if pp.y < r.center().y {
                    to = if k > from { k - 1 } else { k };
                    break;
                }
            }
            drop_to = Some(to);
            // insertion marker
            if let Some(r) = rows.get(if to >= from { (to + 1).min(rows.len()) } else { to }).or(rows.last()) {
                let y = if to >= from && to + 1 >= rows.len() { r.max.y + 3.0 } else { r.min.y - 3.0 };
                ui.painter().line_segment([Pos2::new(r.min.x + 16.0, y), Pos2::new(r.max.x, y)], Stroke::new(2.5, ACCENT));
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(to) = drop_to {
                if to != from && to < n {
                    app.apply(Command::MoveStep { from, to });
                    app.selected_step = Some(to);
                }
            }
            dragging = None;
        }
    }
    ui.ctx().data_mut(|d| match dragging {
        Some(v) => {
            d.insert_temp(drag_id, v);
        }
        None => {
            d.remove::<usize>(drag_id);
        }
    });

    // Footer
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.horizontal(|ui| {
            if ui.small_button("Path from frames").on_hover_text("Replace the path with every frame, top-left to bottom-right").clicked() {
                app.queued.push(Action::AutoPath);
            }
            if n > 0 && ui.small_button("Clear").clicked() {
                app.queued.push(Action::ClearPath);
            }
        });
        let has_sel = !app.editor.view.selection.is_empty();
        let label = if has_sel { "+ Add selection" } else { "+ Add current view" };
        let b = egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(ACCENT).corner_radius(8).min_size(EVec2::new(ui.available_width(), 32.0));
        if ui.add(b).on_hover_text("⇧⌘A").clicked() {
            app.queued.push(Action::AddToPath);
        }
    });
}
