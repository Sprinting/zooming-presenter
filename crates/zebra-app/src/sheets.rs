//! Document-modal questions as sheets, never toasts.
//!
//! On the Mac these are real `NSAlert`s attached to the window as sheets
//! (rfd's async dialog with the window as parent: `beginSheetModalForWindow`, no modal loop). NSAlert
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
    /// Path from Frames would drop steps that have speaker notes.
    AutoPath,
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
            title: crate::t!("sheet-save-changes-title", name = doc_name()),
            message: crate::t!("sheet-save-changes-message"),
            buttons: vec![(crate::t!("sheet-save"), Role::Default), (crate::t!("sheet-dont-save"), Role::Normal), (crate::t!("sheet-cancel"), Role::Cancel)],
            warning: false,
        },
        SheetKind::Alert { title, message } => Spec { title: title.clone(), message: message.clone(), buttons: vec![(crate::t!("sheet-ok"), Role::Default)], warning: true },
        SheetKind::ClearPath => {
            let n = app.editor.doc.path.len();
            Spec {
                title: crate::t!("sheet-clear-path-title"),
                message: crate::t!("sheet-clear-path-message", count = n),
                buttons: vec![(crate::t!("sheet-clear-path"), Role::Default), (crate::t!("sheet-cancel"), Role::Cancel)],
                warning: true,
            }
        }
        SheetKind::AutoPath => Spec {
            title: crate::t!("sheet-auto-path-title"),
            message: crate::t!("sheet-auto-path-message"),
            buttons: vec![(crate::t!("sheet-auto-path"), Role::Default), (crate::t!("sheet-cancel"), Role::Cancel)],
            warning: true,
        },
        SheetKind::RevertAgent { name } => Spec {
            title: crate::t!("sheet-revert-agent-title", name = name.as_str()),
            message: crate::t!("sheet-revert-agent-message", name = name.as_str(), shortcut = Action::Redo.shortcut_text().unwrap_or_default()),
            buttons: vec![(crate::t!("sheet-revert"), Role::Default), (crate::t!("sheet-cancel"), Role::Cancel)],
            warning: true,
        },
        SheetKind::RaisePermission => Spec {
            title: crate::t!("sheet-full-control-title"),
            message: crate::t!("sheet-full-control-message"),
            buttons: vec![(crate::t!("sheet-allow-full-control"), Role::Default), (crate::t!("sheet-cancel"), Role::Cancel)],
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
        SheetKind::SaveChanges(action) => {
            if label == crate::t!("sheet-save") {
                if app.save(false) {
                    app.force_quit = true;
                    app.queued.push(action);
                }
            } else if label == crate::t!("sheet-dont-save") {
                crate::files::discard_autosave(app);
                app.force_quit = true;
                app.queued.push(action);
            }
        }
        SheetKind::Alert { .. } => {}
        SheetKind::ClearPath => {
            if label == crate::t!("sheet-clear-path") && !app.editor.doc.path.is_empty() {
                app.apply(zebra_core::Command::ClearPath { discard_notes: true });
                app.selected_step = None;
            }
        }
        SheetKind::AutoPath => {
            if label == crate::t!("sheet-auto-path") {
                app.apply(zebra_core::Command::AutoPath { discard_notes: true });
                app.selected_step = None;
            }
        }
        SheetKind::RevertAgent { name } => {
            if label == crate::t!("sheet-revert") {
                let n = app.editor.revert_agent_session(&name);
                if n == 0 {
                    app.toast(crate::t!("sheet-toast-nothing-to-revert", name = name.as_str()));
                }
            }
        }
        SheetKind::RaisePermission => {
            if label == crate::t!("sheet-allow-full-control") {
                let _ = app.editor.view(zebra_core::ViewCommand::SetAgentPermission { permission: Permission::Full }, H);
            }
        }
    }
}

