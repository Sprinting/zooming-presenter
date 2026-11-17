//! Document-modal questions as sheets, never toasts.
//!
//! On the Mac these are real `NSAlert`s attached to the window as sheets
//! (rfd with the window as parent: `beginSheetModalForWindow`). NSAlert
//! gives Return to the first button, Esc to "Cancel" and ⌘D to "Don't
//! Save" by itself. Elsewhere, and in UI tests, the same sheet is drawn in
//! egui in the Big Sur alert style: narrow, centred under the toolbar, app
//! icon, bold title, small message, stacked buttons with the default one in
//! the accent colour.
use egui::{Align2, Color32, Pos2, Rect, RichText, Vec2};
use zebra_core::view::Permission;
use zebra_core::Origin;

use crate::actions::Action;
use crate::app::ZebraApp;
use crate::style::{self, Palette};
use crate::widgets as w;

const H: Origin = Origin::Human;

#[derive(Clone, Debug, PartialEq)]
pub enum SheetKind {
    /// "Do you want to save the changes…?" before an action that discards
    /// the document (Close, Quit, New, Open).
    SaveChanges(Action),
    /// A plain alert with one OK button (open and save errors).
    Alert { title: String, message: String },
    ClearPath,
    RevertAgent { name: String },
    RaisePermission,
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Default,
    Cancel,
    Normal,
}

struct Spec {
    title: String,
    message: String,
    buttons: Vec<(String, Role)>,
    warning: bool,
}

fn spec(app: &ZebraApp, k: &SheetKind) -> Spec {
    let doc_name = || match &app.editor.file_path {
        Some(p) => p.file_stem().unwrap_or_default().to_string_lossy().to_string(),
        None => app.editor.doc.title.clone(),
    };
    match k {
        SheetKind::SaveChanges(_) => Spec {
            title: format!("Do you want to save the changes made to the document \u{201c}{}\u{201d}?", doc_name()),
            message: "Your changes will be lost if you don't save them.".into(),
            buttons: vec![("Save".into(), Role::Default), ("Don't Save".into(), Role::Normal), ("Cancel".into(), Role::Cancel)],
            warning: false,
        },
        SheetKind::Alert { title, message } => Spec { title: title.clone(), message: message.clone(), buttons: vec![("OK".into(), Role::Default)], warning: true },
        SheetKind::ClearPath => {
            let n = app.editor.doc.path.len();
            Spec {
                title: "Clear the path?".into(),
                message: format!(
                    "This removes {} from the path. Frames and objects stay on the canvas. You can undo this.",
                    if n == 1 { "the only step".to_string() } else { format!("all {n} steps") }
                ),
                buttons: vec![("Clear Path".into(), Role::Default), ("Cancel".into(), Role::Cancel)],
                warning: true,
            }
        }
        SheetKind::RevertAgent { name } => Spec {
            title: format!("Revert the changes {name} made?"),
            message: format!("This undoes everything {name} did since your last change. Redo (⇧⌘Z) brings it back."),
            buttons: vec![("Revert".into(), Role::Default), ("Cancel".into(), Role::Cancel)],
            warning: true,
        },
        SheetKind::RaisePermission => Spec {
            title: "Give agents full control?".into(),
            message: "Agents will also be able to move your view, open and save files, and start presenting.".into(),
            buttons: vec![("Allow Full Control".into(), Role::Default), ("Cancel".into(), Role::Cancel)],
            warning: true,
        },
    }
}

/// Ask a question as a sheet. Replaces any sheet already waiting.
pub fn request(app: &mut ZebraApp, k: SheetKind) {
    app.sheet = Some(k);
}

/// Act on the button the user chose (`label` as shown).
pub fn resolve(app: &mut ZebraApp, k: SheetKind, label: &str) {
    match k {
        SheetKind::SaveChanges(action) => match label {
            "Save" => {
                if app.save(false) {
                    app.force_quit = true;
                    app.queued.push(action);
                }
            }
            "Don't Save" => {
                app.force_quit = true;
                app.queued.push(action);
            }
            _ => {}
        },
        SheetKind::Alert { .. } => {}
        SheetKind::ClearPath => {
            if label == "Clear Path" && !app.editor.doc.path.is_empty() {
                app.apply(zebra_core::Command::ClearPath);
                app.selected_step = None;
            }
        }
        SheetKind::RevertAgent { name } => {
            if label == "Revert" {
                let n = app.editor.revert_agent_session(&name);
                if n == 0 {
                    app.toast(format!("{name} has no changes to revert."));
                }
            }
        }
        SheetKind::RaisePermission => {
            if label == "Allow Full Control" {
                let _ = app.editor.view(zebra_core::ViewCommand::SetAgentPermission { permission: Permission::Full }, H);
            }
        }
    }
}

