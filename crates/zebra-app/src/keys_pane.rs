//! Settings › Keyboard, laid out like the Mac's own Keyboard Shortcuts:
//! a search field, a list of groups on the left (the menu bar's menus, in
//! menu-bar order) and the group's commands on the right, each with its
//! shortcut. Click a shortcut, then press the new keys; Esc cancels.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::actions::Action;
use crate::app::ZebraApp;
use crate::icons::Icon;
use crate::keymap;
use crate::menus::Item;
use crate::style::{self, Palette};
use crate::widgets as w;

const SIDEBAR_W: f32 = 150.0;
const ROW_H: f32 = 24.0;
const KEYS_W: f32 = 120.0;

/// One group in the list: a menu's name and its rebindable commands.
#[derive(Debug, Clone)]
pub struct Group {
    pub name: String,
    pub actions: Vec<Action>,
}

/// Every rebindable command, grouped by the menu it lives in. Commands
/// that are in no menu (tool keys, picture and video commands) come last.
pub fn groups() -> Vec<Group> {
    let all = keymap::rebindable();
    let mut seen: Vec<Action> = vec![];
    let mut out: Vec<Group> = vec![];
    let mut add = |name: String, list: Vec<Action>, seen: &mut Vec<Action>| {
        let mut actions = vec![];
        for a in list {
            if all.contains(&a) && !seen.contains(&a) {
                seen.push(a.clone());
                actions.push(a);
            }
        }
        if !actions.is_empty() {
            out.push(Group { name, actions });
        }
    };
    add(
        crate::t!("menu-mac-app"),
        vec![Action::Settings, Action::About],
        &mut seen,
    );
    fn flatten(items: &[Item], out: &mut Vec<Action>) {
        for i in items {
            match i {
                Item::A(a) => out.push(a.clone()),
                Item::Sub(_, v) => flatten(v, out),
                Item::Sep | Item::Recent => {}
            }
        }
    }
    for (name, items) in crate::menus::spec() {
        let mut v = vec![];
        flatten(&items, &mut v);
        add(name, v, &mut seen);
    }
    let (tools, rest): (Vec<Action>, Vec<Action>) = all
        .iter()
        .filter(|a| !seen.contains(a))
        .cloned()
        .partition(|a| matches!(a, Action::SetTool(_) | Action::FrameTool));
    add(crate::t!("settings-keys-group-tools"), tools, &mut seen);
    add(crate::t!("settings-keys-group-other"), rest, &mut seen);
    out
}

/// What the pane remembers between frames.
#[derive(Clone, Default)]
struct PaneState {
    query: String,
    group: usize,
}

fn state_id() -> egui::Id {
    egui::Id::unique("zebra-keys-pane")
}

/// Debug demos: start with this search typed.
pub fn set_query(ctx: &egui::Context, q: &str) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<PaneState>(state_id()).query = q.to_string());
}

/// Debug demos: start with group `i` chosen.
pub fn set_group(ctx: &egui::Context, i: usize) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<PaneState>(state_id()).group = i);
}

/// Take the keys typed while a shortcut is being recorded. Returns true
/// when a recording ended this frame.
fn take_typed_keys(app: &mut ZebraApp, ui: &Ui) -> bool {
    let Some(cap) = &mut app.key_capture else {
        return false;
    };
    if cap.got.is_none() {
        // egui also reports ⌘, ⇧, ⌥ and ⌃ as keys of their own: wait for
        // the key they modify.
        cap.got = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !keymap::is_modifier(*key) => Some((*modifiers, *key)),
                _ => None,
            })
        });
    }
    let Some((m, k)) = cap.got else { return false };
    let a = cap.action.clone();
    app.key_capture = None;
    if k == egui::Key::Escape && m.is_none() {
        return true;
    }
    let keys = crate::actions::fmt_shortcut(m, k);
    match keymap::action_for(m, k) {
        Some(other) if other != a && !keymap::rebindable().contains(&other) => {
            app.key_message = Some(crate::t!(
                "settings-keys-taken",
                keys = keys.as_str(),
                action = other.label()
            ));
        }
        _ => {
            let before = keymap::set(&a, m, k);
            app.key_message = before.map(|b| {
                crate::t!(
                    "settings-keys-moved",
                    keys = keys.as_str(),
                    action = b.label()
                )
            });
        }
    }
    true
}

fn matches(q: &str, group: &Group, a: &Action) -> bool {
    q.is_empty() || a.label().to_lowercase().contains(q) || group.name.to_lowercase().contains(q)
}

