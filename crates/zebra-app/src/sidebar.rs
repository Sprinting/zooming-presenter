//! Path sidebar: an opaque source list of numbered step thumbnails (Keynote's
//! navigator), with the classic + / − list footer.
//!
//! Two highlight states (critique item 12):
//! - **selected**: the steps picked in the list, or those showing what is
//!   selected on the canvas. In the accent while the list has the keys,
//!   grey when the canvas has them (the macOS focus look);
//! - **current** (light plate): the step last visited or presented.
//!
//! Keys go to the pane clicked last, as in Keynote's navigator (spec 08):
//! clicking a step gives the list the keys, clicking the canvas gives them
//! back. With the list focused, Up/Down and Home/End move the selected step,
//! Shift+Up/Down grow a selection of several steps (one made by ⇧- or
//! ⌘-click), Delete takes the selected steps out of the path, Escape hands
//! the keys back. The step selection is by step number, apart from the
//! canvas selection: a frame used by two steps lights only the one picked.
//!
//! Insert between steps: hovering the gap between two rows shows an
//! insertion line with a +. It inserts an empty row that waits for a frame:
//! the next frame clicked or drawn on the canvas fills it, as one undo step.
//! Escape, a click on empty canvas or elsewhere in the sidebar cancels it.
//! The empty row is never part of the document.
use egui::{Align2, Color32, CursorIcon, Modifiers, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2 as EVec2};
use zebra_core::{Command, ElementId, Mode, Origin, StepTarget, ViewCommand};

use crate::actions::Action;
use crate::app::ZebraApp;
use crate::icons::{paint_icon, Icon};
use crate::style::{self, accent, Palette};
use crate::thumbs::{THUMB_H, THUMB_W};
use crate::widgets as w;

const H: Origin = Origin::Human;

/// Several steps picked in the list (by number, sorted), the end a Shift
/// range grows from, and the document revision they were picked at: any
/// edit can renumber the steps, so it ends them.
#[derive(Clone, Debug)]
pub struct StepMulti {
    pub steps: Vec<usize>,
    pub anchor: usize,
    pub revision: u64,
}

/// An empty step waiting for a frame, shown as its own row before step
/// `index`. Interface state only: never saved, never seen by agents.
#[derive(Clone, Debug)]
pub struct Pick {
    pub index: usize,
    /// The path as it was when the insert began: if it changes (an agent,
    /// Undo) the place no longer means what it did, and the insert ends.
    path_len: usize,
    loads: u64,
    /// The list has scrolled the empty row into view.
    shown: bool,
}

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

/// The selection of several steps, while it still holds.
fn multi(app: &ZebraApp) -> Option<&StepMulti> {
    let n = app.editor.doc.path.len();
    app.step_multi.as_ref().filter(|m| {
        m.revision == app.editor.revision && m.anchor < n && m.steps.iter().all(|s| *s < n) && app.selected_step.is_some_and(|s| m.steps.contains(&s))
    })
}

/// The steps picked in the list, by number (the canvas selection aside).
pub fn step_selection(app: &ZebraApp) -> Vec<usize> {
    if let Some(m) = multi(app) {
        return m.steps.clone();
    }
    app.selected_step.filter(|s| *s < app.editor.doc.path.len()).into_iter().collect()
}

/// Steps highlighted: those zooming to the canvas selection, else the
/// steps picked in the list.
pub fn selected_steps(app: &ZebraApp) -> Vec<usize> {
    let sel = &app.editor.view.selection;
    if !sel.is_empty() {
        let mut v: Vec<usize> = sel.iter().flat_map(|id| app.editor.doc.steps_using(id)).collect();
        v.sort();
        v.dedup();
        return v;
    }
    step_selection(app)
}

/// The path list has the keys: it was clicked last (not the canvas), it
/// is on screen, and nothing is selected on the canvas.
pub fn list_focused(app: &ZebraApp) -> bool {
    app.path_focus && app.show_sidebar && app.editor.view.mode != Mode::Present && app.editor.view.selection.is_empty() && !app.editor.doc.path.is_empty()
}

