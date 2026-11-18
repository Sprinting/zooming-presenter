//! Menu structure shared by the native macOS menu bar (muda) and the egui
//! fallback menu bar used on other platforms. Follows the standard Mac
//! layout (critique §3.2): every command lives in a menu so Help › Search
//! finds it; titles flip ("Show Sidebar" / "Hide Sidebar", "Undo Rotate");
//! toggles carry a ✓; items grey out when they can't apply.
use zebra_core::{FrameStyle, Mode, Theme, Tool};

use crate::actions::Action;
use crate::app::ZebraApp;

pub enum Item {
    A(Action),
    Sep,
    Sub(String, Vec<Item>),
    /// Filled at runtime (Open Recent).
    Recent,
}

pub fn spec() -> Vec<(String, Vec<Item>)> {
    use Action::*;
    use Item::*;
    vec![
        (
            crate::t!("menu-file"),
            vec![
                A(New),
                A(ShowTemplates),
                A(NewFromSample),
                A(Open),
                Recent,
                Sep,
                A(Close),
                A(Save),
                A(SaveAs),
                A(SaveAsTemplate),
                Sep,
                Sub(crate::t!("menu-export-to"), vec![A(ExportPdf), A(ExportPng)]),
            ],
        ),
        (
            crate::t!("menu-edit"),
            vec![A(Undo), A(Redo), Sep, A(Cut), A(Copy), A(Paste), A(Delete), A(Duplicate), Sep, A(SelectAll), A(Deselect)],
        ),
        (
            crate::t!("menu-insert"),
            vec![
                A(SetTool(Tool::Text)),
                A(InsertFrame),
                Sub(crate::t!("menu-frame"), FrameStyle::ALL.iter().map(|s| A(SetFrameStyle(*s))).collect()),
                Sub(crate::t!("menu-shape"), vec![A(SetTool(Tool::Rectangle)), A(SetTool(Tool::Ellipse)), Sep, A(ShapeLibrary)]),
                A(SetTool(Tool::Line)),
                A(SetTool(Tool::Arrow)),
                A(SetTool(Tool::Pen)),
                Sep,
                A(InsertImage),
                A(InsertVideo),
                A(InsertMusic),
                A(InsertLayout),
                A(InsertDiagram),
                A(InsertSymbols),
                Sep,
                A(WrapInFrame),
            ],
        ),
        (
            crate::t!("menu-format"),
            vec![
                Sub(crate::t!("menu-font"), vec![A(Bold), A(Italic), A(Underline)]),
                Sep,
                Sub(crate::t!("menu-theme"), Theme::ids().into_iter().map(|t| A(SetTheme(t))).chain([Sep, A(ThemeEditor)]).collect()),
            ],
        ),
        (
            crate::t!("menu-arrange"),
            vec![A(BringForward), A(BringToFront), A(SendBackward), A(SendToBack), Sep, A(ToggleLock), Sep, A(Group), A(Ungroup)],
        ),
        (
            crate::t!("menu-view"),
            vec![
                A(ZoomIn),
                A(ZoomOut),
                A(ZoomReset),
                A(FitAll),
                A(ZoomToSelection),
                Sep,
                A(ToggleSidebar),
                A(ToggleProperties),
                A(ToggleActivity),
            ],
        ),
        (crate::t!("menu-path"), vec![A(TogglePathEdit), A(AddToPath), A(AddCurrentView), Sep, A(AutoPath), A(ClearPath)]),
        (crate::t!("menu-play"), vec![A(PresentStart), A(PresentCurrent), Sep, A(TogglePresenterView), A(PlaybackOptions)]),
        (
            crate::t!("menu-agent"),
            vec![A(ToggleAiControl), Sep, A(PauseAgents), A(StopAgents), A(FollowAgent), Sep, A(RevertAgent)],
        ),
        (crate::t!("menu-help"), vec![A(ShowShortcuts), A(CommandPalette), Sep, A(Acknowledgements)]),
    ]
}

/// The live state of one menu item.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemState {
    pub title: String,
    pub enabled: bool,
    /// `Some` for items that show a ✓ when on.
    pub checked: Option<bool>,
}