pub fn show(app: &mut ZebraApp, ui: &mut Ui) {
    let p = Palette::of(ui.ctx());
    let recorded_now = take_typed_keys(app, ui);
    let mut st: PaneState = ui
        .ctx()
        .data_mut(|d| d.get_temp(state_id()))
        .unwrap_or_default();
    let groups = groups();
    st.group = st.group.min(groups.len().saturating_sub(1));

    // Recording keys for a command in another group: show that group.
    if let Some(cap) = &app.key_capture {
        if let Some(i) = groups.iter().position(|g| g.actions.contains(&cap.action)) {
            if st.query.is_empty() && i != st.group {
                st.group = i;
            }
        }
    }
    let search = w::search_field(
        ui,
        &mut st.query,
        ui.available_width(),
        &crate::t!("settings-keys-search"),
    );
    if search.changed() {
        app.key_capture = None;
    }
    let q = st.query.trim().to_lowercase();

    // The list box: sidebar of groups, then the commands.
    let footer_h = 30.0;
    let size = Vec2::new(
        ui.available_width(),
        (ui.available_height() - footer_h).max(120.0),
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        4.0,
        p.field,
        Stroke::new(if p.dark { 0.0 } else { 0.6 }, p.control_edge),
        StrokeKind::Inside,
    );
    let side = Rect::from_min_max(rect.min, egui::pos2(rect.left() + SIDEBAR_W, rect.bottom()));
    let list = Rect::from_min_max(egui::pos2(side.right() + 1.0, rect.top()), rect.max);
    ui.painter().rect_filled(
        side.shrink(0.5),
        egui::CornerRadius {
            nw: 4,
            sw: 4,
            ne: 0,
            se: 0,
        },
        p.sidebar,
    );
    ui.painter().line_segment(
        [side.right_top(), side.right_bottom()],
        Stroke::new(1.0, p.border),
    );

    // Groups.
    let mut side_outer =
        ui.new_child(egui::UiBuilder::new().max_rect(side.shrink2(Vec2::new(6.0, 6.0))));
    egui::ScrollArea::vertical()
        .id_salt("zebra-keys-groups")
        .auto_shrink([false, false])
        .show(&mut side_outer, |side_ui| {
            side_ui.spacing_mut().item_spacing.y = 1.0;
            for (i, g) in groups.iter().enumerate() {
                let hits = g.actions.iter().filter(|a| matches(&q, g, a)).count();
                let chosen = q.is_empty() && i == st.group;
                let (r, resp) = side_ui.allocate_exact_size(
                    Vec2::new(side_ui.available_width(), 22.0),
                    Sense::click(),
                );
                let resp = resp.on_hover_cursor(egui::CursorIcon::Default);
                if chosen {
                    side_ui.painter().rect_filled(r, 5.0, style::accent());
                } else if resp.hovered() {
                    side_ui.painter().rect_filled(r, 5.0, p.hover);
                }
                let color = if chosen {
                    w::on_accent()
                } else if hits == 0 {
                    p.disabled
                } else {
                    p.text
                };
                side_ui.painter().text(
                    r.left_center() + Vec2::new(8.0, 0.0),
                    Align2::LEFT_CENTER,
                    &g.name,
                    style::ui_font(13.0),
                    color,
                );
                if resp.clicked() {
                    st.group = i;
                    st.query.clear();
                    app.key_capture = None;
                }
            }
        });

    // Commands.
    let capturing = app.key_capture.as_ref().map(|c| c.action.clone());
    let held = ui.input(|i| i.modifiers);
    let mut list_ui =
        ui.new_child(egui::UiBuilder::new().max_rect(list.shrink2(Vec2::new(1.0, 4.0))));
    egui::ScrollArea::vertical()
        .id_salt("zebra-keys-list")
        .auto_shrink([false, false])
        .show(&mut list_ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut row = 0usize;
            let mut any = false;
            let shown: Vec<(usize, &Group)> = if q.is_empty() {
                groups
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i == st.group)
                    .collect()
            } else {
                groups.iter().enumerate().collect()
            };
            for (_, g) in shown {
                let acts: Vec<&Action> = g.actions.iter().filter(|a| matches(&q, g, a)).collect();
                if acts.is_empty() {
                    continue;
                }
                any = true;
                if !q.is_empty() {
                    // While searching, every group with a match, under its name.
                    let (r, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
                    ui.painter().text(
                        r.left_bottom() + Vec2::new(12.0, -5.0),
                        Align2::LEFT_BOTTOM,
                        &g.name,
                        style::semibold(11.0),
                        p.weak,
                    );
                    row = 0;
                }
                for a in acts {
                    command_row(
                        app,
                        ui,
                        a,
                        row,
                        capturing.as_ref() == Some(a),
                        held,
                        recorded_now,
                        &p,
                    );
                    row += 1;
                }
            }
            if !any {
                let r = ui.available_rect_before_wrap();
                ui.painter().text(
                    egui::pos2(r.center().x, r.top() + 60.0),
                    Align2::CENTER_CENTER,
                    crate::t!("settings-keys-no-results"),
                    style::ui_font(13.0),
                    p.weak,
                );
            }
        });

    // Footer: what just happened on the left, Restore Defaults on the right.
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let changed = !keymap::overrides().keys.is_empty();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if w::push_button_ext(
                ui,
                &crate::t!("settings-keys-restore"),
                w::Size::Regular,
                changed,
                None,
            )
            .clicked()
            {
                keymap::reset_all();
                app.key_capture = None;
                app.key_message = None;
            }
            // What just happened, else how to change a shortcut.
            let note = app
                .key_message
                .clone()
                .unwrap_or_else(|| crate::t!("settings-keys-caption"));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                w::caption(ui, note);
            });
        });
    });
    ui.ctx().data_mut(|d| d.insert_temp(state_id(), st));
}

