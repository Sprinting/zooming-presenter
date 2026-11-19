//! Path sidebar: an opaque source list of numbered step thumbnails (Keynote's
//! navigator), with the classic + / − list footer.
//!
//! Two highlight states (critique item 12):
//! - **selected** (accent plate): the steps that show what is selected on
//!   the canvas, or the step clicked last when nothing is selected;
//! - **current** (grey plate): the step last visited or presented.
use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2 as EVec2};
use zebra_core::{Command, Mode, Origin, StepTarget, ViewCommand};

use crate::actions::Action;
use crate::app::ZebraApp;
use crate::icons::{paint_icon, Icon};
use crate::style::{self, accent, Palette};
use crate::thumbs::{THUMB_H, THUMB_W};
use crate::widgets as w;

const H: Origin = Origin::Human;

pub fn step_label(app: &ZebraApp, i: usize) -> String {
    let doc = &app.editor.doc;
    match &doc.path[i].target {
        StepTarget::Element { id } => doc
            .get(id)
            .map(|e| match app.editor.own_label(e) {
                Some(l) => l,
                None if e.is_frame() => crate::t!("sidebar-untitled-frame"),
                None => crate::words::type_name(&e.kind),
            })
            .unwrap_or_else(|| crate::t!("sidebar-missing-step")),
        StepTarget::View { .. } => crate::t!("sidebar-step-view"),
        StepTarget::Overview => crate::t!("sidebar-step-overview"),
    }
}

/// The step to highlight once the one at `removed` is gone: the one that
/// took its place, else the new last one (as Keynote does with slides;
/// leaving none highlighted, the notes still said "step 1").
pub fn step_after_removal(removed: usize, left: usize) -> Option<usize> {
    (left > 0).then(|| removed.min(left - 1))
}

/// Steps highlighted with the accent: those zooming to the canvas
/// selection, else the step clicked last.
pub fn selected_steps(app: &ZebraApp) -> Vec<usize> {
    let sel = &app.editor.view.selection;
    if !sel.is_empty() {
        let mut v: Vec<usize> = sel.iter().flat_map(|id| app.editor.doc.steps_using(id)).collect();
        v.sort();
        v.dedup();
        return v;
    }
    app.selected_step.filter(|s| *s < app.editor.doc.path.len()).into_iter().collect()
}