/// Select one step and fly to it, as a click on its row does.
pub fn select_step(app: &mut ZebraApp, i: usize) {
    app.selected_step = Some(i);
    app.step_multi = None;
    if !app.editor.view.selection.is_empty() {
        let _ = app.editor.view(ViewCommand::ClearSelection, H);
    }
    let _ = app.editor.view(ViewCommand::GotoStep { step: i }, H);
}

/// A click on step `i`'s row: ⇧ selects the range from the anchor, ⌘ adds
/// or takes out one step, a plain click selects just that one.
pub fn click_step(app: &mut ZebraApp, i: usize, m: Modifiers) {
    app.path_focus = true;
    let n = app.editor.doc.path.len();
    let revision = app.editor.revision;
    let anchor = multi(app).map(|m| m.anchor).or(app.selected_step).filter(|a| *a < n && app.editor.view.selection.is_empty());
    if m.shift && anchor.is_some() {
        let a = anchor.unwrap_or(i);
        app.step_multi = Some(StepMulti { steps: (a.min(i)..=a.max(i)).collect(), anchor: a, revision });
        app.selected_step = Some(i);
    } else if m.command {
        let mut steps = if app.editor.view.selection.is_empty() { step_selection(app) } else { vec![] };
        if let Some(k) = steps.iter().position(|s| *s == i).filter(|_| steps.len() > 1) {
            // Taken out: the list stays on a step still selected.
            steps.remove(k);
            let keep = steps[k.min(steps.len() - 1)];
            app.step_multi = Some(StepMulti { steps, anchor: keep, revision });
            app.selected_step = Some(keep);
            return;
        }
        if !steps.contains(&i) {
            steps.push(i);
            steps.sort_unstable();
        }
        app.step_multi = Some(StepMulti { steps, anchor: i, revision });
        app.selected_step = Some(i);
    } else {
        app.step_multi = None;
        app.selected_step = Some(i);
    }
    if !app.editor.view.selection.is_empty() {
        let _ = app.editor.view(ViewCommand::ClearSelection, H);
    }
    let _ = app.editor.view(ViewCommand::GotoStep { step: i }, H);
}

/// A key pressed while the list has the keys. True when the list used it.
/// Delete and Escape come as actions (so the menus agree), not here.
pub fn key(app: &mut ZebraApp, key: egui::Key, m: Modifiers) -> bool {
    use egui::Key;
    let n = app.editor.doc.path.len();
    if n == 0 || m.command || m.alt || m.ctrl || m.mac_cmd {
        return false;
    }
    match key {
        Key::ArrowUp | Key::ArrowDown => {
            let down = key == Key::ArrowDown;
            let from = app.selected_step.filter(|s| *s < n);
            let to = match from {
                None => 0,
                Some(c) if down => (c + 1).min(n - 1),
                Some(c) => c.saturating_sub(1),
            };
            if m.shift {
                // Grows a selection of several steps; with one step it
                // does nothing (Shift isn't a way to start one).
                if let Some(anchor) = multi(app).map(|m| m.anchor) {
                    let revision = app.editor.revision;
                    app.step_multi = Some(StepMulti { steps: (anchor.min(to)..=anchor.max(to)).collect(), anchor, revision });
                    app.selected_step = Some(to);
                    let _ = app.editor.view(ViewCommand::GotoStep { step: to }, H);
                }
                return true;
            }
            select_step(app, to);
            true
        }
        Key::Home | Key::End if !m.shift => {
            select_step(app, if key == Key::Home { 0 } else { n - 1 });
            true
        }
        _ => false,
    }
}

/// Take steps out of the path (never their frames), as one undo step;
/// the step after them is selected.
pub fn remove_steps(app: &mut ZebraApp, steps: &[usize]) {
    let Some(first) = steps.iter().min().copied() else { return };
    let mut steps = steps.to_vec();
    steps.sort_unstable();
    steps.dedup();
    let cmds: Vec<Command> = steps.iter().rev().map(|i| Command::RemoveStep { index: *i }).collect();
    let r = app.editor.apply_batch(cmds, Some(crate::t!("sidebar-undo-remove-step", count = steps.len())), H);
    if app.report(r).is_some() {
        app.selected_step = step_after_removal(first, app.editor.doc.path.len());
        app.step_multi = None;
    }
}