/// One command: its name, then its shortcut (click to record new keys),
/// then a button that removes the shortcut.
#[allow(clippy::too_many_arguments)]
fn command_row(
    app: &mut ZebraApp,
    ui: &mut Ui,
    a: &Action,
    row: usize,
    recording: bool,
    held: egui::Modifiers,
    recorded_now: bool,
    p: &Palette,
) {
    let (r, row_resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
    let row_hovered = row_resp.hovered() || ui.rect_contains_pointer(r);
    if row % 2 == 1 {
        ui.painter().rect_filled(
            r,
            0.0,
            if p.dark {
                Color32::from_white_alpha(6)
            } else {
                Color32::from_black_alpha(8)
            },
        );
    }
    let painter = ui.painter();
    painter.text(
        r.left_center() + Vec2::new(12.0, 0.0),
        Align2::LEFT_CENTER,
        a.label(),
        style::ui_font(13.0),
        p.text,
    );

    // ✕ at the far right, while the pointer is on a row with keys.
    let x_rect = Rect::from_center_size(
        egui::pos2(r.right() - 18.0, r.center().y),
        Vec2::splat(16.0),
    );
    let has = keymap::binding_for(a).is_some();
    if has && row_hovered && !recording {
        let resp = ui
            .put(x_rect, |ui: &mut Ui| {
                w::icon_button(ui, Icon::Close, 16.0, true, false)
            })
            .on_hover_text(crate::t!("settings-keys-remove"));
        if resp.clicked() {
            keymap::clear(a);
            app.key_capture = None;
            app.key_message = None;
        }
    }

    // The shortcut: plain text, right-aligned; a field while recording.
    let keys_rect = Rect::from_min_size(
        egui::pos2(x_rect.left() - 6.0 - KEYS_W, r.top() + 2.0),
        Vec2::new(KEYS_W, ROW_H - 4.0),
    );
    let resp = ui.interact(
        keys_rect,
        egui::Id::unique(("zebra-keys-row", keymap::id(a))),
        Sense::click(),
    );
    let painter = ui.painter();
    if recording {
        painter.rect(
            keys_rect,
            4.0,
            p.field,
            Stroke::new(1.0, p.control_edge),
            StrokeKind::Inside,
        );
        w::focus_ring(ui, keys_rect, 4.0);
        let text = if held.any() {
            format!("{}…", crate::i18n::modifier_text(held))
        } else {
            crate::t!("settings-keys-type")
        };
        ui.painter().text(
            keys_rect.right_center() - Vec2::new(8.0, 0.0),
            Align2::RIGHT_CENTER,
            text,
            style::ui_font(13.0),
            p.weak,
        );
    } else {
        if resp.hovered() {
            painter.rect_filled(keys_rect, 4.0, p.hover);
        }
        // The first binding, as its menu item shows it (the others are the
        // same keys on other layouts, or older alternatives).
        let (text, color) = match a.shortcut_text() {
            Some(k) => (k, p.text),
            None => (crate::t!("settings-keys-none"), p.disabled),
        };
        painter.text(
            keys_rect.right_center() - Vec2::new(8.0, 0.0),
            Align2::RIGHT_CENTER,
            text,
            style::ui_font(13.0),
            color,
        );
    }
    // Space or Return that just ended a recording must not start another.
    if resp.clicked() && !recorded_now {
        app.key_message = None;
        app.key_capture = if recording {
            None
        } else {
            Some(keymap::Capture {
                action: a.clone(),
                got: None,
            })
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rebindable_command_is_in_exactly_one_group() {
        let g = groups();
        for a in keymap::rebindable() {
            let n = g.iter().filter(|g| g.actions.contains(&a)).count();
            assert_eq!(n, 1, "{a:?} is in {n} groups");
        }
        let names: Vec<&str> = g.iter().map(|g| g.name.as_str()).collect();
        println!("groups: {names:?}");
        assert_eq!(names.first().copied(), Some("Zebra"));
        assert!(names.contains(&"Arrange") && names.contains(&"File"));
    }
}