/// A native sheet on screen: its kind and the answer to come.
#[cfg(target_os = "macos")]
pub struct OpenSheet {
    kind: SheetKind,
    answer: std::pin::Pin<Box<dyn std::future::Future<Output = rfd::MessageDialogResult>>>,
}

/// Native sheets: call from `eframe::App::ui` where the window handle is.
/// The NSAlert is attached as a sheet without a modal loop (a blocking
/// one spun the CPU at 100% while it was open); its answer is polled here
/// each frame, and AppKit wakes us when a button is pressed.
/// Returns true while a sheet is open.
#[cfg(target_os = "macos")]
pub fn run_native(app: &mut ZebraApp, frame: Option<&eframe::Frame>) -> bool {
    if !app.native_dialogs {
        return false;
    }
    if app.open_sheet.is_none() {
        let Some(k) = app.sheet.take() else { return false };
        let s = spec(app, &k);
        let buttons = match s.buttons.as_slice() {
            [(a, _)] => rfd::MessageButtons::OkCustom(a.clone()),
            [(a, _), (b, _)] => rfd::MessageButtons::OkCancelCustom(a.clone(), b.clone()),
            [(a, _), (b, _), (c, _)] => rfd::MessageButtons::YesNoCancelCustom(a.clone(), b.clone(), c.clone()),
            _ => rfd::MessageButtons::Ok,
        };
        let dialog = rfd::AsyncMessageDialog::new()
            .set_level(if s.warning { rfd::MessageLevel::Warning } else { rfd::MessageLevel::Info })
            .set_title(&s.title)
            .set_description(&s.message)
            .set_buttons(buttons);
        // Attached to its own window, whichever document window that is.
        let dialog = match (frame, app.mac_sync.window.clone()) {
            (Some(f), _) => dialog.set_parent(f),
            (None, Some(w)) => dialog.set_parent(&crate::mac::NsParent(w)),
            (None, None) => dialog,
        };
        let answer = dialog.show();
        app.open_sheet = Some(OpenSheet { kind: k, answer: Box::pin(answer) });
    }
    let ctx = app.ctx.clone();
    let waker = std::task::Waker::from(std::sync::Arc::new(Repaint(ctx)));
    let mut cx = std::task::Context::from_waker(&waker);
    let open = app.open_sheet.as_mut().expect("sheet");
    let std::task::Poll::Ready(r) = open.answer.as_mut().poll(&mut cx) else { return true };
    let k = app.open_sheet.take().expect("sheet").kind;
    let label = match r {
        rfd::MessageDialogResult::Custom(l) => l,
        rfd::MessageDialogResult::Ok => crate::t!("sheet-ok"),
        _ => crate::t!("sheet-cancel"),
    };
    resolve(app, k, &label);
    true
}

/// Wakes the UI when a native sheet is answered.
#[cfg(target_os = "macos")]
struct Repaint(egui::Context);

#[cfg(target_os = "macos")]
impl std::task::Wake for Repaint {
    fn wake(self: std::sync::Arc<Self>) {
        self.0.request_repaint();
    }
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
    egui::Area::new(app.uid("sheet-block")).order(egui::Order::Foreground).fixed_pos(content.min).show(ctx, |ui| {
        let r = ui.allocate_rect(content, egui::Sense::click_and_drag());
        ui.painter().rect_filled(r.rect, 0.0, Color32::from_black_alpha(if p.dark { 40 } else { 14 }));
    });
    egui::Area::new(app.uid("sheet"))
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
            chosen = Some(s.buttons.iter().find(|b| b.1 == Role::Cancel).map(|b| b.0.clone()).unwrap_or_else(|| crate::t!("sheet-ok")));
        }
        let dont_save = crate::t!("sheet-dont-save");
        if i.modifiers.command && i.key_pressed(egui::Key::D) && s.buttons.iter().any(|b| b.0 == dont_save) {
            chosen = Some(dont_save);
        }
    });
    if let Some(label) = chosen {
        app.sheet = None;
        resolve(app, k, &label);
    }
}