/// Edit › Delete (and ⌫) with the list focused.
pub fn delete_selected(app: &mut ZebraApp) {
    let sel = step_selection(app);
    remove_steps(app, &sel);
    if let Some(s) = app.selected_step {
        let _ = app.editor.view(ViewCommand::GotoStep { step: s }, H);
    }
}

/// Edit › Select All with the list focused: every step.
pub fn select_all_steps(app: &mut ZebraApp) {
    let n = app.editor.doc.path.len();
    if n == 0 {
        return;
    }
    let cur = app.selected_step.filter(|s| *s < n).unwrap_or(0);
    app.selected_step = Some(cur);
    app.step_multi = Some(StepMulti { steps: (0..n).collect(), anchor: cur, revision: app.editor.revision });
}

/// Start inserting a step before step `index` (or at the end): an empty
/// row waits for a frame.
pub fn start_pick(app: &mut ZebraApp, index: usize) {
    let n = app.editor.doc.path.len();
    app.step_pick = Some(Pick { index: index.min(n), path_len: n, loads: app.editor.loads, shown: false });
    app.step_multi = None;
    app.path_focus = true;
}

/// Drop the empty row; nothing was changed, so there is nothing to undo.
pub fn cancel_pick(app: &mut ZebraApp) {
    app.step_pick = None;
}

/// Once a frame: something selected on the canvas (a toolbar insert, an
/// undo) takes the keys from the list, so a later Escape doesn't hand them
/// back to it. And an insert ends when its place no longer means what it
/// did, or it can't be finished from here.
pub fn check_focus(app: &mut ZebraApp) {
    if !app.editor.view.selection.is_empty() {
        app.path_focus = false;
    }
    check_pick(app);
}

fn check_pick(app: &mut ZebraApp) {
    if let Some(pk) = &app.step_pick {
        if pk.path_len != app.editor.doc.path.len() || pk.loads != app.editor.loads || app.editor.view.mode == Mode::Present || !app.show_sidebar {
            app.step_pick = None;
        }
    }
}

/// Fill the empty row with a frame that is already there.
pub fn fill_pick(app: &mut ZebraApp, id: ElementId) -> bool {
    let Some(pk) = app.step_pick.take() else { return false };
    let cmd = Command::AddStep { target: StepTarget::Element { id }, index: Some(pk.index), builds: vec![], notes: String::new() };
    let r = app.editor.apply_batch(vec![cmd], Some(crate::t!("sidebar-undo-insert-step")), H);
    if app.report(r).is_none() {
        return false;
    }
    app.selected_step = Some(pk.index);
    app.step_multi = None;
    true
}

/// A frame drawn (or inserted) while a row waits: the frame and its step
/// are one undo step. `cmd` adds the frame.
pub fn add_frame_for_pick(app: &mut ZebraApp, cmd: Command) -> Option<zebra_core::Outcome> {
    let Some(pk) = app.step_pick.take() else { return app.apply(cmd) };
    if let Err(e) = app.editor.begin(crate::t!("sidebar-undo-insert-step"), H) {
        app.report::<()>(Err(e));
        return None;
    }
    let r = app.editor.apply(cmd, H).and_then(|out| {
        let id = out.created.first().cloned().ok_or_else(|| zebra_core::ZError::new("internal", "no frame was made"))?;
        app.editor.apply(Command::AddStep { target: StepTarget::Element { id }, index: Some(pk.index), builds: vec![], notes: String::new() }, H)?;
        Ok(out)
    });
    match r {
        Ok(out) => {
            app.editor.commit_as(&H);
            app.selected_step = Some(pk.index);
            app.step_multi = None;
            Some(out)
        }
        Err(e) => {
            app.editor.abort_as(&H);
            app.report::<()>(Err(e));
            None
        }
    }
}

/// The frame of the canvas selection, for Add Step After This One.
fn selected_frame(app: &ZebraApp) -> Option<ElementId> {
    app.top_level(&app.selection()).into_iter().find(|id| app.editor.doc.get(id).is_some_and(|e| e.is_frame()))
}

