//! Menu structure shared by the native macOS menu bar (muda) and the egui
//! fallback menu bar used on other platforms.
use zebra_core::{FrameStyle, Theme, Tool};

use crate::actions::Action;
use crate::app::ZebraApp;

pub enum Item {
    A(Action),
    Sep,
    Sub(&'static str, Vec<Item>),
    /// Filled at runtime (Open Recent).
    Recent,
}

pub fn spec() -> Vec<(&'static str, Vec<Item>)> {
    use Action::*;
    use Item::*;
    vec![
        (
            "File",
            vec![A(New), A(ShowTemplates), A(NewFromSample), A(Open), Recent, Sep, A(Save), A(SaveAs), A(SaveAsTemplate), Sep, A(ExportPdf), A(ExportPng)],
        ),
        (
            "Edit",
            vec![A(Undo), A(Redo), Sep, A(Cut), A(Copy), A(Paste), A(Duplicate), A(Delete), Sep, A(SelectAll), A(Deselect), Sep, A(CommandPalette)],
        ),
        (
            "Insert",
            vec![
                A(SetTool(Tool::Text)),
                Sub("Frame", FrameStyle::ALL.iter().map(|s| A(SetFrameStyle(*s))).collect()),
                A(SetTool(Tool::Rectangle)),
                A(SetTool(Tool::Ellipse)),
                A(SetTool(Tool::Line)),
                A(SetTool(Tool::Arrow)),
                A(SetTool(Tool::Pen)),
                A(InsertImage),
                A(InsertLayout),
                Sep,
                A(WrapInFrame),
            ],
        ),
        ("Arrange", vec![A(BringForward), A(SendBackward), A(BringToFront), A(SendToBack), Sep, A(ToggleLock)]),
        (
            "View",
            vec![
                A(ZoomIn),
                A(ZoomOut),
                A(ZoomReset),
                A(FitAll),
                A(ZoomToSelection),
                Sep,
                A(ToggleSidebar),
                A(ToggleProperties),
                Sep,
                Sub("Theme", Theme::ids().into_iter().map(|t| A(SetTheme(t))).chain([Sep, A(ThemeEditor)]).collect()),
                A(ToggleReduceMotion),
                Sep,
                A(ShowShortcuts),
            ],
        ),
        ("Path", vec![A(TogglePathEdit), A(AddToPath), A(AddCurrentView), Sep, A(AutoPath), A(ClearPath)]),
        ("Present", vec![A(PresentStart), A(PresentCurrent), Sep, A(TogglePresenterView), A(PlaybackOptions)]),
        ("Agent", vec![A(ToggleAiControl), Sep, A(ToggleActivity), A(PauseAgents), A(StopAgents), A(FollowAgent), Sep, A(RevertAgent)]),
        ("Help", vec![A(ShowShortcuts), A(About)]),
    ]
}

/// egui menu bar (non-macOS; macOS uses the real menu bar).
#[allow(dead_code)]
pub fn egui_menu_bar(app: &mut ZebraApp, ui: &mut egui::Ui) {
    egui::MenuBar::new().ui(ui, |ui| {
        for (title, items) in spec() {
            ui.menu_button(title, |ui| items_ui(app, ui, &items));
        }
    });
}

fn items_ui(app: &mut ZebraApp, ui: &mut egui::Ui, items: &[Item]) {
    for it in items {
        match it {
            Item::A(a) => {
                let mut b = egui::Button::new(a.label());
                if let Some(s) = a.shortcut_text() {
                    b = b.shortcut_text(s);
                }
                if ui.add(b).clicked() {
                    app.queued.push(a.clone());
                    ui.close();
                }
            }
            Item::Sep => {
                ui.separator();
            }
            Item::Sub(t, sub) => {
                ui.menu_button(*t, |ui| items_ui(app, ui, sub));
            }
            Item::Recent => {
                let recent = app.recent.clone();
                ui.menu_button("Open Recent", |ui| {
                    if recent.is_empty() {
                        ui.label("No recent files");
                    }
                    for p in recent {
                        if ui.button(p.display().to_string()).clicked() {
                            app.queued.push(Action::OpenPath(p));
                            ui.close();
                        }
                    }
                });
            }
        }
    }
}