/// Native sheets: call from `eframe::App::ui` where the window handle is.
/// Returns true if a sheet was shown.
#[cfg(target_os = "macos")]
pub fn run_native(app: &mut ZebraApp, frame: &eframe::Frame) -> bool {
    if !app.native_dialogs {
        return false;
    }
    let Some(k) = app.sheet.take() else { return false };
    let s = spec(app, &k);
    let buttons = match s.buttons.as_slice() {
        [(a, _)] => rfd::MessageButtons::OkCustom(a.clone()),
        [(a, _), (b, _)] => rfd::MessageButtons::OkCancelCustom(a.clone(), b.clone()),
        [(a, _), (b, _), (c, _)] => rfd::MessageButtons::YesNoCancelCustom(a.clone(), b.clone(), c.clone()),
        _ => rfd::MessageButtons::Ok,
    };
    let r = rfd::MessageDialog::new()
        .set_level(if s.warning { rfd::MessageLevel::Warning } else { rfd::MessageLevel::Info })
        .set_title(&s.title)
        .set_description(&s.message)
        .set_buttons(buttons)
        .set_parent(frame)
        .show();
    let label = match r {
        rfd::MessageDialogResult::Custom(l) => l,
        rfd::MessageDialogResult::Ok => "OK".into(),
        _ => "Cancel".into(),
    };
    resolve(app, k, &label);
    true
}

/// egui-drawn sheet (non-Mac, tests, and the Mac before the window exists).
pub fn ui(app: &mut ZebraApp, ctx: &egui::Context) {
    if app.native_dialogs {
        return;
    }
    let Some(k) = app.sheet.clone() else { return };
    let s = spec(app, &k);
    let p = Palette::of(ctx);
    let content = ctx.content_rect();
    let top = content.top() + crate::panels::TITLE_ROW + crate::panels::TOOLBAR_ROW;
    let mut chosen: Option<String> = None;

    // Block the window behind the sheet (sheets are document-modal), with
    // no grey scrim: AppKit only dims slightly.
    egui::Area::new(egui::Id::unique("sheet-block")).order(egui::Order::Foreground).fixed_pos(content.min).show(ctx, |ui| {
        let r = ui.allocate_rect(content, egui::Sense::click_and_drag());
        ui.painter().rect_filled(r.rect, 0.0, Color32::from_black_alpha(if p.dark { 40 } else { 14 }));
    });
    egui::Area::new(egui::Id::unique("sheet"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(content.center().x - 130.0, top + 2.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(p.panel)
                .stroke(egui::Stroke::new(0.6, if p.dark { Color32::from_gray(0x50) } else { Color32::from_black_alpha(45) }))
                .corner_radius(10)
                .shadow(ui.visuals().window_shadow)
                .inner_margin(egui::Margin { left: 16, right: 16, top: 18, bottom: 16 })
                .show(ui, |ui| {
                    ui.set_width(228.0);
                    ui.vertical_centered(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(54.0), egui::Sense::hover());
                        crate::canvas::draw_zebra_disc(ui.painter(), r.center(), 24.0);
                        if s.warning {
                            let b = Rect::from_center_size(r.right_bottom() - Vec2::new(10.0, 10.0), Vec2::splat(22.0));
                            ui.painter().circle_filled(b.center(), 11.0, p.panel);
                            crate::icons::paint_icon(ui.painter(), b, crate::icons::Icon::Warning, Color32::from_rgb(0xff, 0x9f, 0x0a));
                        }
                        ui.add_space(8.0);
                        ui.add(egui::Label::new(RichText::new(&s.title).font(style::semibold(13.0)).color(p.text)).wrap().halign(egui::Align::Center));
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(RichText::new(&s.message).font(style::ui_font(11.0)).color(p.text)).wrap().halign(egui::Align::Center));
                        ui.add_space(14.0);
                        let wdt = ui.available_width();
                        for (label, role) in &s.buttons {
                            let r = if *role == Role::Default {
                                let (rect, resp) = ui.allocate_exact_size(Vec2::new(wdt, w::H_REGULAR + 2.0), egui::Sense::click());
                                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::DefaultButton, true, label));
                                ui.painter().rect_filled(rect, w::RADIUS, style::accent());
                                ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, style::ui_font(13.0), w::on_accent());
                                resp
                            } else {
                                w::push_button_ext(ui, label, w::Size::Regular, true, Some(wdt))
                            };
                            if r.clicked() {
                                chosen = Some(label.clone());
                            }
                            ui.add_space(2.0);
                        }
                    });
                });
        });
    // Keyboard: Return = default, Esc = Cancel, ⌘D = Don't Save.
    ctx.input(|i| {
        if i.key_pressed(egui::Key::Enter) {
            chosen = s.buttons.iter().find(|b| b.1 == Role::Default).map(|b| b.0.clone());
        }
        if i.key_pressed(egui::Key::Escape) {
            chosen = Some(s.buttons.iter().find(|b| b.1 == Role::Cancel).map(|b| b.0.clone()).unwrap_or_else(|| "OK".into()));
        }
        if i.modifiers.command && i.key_pressed(egui::Key::D) && s.buttons.iter().any(|b| b.0 == "Don't Save") {
            chosen = Some("Don't Save".into());
        }
    });
    if let Some(label) = chosen {
        app.sheet = None;
        resolve(app, k, &label);
    }
}