pub fn ui(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let n = app.editor.doc.path.len();
    let editing = app.editor.view.mode == Mode::PathEdit;

    // Header: source-list section title, step count, Edit Path / Done.
    ui.horizontal(|ui| {
        ui.label(RichText::new(crate::t!("sidebar-path")).font(style::semibold(11.0)).color(p.weak));
        if n > 0 {
            ui.label(RichText::new(crate::t!("sidebar-step-count", count = n)).font(style::ui_font(11.0)).color(p.weak));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = if editing {
                let done = crate::t!("sidebar-done");
                let (rect, resp) = ui.allocate_exact_size(EVec2::new(52.0, w::H_SMALL), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &done));
                ui.painter().rect_filled(rect, w::RADIUS, accent());
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, &done, style::ui_font(11.0), w::on_accent());
                resp
            } else {
                w::push_button_ext(ui, &crate::t!("sidebar-edit-path"), w::Size::Small, true, None)
            };
            let tip = if editing {
                crate::i18n::with_key(crate::t!("sidebar-done-tip"), egui::Modifiers::NONE, egui::Key::Escape)
            } else {
                crate::i18n::with_shortcut(crate::t!("sidebar-edit-path-tip"), &Action::TogglePathEdit)
            };
            if r.on_hover_text(tip).clicked() {
                app.queued.push(Action::TogglePathEdit);
            }
        });
    });
    ui.add_space(4.0);

    // Footer first, so the list takes exactly the space that is left.
    egui::Panel::bottom("path-footer").exact_size(34.0).frame(egui::Frame::new().fill(p.sidebar)).show(ui, |ui| footer(app, ui, p));
    // Speaker notes for the current step (stream 4), above the footer.
    if app.current_step().is_some() {
        egui::Panel::bottom("path-notes")
            .exact_size(118.0)
            .frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin::symmetric(8, 4)))
            .show(ui, |ui| crate::presenting::sidebar_notes(app, ui, p));
    }

    // The footer's − button may just have shortened the path: count again.
    let n = app.editor.doc.path.len();
    let selected = selected_steps(app);
    let current = if app.editor.view.mode == Mode::Present { app.editor.view.present.step } else { app.selected_step };
    let drag_id = app.uid("path-drag");
    // The row being dragged, with the step it was when the drag began: if
    // the path changes under the drag (an agent, Undo), the drag ends
    // rather than move whichever step has that number now.
    let held: Option<(usize, zebra_core::PathStep)> = ui.ctx().data(|d| d.get_temp(drag_id));
    let mut dragging: Option<usize> = held.as_ref().filter(|(i, was)| app.editor.doc.path.get(*i) == Some(was)).map(|(i, _)| *i);
    let mut rows: Vec<Rect> = vec![];
    let pointer = ui.ctx().pointer_hover_pos();
    // Scroll to a step the camera was sent to (by anyone: an agent's
    // goto_step too), else to the selected or current step when it changes.
    let last_id = app.uid("path-last-shown");
    let went = app.editor.went_to_step;
    let last: (Vec<usize>, Option<usize>, u64) = ui.ctx().data(|d| d.get_temp(last_id)).unwrap_or_default();
    let scroll_to = if went.0 != last.2 {
        Some(went.1)
    } else if selected != last.0 {
        selected.first().copied()
    } else if current != last.1 {
        current
    } else {
        None
    };
    ui.ctx().data_mut(|d| d.insert_temp(last_id, (selected.clone(), current, went.0)));
    let flash = app.flash_step.and_then(|(i, t)| {
        let age = app.now() - t;
        (age < 1.2).then_some((i, 1.0 - (age / 1.2) as f32))
    });
    if flash.is_some() {
        ui.ctx().request_repaint();
    } else {
        app.flash_step = None;
    }

    // Only the rows in view are laid out (and their thumbnails drawn): a
    // long path costs what a short one does. Rows are all one height, so
    // where each one is follows from its number.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, view| {
        if n == 0 {
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(EVec2::splat(30.0), Sense::hover());
                paint_icon(ui.painter(), r, Icon::Path, p.weak);
                ui.add_space(6.0);
                ui.label(RichText::new(crate::t!("sidebar-no-steps")).font(style::semibold(13.0)).color(p.text));
                ui.add_space(2.0);
                w::caption(ui, crate::t!("sidebar-no-steps-caption"));
            });
            app.thumbs.show(0..0);
            return;
        }
        let tw = ui.available_width();
        let num_w = number_column_width(ui, n);
        app.step_number_w = num_w;
        let th = (tw - num_w - 6.0) * THUMB_H as f32 / THUMB_W as f32;
        let row_h = th + 26.0;
        // As the rows were spaced when each was allocated in turn.
        let pitch = row_h + 2.0 + ui.spacing().item_spacing.y;
        let origin = ui.max_rect().min;
        rows = (0..n).map(|i| Rect::from_min_size(origin + EVec2::new(0.0, i as f32 * pitch), EVec2::new(tw, row_h))).collect();
        ui.allocate_space(EVec2::new(tw, n as f32 * pitch));
        if let Some(i) = scroll_to.filter(|i| *i < n) {
            ui.scroll_to_rect(rows[i], None);
        }
        let first = ((view.min.y / pitch).floor().max(0.0) as usize).min(n);
        let last = ((view.max.y / pitch).ceil().max(0.0) as usize).min(n);
        app.thumbs.show(first..last);
        // The row being dragged stays live when scrolled out of view.
        let extra = dragging.filter(|d| !(first..last).contains(d));
        let row = StepRow { tw, num_w, th, bg: style::c32(app.editor.theme().color(&"theme:background".into())), selected: &selected, current, flash };
        for i in (first..last).chain(extra) {
            // A row's menu can remove or add steps: the rows after it wait
            // for the next frame instead of reading steps that moved.
            if app.editor.doc.path.len() != n {
                ui.ctx().request_repaint();
                break;
            }
            row.ui(app, ui, p, i, rows[i], n, &mut dragging);
        }
    });

    // Drag to reorder: an accent insertion line between rows.
    if let Some(from) = dragging {
        let mut drop_to: Option<usize> = None;
        if let Some(pp) = pointer {
            let mut to = rows.len().saturating_sub(1);
            for (k, r) in rows.iter().enumerate() {
                if pp.y < r.center().y {
                    to = if k > from { k - 1 } else { k };
                    break;
                }
            }
            drop_to = Some(to);
            if let Some(r) = rows.get(if to >= from { (to + 1).min(rows.len()) } else { to }).or(rows.last()) {
                let y = if to >= from && to + 1 >= rows.len() { r.max.y + 1.0 } else { r.min.y - 1.0 };
                let painter = ui.painter();
                painter.line_segment([Pos2::new(r.min.x + 8.0, y), Pos2::new(r.max.x, y)], Stroke::new(2.0, accent()));
                painter.circle_stroke(Pos2::new(r.min.x + 5.0, y), 3.0, Stroke::new(2.0, accent()));
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(to) = drop_to {
                if to != from && to < app.editor.doc.path.len() && from < app.editor.doc.path.len() {
                    app.apply(Command::MoveStep { from, to });
                    app.selected_step = Some(to);
                }
            }
            dragging = None;
        }
    }
    app.step_rows = rows;
    let held = dragging.and_then(|i| Some((i, app.editor.doc.path.get(i)?.clone())));
    ui.ctx().data_mut(|d| match held {
        Some(v) => {
            d.insert_temp(drag_id, v);
        }
        None => {
            d.remove::<(usize, zebra_core::PathStep)>(drag_id);
        }
    });
}