/// Title, enabled state and ✓ for an action, from the app's state.
pub fn state(app: &ZebraApp, a: &Action) -> ItemState {
    use Action::*;
    let has_sel = !app.editor.view.selection.is_empty();
    let editable = !app.editable_selection().is_empty();
    let has_doc = !app.editor.doc.elements.is_empty();
    let connected = app.agent_connected();
    let mut title = a.label();
    let mut checked = None;
    let enabled = match a {
        Undo => {
            if let Some(h) = app.editor.history().last() {
                title = if h.origin.is_agent() {
                    crate::t!("menu-undo-by-agent", label = h.label.as_str(), agent = h.origin.display())
                } else {
                    crate::t!("menu-undo-named", label = h.label.as_str())
                };
            }
            app.editor.can_undo()
        }
        Redo => {
            if let Some(h) = app.editor.redo_entry() {
                title = if h.origin.is_agent() {
                    crate::t!("menu-redo-by-agent", label = h.label.as_str(), agent = h.origin.display())
                } else {
                    crate::t!("menu-redo-named", label = h.label.as_str())
                };
            }
            app.editor.can_redo()
        }
        Cut | Delete => editable,
        Copy | Duplicate | WrapInFrame | BringForward | BringToFront | SendBackward | SendToBack | ZoomToSelection | Group | Ungroup => has_sel,
        // Zebra's own copy, or anything another app put on the clipboard
        // (a picture, files, text): Paste takes those too.
        Paste => !app.clipboard.is_empty() || crate::media_ui::system_clipboard_has_content(),
        Deselect => has_sel,
        SelectAll | FitAll | ExportPdf | ExportPng | PresentStart | AutoPath | AddCurrentView | AddToPath => has_doc,
        PresentCurrent => has_doc && !app.editor.doc.path.is_empty(),
        ClearPath => !app.editor.doc.path.is_empty(),
        ToggleLock => {
            let sel = app.selection();
            let locked = !sel.is_empty() && sel.iter().all(|id| app.editor.doc.get(id).is_some_and(|e| e.locked));
            title = if locked { crate::t!("menu-unlock") } else { crate::t!("action-toggle-lock") };
            has_sel
        }
        Bold | Italic | Underline => {
            let texts: Vec<(bool, bool)> = app
                .selection()
                .iter()
                .filter_map(|id| match app.editor.doc.get(id).map(|e| &e.kind) {
                    Some(zebra_core::ElementKind::Text(t)) => Some((t.bold, t.italic)),
                    _ => None,
                })
                .collect();
            checked = Some(
                !texts.is_empty()
                    && match a {
                        Bold => texts.iter().all(|t| t.0),
                        Italic => texts.iter().all(|t| t.1),
                        _ => false,
                    },
            );
            !texts.is_empty()
        }
        ToggleSidebar => {
            title = if app.show_sidebar { crate::t!("menu-hide-sidebar") } else { crate::t!("action-toggle-sidebar") };
            true
        }
        ToggleProperties => {
            title = if app.show_props { crate::t!("menu-hide-inspector") } else { crate::t!("action-toggle-properties") };
            true
        }
        ToggleActivity => {
            title = if app.show_activity { crate::t!("menu-hide-agent-activity") } else { crate::t!("menu-show-agent-activity") };
            true
        }
        TogglePathEdit => {
            checked = Some(app.editor.view.mode == Mode::PathEdit);
            true
        }
        SetTool(t) => {
            checked = Some(app.editor.view.tool == *t);
            true
        }
        SetFrameStyle(s) => {
            checked = Some(app.editor.view.tool == Tool::Frame && app.editor.view.frame_style == *s);
            true
        }
        SetTheme(t) => {
            title = crate::words::theme_name(&Theme::resolve(t));
            checked = Some(app.editor.doc.theme == *t);
            true
        }
        ToggleAiControl => {
            checked = Some(app.ai_enabled);
            true
        }
        PauseAgents => {
            title = if app.editor.view.agent_paused { crate::t!("menu-resume-agent") } else { crate::t!("menu-pause-agent") };
            connected
        }
        StopAgents | RevertAgent => connected,
        FollowAgent => {
            checked = Some(app.editor.view.follow_agent);
            connected
        }
        _ => true,
    };
    ItemState { title, enabled, checked }
}

/// Is this action a ✓-style item (needs a check menu item natively)?
pub fn is_checkable(a: &Action) -> bool {
    use Action::*;
    matches!(a, Bold | Italic | Underline | TogglePathEdit | SetTool(_) | SetFrameStyle(_) | SetTheme(_) | ToggleAiControl | FollowAgent)
}

/// egui menu bar (non-macOS; macOS uses the real menu bar).
#[allow(dead_code)]
pub fn egui_menu_bar(app: &mut ZebraApp, ui: &mut egui::Ui) {
    egui::MenuBar::new().ui(ui, |ui| {
        for (title, items) in spec() {
            ui.menu_button(title.as_str(), |ui| {
                ui.set_min_width(220.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                items_ui(app, ui, &items)
            });
        }
        if ui.button(crate::t!("action-settings")).clicked() {
            app.queued.push(Action::Settings);
        }
    });
}

fn items_ui(app: &mut ZebraApp, ui: &mut egui::Ui, items: &[Item]) {
    for it in items {
        match it {
            Item::A(a) => {
                let st = state(app, a);
                let sc = a.shortcut_text();
                if crate::widgets::menu_row(ui, st.checked, &st.title, sc.as_deref(), st.enabled).clicked() {
                    app.queued.push(a.clone());
                }
            }
            Item::Sep => crate::widgets::menu_separator(ui),
            Item::Sub(t, sub) => {
                ui.menu_button(t.as_str(), |ui| {
                    ui.set_min_width(200.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    items_ui(app, ui, sub)
                });
            }
            Item::Recent => {
                let recent = app.recent.clone();
                ui.menu_button(crate::t!("menu-open-recent"), |ui| {
                    ui.set_min_width(260.0);
                    if recent.is_empty() {
                        crate::widgets::menu_row(ui, None, &crate::t!("menu-no-recent-documents"), None, false);
                    }
                    for p in recent {
                        let name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                        if crate::widgets::menu_item(ui, &name, None).on_hover_text(p.display().to_string()).clicked() {
                            app.queued.push(Action::OpenPath(p));
                        }
                    }
                });
            }
        }
    }
}