pub fn ui(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    check_pick(app);
    let had_pick = app.step_pick.is_some();
    let side = ui.max_rect();
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
    check_pick(app);
    let n = app.editor.doc.path.len();
    let selected = selected_steps(app);
    let focused = list_focused(app);
    let current = if app.editor.view.mode == Mode::Present { app.editor.view.present.step } else { app.selected_step };
    let drag_id = app.uid("path-drag");
    // The row being dragged, with the step it was when the drag began: if
    // the path changes under the drag (an agent, Undo), the drag ends
    // rather than move whichever step has that number now.
    let held: Option<(usize, zebra_core::PathStep)> = ui.ctx().data(|d| d.get_temp(drag_id));
    let mut dragging: Option<usize> = held.as_ref().filter(|(i, was)| app.editor.doc.path.get(*i) == Some(was)).map(|(i, _)| *i);
    let mut rows: Vec<Rect> = vec![];
    let mut empty_row: Option<Rect> = None;
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
    // The empty row of an insert takes a place of its own in the list.
    let gap_at = app.step_pick.as_ref().map(|pk| pk.index);
    let slots = n + gap_at.is_some() as usize;
    let mut insert_at: Option<usize> = None;

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
        let num_w = number_column_width(ui, slots);
        app.step_number_w = num_w;
        let th = (tw - num_w - 6.0) * THUMB_H as f32 / THUMB_W as f32;
        let row_h = th + 26.0;
        // As the rows were spaced when each was allocated in turn.
        let pitch = row_h + 2.0 + ui.spacing().item_spacing.y;
        let origin = ui.max_rect().min;
        let slot_rect = |k: usize| Rect::from_min_size(origin + EVec2::new(0.0, k as f32 * pitch), EVec2::new(tw, row_h));
        // Step i's place in the list, after the empty row if there is one.
        let slot = |i: usize| i + gap_at.is_some_and(|g| i >= g) as usize;
        rows = (0..n).map(|i| slot_rect(slot(i))).collect();
        empty_row = gap_at.map(slot_rect);
        ui.allocate_space(EVec2::new(tw, slots as f32 * pitch));
        if let Some(pk) = app.step_pick.as_mut().filter(|pk| !pk.shown) {
            pk.shown = true;
            if let Some(r) = empty_row {
                ui.scroll_to_rect(r, None);
            }
        } else if let Some(i) = scroll_to.filter(|i| *i < n) {
            ui.scroll_to_rect(rows[i], None);
        }
        let first = ((view.min.y / pitch).floor().max(0.0) as usize).min(slots);
        let last = ((view.max.y / pitch).ceil().max(0.0) as usize).min(slots);
        // The steps in those places.
        let step_of = |k: usize| match gap_at {
            Some(g) if k > g => k - 1,
            _ => k,
        };
        let (fs, ls) = (step_of(first).min(n), step_of(last).min(n));
        app.thumbs.show(fs..ls);
        // The row being dragged stays live when scrolled out of view.
        let extra = dragging.filter(|d| !(fs..ls).contains(d));
        let row = StepRow {
            tw,
            num_w,
            th,
            bg: style::c32(app.editor.theme().color(&"theme:background".into())),
            selected: &selected,
            focused,
            current,
            flash,
            picking: gap_at.is_some(),
        };
        for i in (fs..ls).chain(extra) {
            // A row's menu can remove or add steps: the rows after it wait
            // for the next frame instead of reading steps that moved.
            if app.editor.doc.path.len() != n {
                ui.ctx().request_repaint();
                break;
            }
            row.ui(app, ui, p, i, slot(i) + 1, rows[i], n, &mut dragging);
        }
        if let (Some(r), Some(g)) = (empty_row, gap_at) {
            if ui.is_rect_visible(r) {
                empty_step_row(ui, app.uid("path-empty-step"), p, r, num_w, th, g + 1);
            }
        }
        // Insert between steps: the gap nearest the pointer shows an
        // insertion line with a + (Keynote's navigator).
        let idle = dragging.is_none() && gap_at.is_none() && app.editor.doc.path.len() == n && n >= 2;
        if let Some(pp) = pointer.filter(|pp| idle && ui.clip_rect().contains(*pp) && pp.x >= origin.x && pp.x <= origin.x + tw) {
            let k = ((pp.y - origin.y) / pitch).round();
            if k >= 1.0 && (k as usize) < n {
                let k = k as usize;
                let y = (rows[k - 1].max.y + rows[k].min.y) / 2.0;
                if (pp.y - y).abs() <= 7.0 {
                    let c = Pos2::new(origin.x + num_w / 2.0 + 1.0, y);
                    let tip = crate::t!("sidebar-insert-step-tip");
                    let resp = ui.interact(Rect::from_center_size(c, EVec2::new(20.0, 16.0)), app.uid("path-insert"), Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
                    let painter = ui.painter();
                    let col = if resp.is_pointer_button_down_on() { accent().gamma_multiply(0.8) } else { accent() };
                    painter.line_segment([Pos2::new(c.x + 7.0, y), Pos2::new(origin.x + tw - 2.0, y)], Stroke::new(2.0, col));
                    painter.circle_filled(c, 6.0, col);
                    let s = Stroke::new(1.5, w::on_accent());
                    painter.line_segment([c - EVec2::new(3.0, 0.0), c + EVec2::new(3.0, 0.0)], s);
                    painter.line_segment([c - EVec2::new(0.0, 3.0), c + EVec2::new(0.0, 3.0)], s);
                    if resp.on_hover_text(tip).clicked() {
                        insert_at = Some(k);
                    }
                }
            }
        }
    });
    if let Some(k) = insert_at {
        start_pick(app, k);
        ui.ctx().request_repaint();
    }
    // A press anywhere else in the sidebar cancels a waiting insert.
    if had_pick && app.step_pick.is_some() {
        let press = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
        if press.is_some_and(|pp| side.contains(pp) && !empty_row.is_some_and(|r| r.contains(pp))) {
            cancel_pick(app);
        }
    }

    // Drag to reorder: an accent insertion line between rows.
    if let Some(from) = dragging {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
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
                    app.step_multi = None;
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

/// The empty row of an insert: a dashed place for the frame to come.
fn empty_step_row(ui: &mut Ui, id: egui::Id, p: &Palette, r: Rect, num_w: f32, th: f32, number: usize) {
    let resp = ui.interact(r, id, Sense::hover());
    let painter = ui.painter();
    let plate = r.shrink2(EVec2::new(0.0, 1.0));
    painter.rect_filled(plate, 6.0, p.hover);
    painter.text(Pos2::new(r.min.x + num_w / 2.0 + 1.0, r.min.y + 12.0), Align2::CENTER_CENTER, number.to_string(), style::semibold(11.0), p.weak);
    let img_r = Rect::from_min_size(r.min + EVec2::new(num_w, 5.0), EVec2::new(r.width() - num_w - 6.0, th));
    let q = [img_r.left_top(), img_r.right_top(), img_r.right_bottom(), img_r.left_bottom(), img_r.left_top()];
    painter.extend(egui::Shape::dashed_line(&q, Stroke::new(1.5, accent()), 5.0, 4.0));
    let text = crate::t!("sidebar-pick-a-frame");
    let icon = Rect::from_center_size(img_r.center() - EVec2::new(0.0, 10.0), EVec2::splat(18.0));
    paint_icon(painter, icon, Icon::Frame(zebra_core::FrameStyle::Rectangle), accent());
    painter.text(img_r.center() + EVec2::new(0.0, 12.0), Align2::CENTER_CENTER, &text, style::ui_font(11.0), p.text);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListItem, true, &text));
    resp.on_hover_text(crate::t!("sidebar-pick-a-frame-tip"));
}