/// The step number column: 22 points, or wider when the path's last
/// number needs it ("1198" used to run into the thumbnail).
pub fn number_column_width(ui: &Ui, n: usize) -> f32 {
    // Digits differ in width: as many of the widest as the last number has.
    let digit = |d: char| ui.painter().layout_no_wrap(d.to_string(), style::semibold(11.0), Color32::WHITE).size().x;
    let widest = ('0'..='9').map(digit).fold(0.0, f32::max) * n.max(1).to_string().len() as f32;
    (widest + 10.0).max(22.0).ceil()
}

/// What every step row needs to know, worked out once per frame.
struct StepRow<'a> {
    tw: f32,
    num_w: f32,
    th: f32,
    bg: Color32,
    selected: &'a [usize],
    current: Option<usize>,
    flash: Option<(usize, f32)>,
}

impl StepRow<'_> {
    /// Step `i`'s row in `r`: plate, number, thumbnail, label and badges,
    /// click, drag and its context menu.
    #[allow(clippy::too_many_arguments)]
    fn ui(&self, app: &mut ZebraApp, ui: &mut Ui, p: &Palette, i: usize, r: Rect, n: usize, dragging: &mut Option<usize>) {
        let (tw, num_w, th) = (self.tw, self.num_w, self.th);
        let resp = ui.interact(r, app.uid("path-step").with(i), Sense::click_and_drag());
        let is_sel = self.selected.contains(&i);
        let is_cur = self.current == Some(i);
        if ui.is_rect_visible(r) {
            let painter = ui.painter();
            let plate = r.shrink2(EVec2::new(0.0, 1.0));
            if is_sel {
                painter.rect_filled(plate, 6.0, accent());
            } else if is_cur {
                painter.rect_filled(plate, 6.0, p.pressed);
            } else if resp.hovered() && dragging.is_none() {
                painter.rect_filled(plate, 6.0, p.hover);
            }
            if let Some((fi, a)) = self.flash {
                if fi <= i {
                    painter.rect_stroke(plate, 6.0, Stroke::new(2.0, accent().gamma_multiply(a)), egui::StrokeKind::Inside);
                }
            }
            let tc = if is_sel { w::on_accent() } else { p.text };
            let wc = if is_sel { w::on_accent() } else { p.weak };
            // Step number, top-left like Keynote's slide numbers.
            painter.text(Pos2::new(r.min.x + num_w / 2.0 + 1.0, r.min.y + 12.0), Align2::CENTER_CENTER, (i + 1).to_string(), style::semibold(11.0), tc);
            // Thumbnail.
            let img_r = Rect::from_min_size(r.min + EVec2::new(num_w, 5.0), EVec2::new(tw - num_w - 6.0, th));
            painter.rect_filled(img_r, 3.0, self.bg);
            if let Some((tex, fresh)) = app.thumbs.get(i) {
                let tint = if fresh && !app.thumbs.stale(app.editor.revision) { Color32::WHITE } else { Color32::from_white_alpha(200) };
                painter.image(tex.id(), img_r.shrink(0.5), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
            }
            painter.rect_stroke(img_r, 3.0, Stroke::new(1.0, if is_sel { Color32::from_white_alpha(160) } else { p.border }), egui::StrokeKind::Outside);
            // Label and badges.
            let step = &app.editor.doc.path[i];
            let mut right = img_r.max.x;
            let ly = img_r.max.y + 10.0;
            if !step.builds.is_empty() {
                let t = crate::t!("sidebar-build-count", count = step.builds.len());
                let g = painter.layout_no_wrap(t, style::ui_font(11.0), wc);
                right -= g.size().x;
                painter.galley(Pos2::new(right, ly - g.size().y / 2.0), g, wc);
                right -= 6.0;
            }
            if !step.notes.is_empty() {
                right -= 13.0;
                paint_icon(painter, Rect::from_min_size(Pos2::new(right, ly - 6.5), EVec2::splat(13.0)), Icon::Note, wc);
                right -= 4.0;
            }
            let label = step_label(app, i);
            let clip = Rect::from_min_max(Pos2::new(img_r.min.x, ly - 8.0), Pos2::new(right, ly + 8.0));
            painter.with_clip_rect(clip).text(Pos2::new(img_r.min.x, ly), Align2::LEFT_CENTER, &label, style::ui_font(11.0), tc);
        }
        if resp.clicked() {
            app.selected_step = Some(i);
            if !app.editor.view.selection.is_empty() {
                let _ = app.editor.view(ViewCommand::ClearSelection, H);
            }
            let _ = app.editor.view(ViewCommand::GotoStep { step: i }, H);
        }
        if resp.drag_started() {
            *dragging = Some(i);
        }
        let resp = if resp.hovered() { resp.on_hover_text(crate::t!("sidebar-step-tip", number = ((i + 1) as u64), label = step_label(app, i))) } else { resp };
        egui::Popup::context_menu(&resp).width(200.0).show(|ui| {
            if w::menu_item(ui, &crate::t!("sidebar-go-to-step"), None).clicked() {
                let _ = app.editor.view(ViewCommand::GotoStep { step: i }, H);
            }
            if w::menu_item(ui, &crate::t!("sidebar-play-from-here"), None).clicked() {
                let _ = app.editor.view(ViewCommand::Present { step: Some(i) }, H);
            }
            let has_target = matches!(&app.editor.doc.path[i].target, StepTarget::Element { .. });
            if w::menu_row(ui, None, &crate::t!("sidebar-select-on-canvas"), None, has_target).clicked() {
                if let StepTarget::Element { id } = &app.editor.doc.path[i].target {
                    app.select(vec![id.clone()]);
                }
            }
            if matches!(&app.editor.doc.path[i].target, StepTarget::Overview) {
                let hide = app.editor.doc.path[i].hide_builds;
                if w::menu_check(ui, !hide, &crate::t!("sidebar-show-fade-ins-dimmed"), None).clicked() {
                    app.apply(Command::UpdateStep { index: i, target: None, builds: None, notes: None, duration: None, dwell: None, hide_builds: Some(!hide) });
                }
            }
            w::menu_separator(ui);
            if w::menu_item(ui, &crate::t!("sidebar-duplicate-step"), None).clicked() {
                let s = app.editor.doc.path[i].clone();
                app.apply(Command::AddStep { target: s.target, index: Some(i + 1), builds: vec![], notes: s.notes });
            }
            if w::menu_row(ui, None, &crate::t!("sidebar-move-to-start"), None, i > 0).clicked() {
                app.apply(Command::MoveStep { from: i, to: 0 });
            }
            if w::menu_row(ui, None, &crate::t!("sidebar-move-to-end"), None, i + 1 < n).clicked() {
                app.apply(Command::MoveStep { from: i, to: n - 1 });
            }
            w::menu_separator(ui);
            if w::menu_item(ui, &crate::t!("sidebar-remove-from-path"), None).clicked() {
                app.apply(Command::RemoveStep { index: i });
                app.selected_step = step_after_removal(i, app.editor.doc.path.len());
            }
        });
    }
}

/// The classic list footer: + / − gradient buttons and an action pop-up.
fn footer(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    let r = ui.max_rect();
    ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, p.border));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let has_doc = !app.editor.doc.elements.is_empty();
        let add_tip = crate::i18n::with_shortcut(
            if app.editor.view.selection.is_empty() { crate::t!("sidebar-add-tip-view") } else { crate::t!("sidebar-add-tip-selection") },
            &Action::AddToPath,
        );
        if footer_button(ui, Icon::Plus, has_doc, p, &Action::AddToPath.label()).on_hover_text(add_tip).clicked() {
            app.queued.push(Action::AddToPath);
        }
        let sel = selected_steps(app);
        let rm_tip = crate::t!("sidebar-remove-tip");
        let rm = footer_button(ui, Icon::Minus, !sel.is_empty(), p, &rm_tip).on_hover_text(rm_tip.as_str());
        if rm.clicked() {
            let cmds: Vec<Command> = sel.iter().rev().map(|i| Command::RemoveStep { index: *i }).collect();
            let r = app.editor.apply_batch(cmds, Some(crate::t!("sidebar-undo-remove-step")), H);
            app.report(r);
            app.selected_step = sel.first().and_then(|i| step_after_removal(*i, app.editor.doc.path.len()));
        }
        ui.add_space(6.0);
        let more_tip = crate::t!("sidebar-more-tip");
        let more = footer_button(ui, Icon::ChevronDown, true, p, &more_tip).on_hover_text(more_tip.as_str());
        egui::Popup::menu(&more).width(200.0).show(|ui| {
            if w::menu_item(ui, &crate::t!("sidebar-path-from-frames"), None).clicked() {
                app.queued.push(Action::AutoPath);
            }
            if w::menu_item(ui, &crate::t!("sidebar-add-current-view"), None).clicked() {
                app.queued.push(Action::AddCurrentView);
            }
            w::menu_separator(ui);
            let n = app.editor.doc.path.len();
            if w::menu_row(ui, None, &crate::t!("sidebar-clear-path"), None, n > 0).clicked() {
                app.queued.push(Action::ClearPath);
            }
        });
    });
}

/// `label` names it for VoiceOver (translated).
fn footer_button(ui: &mut Ui, icon: Icon, enabled: bool, p: &Palette, label: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(EVec2::new(24.0, 21.0), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if enabled && resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 3.0, p.pressed);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, p.hover);
    }
    paint_icon(ui.painter(), Rect::from_center_size(rect.center(), EVec2::splat(11.0)), icon, if enabled { p.text } else { p.disabled });
    resp
}

#[cfg(test)]
mod removal_tests {
    use super::step_after_removal;

    #[test]
    fn removing_a_step_highlights_its_neighbour() {
        assert_eq!(step_after_removal(1, 2), Some(1), "the step that moved up");
        assert_eq!(step_after_removal(2, 2), Some(1), "the last one went: the new last");
        assert_eq!(step_after_removal(0, 0), None, "no steps left");
    }
}