/// What every step row needs to know, worked out once per frame.
struct StepRow<'a> {
    tw: f32,
    num_w: f32,
    th: f32,
    bg: Color32,
    selected: &'a [usize],
    /// The list has the keys: selected rows in the accent, else grey.
    focused: bool,
    current: Option<usize>,
    flash: Option<(usize, f32)>,
    /// An insert waits for a frame: rows don't drag.
    picking: bool,
}

impl StepRow<'_> {
    /// Step `i`'s row in `r`, shown as number `number`: plate, number,
    /// thumbnail, label and badges, click, drag and its context menu.
    #[allow(clippy::too_many_arguments)]
    fn ui(&self, app: &mut ZebraApp, ui: &mut Ui, p: &Palette, i: usize, number: usize, r: Rect, n: usize, dragging: &mut Option<usize>) {
        let (tw, num_w, th) = (self.tw, self.num_w, self.th);
        let sense = if self.picking { Sense::click() } else { Sense::click_and_drag() };
        let resp = ui.interact(r, app.uid("path-step").with(i), sense);
        let is_sel = self.selected.contains(&i);
        let lit = is_sel && self.focused;
        let is_cur = self.current == Some(i);
        if ui.is_rect_visible(r) {
            let painter = ui.painter();
            let plate = r.shrink2(EVec2::new(0.0, 1.0));
            if lit {
                painter.rect_filled(plate, 6.0, accent());
            } else if is_sel {
                // Selected, but the keys are with the canvas: grey.
                painter.rect_filled(plate, 6.0, p.pressed);
            } else if is_cur || (resp.hovered() && dragging.is_none()) {
                painter.rect_filled(plate, 6.0, p.hover);
            }
            if let Some((fi, a)) = self.flash {
                if fi <= i {
                    painter.rect_stroke(plate, 6.0, Stroke::new(2.0, accent().gamma_multiply(a)), egui::StrokeKind::Inside);
                }
            }
            let tc = if lit { w::on_accent() } else { p.text };
            let wc = if lit { w::on_accent() } else { p.weak };
            // Step number, top-left like Keynote's slide numbers.
            painter.text(Pos2::new(r.min.x + num_w / 2.0 + 1.0, r.min.y + 12.0), Align2::CENTER_CENTER, number.to_string(), style::semibold(11.0), tc);
            // Thumbnail.
            let img_r = Rect::from_min_size(r.min + EVec2::new(num_w, 5.0), EVec2::new(tw - num_w - 6.0, th));
            painter.rect_filled(img_r, 3.0, self.bg);
            if let Some((tex, fresh)) = app.thumbs.get(i) {
                let tint = if fresh && !app.thumbs.stale(app.editor.revision) { Color32::WHITE } else { Color32::from_white_alpha(200) };
                painter.image(tex.id(), img_r.shrink(0.5), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
            }
            painter.rect_stroke(img_r, 3.0, Stroke::new(1.0, if lit { Color32::from_white_alpha(160) } else { p.border }), egui::StrokeKind::Outside);
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
        // Rows drag to reorder: an open hand says so (Kartik, 18 Nov).
        if resp.hovered() && dragging.is_none() && !self.picking {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            click_step(app, i, m);
        }
        if resp.drag_started() && !self.picking {
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
                    app.path_focus = false;
                }
            }
            if matches!(&app.editor.doc.path[i].target, StepTarget::Overview) {
                let hide = app.editor.doc.path[i].hide_builds;
                if w::menu_check(ui, !hide, &crate::t!("sidebar-show-fade-ins-dimmed"), None).clicked() {
                    app.apply(Command::UpdateStep { index: i, target: None, builds: None, notes: None, duration: None, dwell: None, hide_builds: Some(!hide) });
                }
            }
            w::menu_separator(ui);
            // The frame selected on the canvas, else an empty step that
            // waits for one to be clicked or drawn.
            if w::menu_item(ui, &crate::t!("sidebar-add-step-after"), None).clicked() {
                match selected_frame(app) {
                    Some(id) => {
                        let cmd = Command::AddStep { target: StepTarget::Element { id }, index: Some(i + 1), builds: vec![], notes: String::new() };
                        if app.apply(cmd).is_some() {
                            app.selected_step = Some(i + 1);
                            app.step_multi = None;
                        }
                    }
                    None => start_pick(app, i + 1),
                }
            }
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
                remove_steps(app, &[i]);
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
            remove_steps(app, &sel);
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
