//! Every menu item, toolbar button and keyboard shortcut is an [`Action`].
//! `ZebraApp::perform` turns actions into zebra-core commands, so the native
//! menu bar, the egui fallback menus, shortcuts and the ⌘K palette all share
//! one path (and the AI drives the very same core commands over the API).
use egui::{Key, Modifiers};
use zebra_core::{FrameStyle, Tool};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    // File
    New,
    NewFromSample,
    /// Open the template chooser.
    ShowTemplates,
    NewFromTemplate(String),
    NewFromTemplateFile(std::path::PathBuf),
    SaveAsTemplate,
    Open,
    OpenPath(std::path::PathBuf),
    Save,
    SaveAs,
    ExportPdf,
    ExportPng,
    /// File > Export > Web Page (HTML)…
    ExportHtml,
    /// File › Import… (PDF).
    Import,
    Close,
    Quit,
    // Edit
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    SelectAll,
    Deselect,
    // Insert / tools
    SetTool(Tool),
    FrameTool,
    SetFrameStyle(FrameStyle),
    /// A frame in the nearest empty space.
    InsertFrame,
    InsertImage,
    InsertLayout,
    /// Insert › Diagram…: the diagram gallery.
    InsertDiagram,
    /// Insert › Symbols & Shapes…: the library on its Symbols tab.
    InsertSymbols,
    /// Background music for the presentation.
    InsertMusic,
    ReplaceImage,
    CropImage,
    PlayPauseVideo,
    WrapInFrame,
    /// Opens the shapes and icons library (stream 3 fills it in).
    ShapeLibrary,
    /// Insert › Video… (stream 2 fills it in).
    InsertVideo,
    /// Insert › Audio… (stream 2 fills it in).
    // Format (whole text box until rich text runs land; stream 3 routes
    // these to the text selection while editing)
    Bold,
    Italic,
    Underline,
    // Arrange
    BringForward,
    SendBackward,
    BringToFront,
    SendToBack,
    ToggleLock,
    /// Group / Ungroup (stream 3 fills these in).
    Group,
    Ungroup,
    // View
    ZoomIn,
    ZoomOut,
    FitAll,
    ZoomToSelection,
    ZoomReset,
    ToggleSidebar,
    ToggleProperties,
    ToggleReduceMotion,
    CommandPalette,
    ShowShortcuts,
    Settings,
    // Path
    TogglePathEdit,
    AddToPath,
    AddCurrentView,
    AutoPath,
    ClearPath,
    // Present
    PresentStart,
    PresentCurrent,
    TogglePresenterView,
    PlaybackOptions,
    // Agent
    ToggleAiControl,
    ToggleActivity,
    PauseAgents,
    StopAgents,
    FollowAgent,
    RevertAgent,
    /// Change what agents may do; raising to Full asks first.
    SetAgentPermission(zebra_core::view::Permission),
    // Theme
    SetTheme(String),
    ThemeEditor,
    About,
    /// Credits and licences of the clip art, icons and fonts.
    Acknowledgements,
}

impl Action {
    pub fn label(&self) -> String {
        use Action::*;
        match self {
            New => crate::t!("action-new"),
            NewFromSample => crate::t!("action-new-from-sample"),
            ShowTemplates => crate::t!("action-show-templates"),
            NewFromTemplate(t) => crate::t!("action-new-from-template", name = zebra_core::templates::TEMPLATES.iter().find(|x| x.id == *t).map(crate::words::template_name).unwrap_or_else(|| t.to_string())),
            NewFromTemplateFile(p) => p.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
            SaveAsTemplate => crate::t!("action-save-as-template"),
            Open => crate::t!("action-open"),
            OpenPath(p) => p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
            Save => crate::t!("action-save"),
            SaveAs => crate::t!("action-save-as"),
            ExportPdf => crate::t!("action-export-pdf"),
            ExportPng => crate::t!("action-export-png"),
            ExportHtml => crate::t!("action-export-html"),
            Import => crate::t!("action-import"),
            Close => crate::t!("action-close"),
            Quit => crate::t!("action-quit"),
            Undo => crate::t!("action-undo"),
            Redo => crate::t!("action-redo"),
            Cut => crate::t!("action-cut"),
            Copy => crate::t!("action-copy"),
            Paste => crate::t!("action-paste"),
            Duplicate => crate::t!("action-duplicate"),
            Delete => crate::t!("action-delete"),
            SelectAll => crate::t!("action-select-all"),
            Deselect => crate::t!("action-deselect"),
            SetTool(t) => crate::t!("action-set-tool", tool = tool_name(*t)),
            FrameTool => crate::t!("action-frame-tool"),
            InsertFrame => crate::t!("action-insert-frame"),
            SetFrameStyle(s) => crate::t!("action-set-frame-style", style = frame_name(*s)),
            InsertImage => crate::t!("action-insert-image"),
            InsertLayout => crate::t!("action-insert-layout"),
            InsertDiagram => crate::t!("action-insert-diagram"),
            InsertSymbols => crate::t!("action-insert-symbols"),
            InsertMusic => crate::t!("action-insert-music"),
            ReplaceImage => crate::t!("action-replace-image"),
            CropImage => crate::t!("action-crop-image"),
            PlayPauseVideo => crate::t!("action-play-pause-video"),
            WrapInFrame => crate::t!("action-wrap-in-frame"),
            ShapeLibrary => crate::t!("action-insert-symbols"),
            InsertVideo => crate::t!("action-insert-video"),
            Bold => crate::t!("action-bold"),
            Italic => crate::t!("action-italic"),
            Underline => crate::t!("action-underline"),
            BringForward => crate::t!("action-bring-forward"),
            SendBackward => crate::t!("action-send-backward"),
            BringToFront => crate::t!("action-bring-to-front"),
            SendToBack => crate::t!("action-send-to-back"),
            ToggleLock => crate::t!("action-toggle-lock"),
            Group => crate::t!("action-group"),
            Ungroup => crate::t!("action-ungroup"),
            ZoomIn => crate::t!("action-zoom-in"),
            ZoomOut => crate::t!("action-zoom-out"),
            FitAll => crate::t!("action-fit-all"),
            ZoomToSelection => crate::t!("action-zoom-to-selection"),
            ZoomReset => crate::t!("action-zoom-reset"),
            ToggleSidebar => crate::t!("action-toggle-sidebar"),
            ToggleProperties => crate::t!("action-toggle-properties"),
            ToggleReduceMotion => crate::t!("action-toggle-reduce-motion"),
            CommandPalette => crate::t!("action-command-palette"),
            ShowShortcuts => crate::t!("action-show-shortcuts"),
            Settings => crate::t!("action-settings"),
            TogglePathEdit => crate::t!("action-toggle-path-edit"),
            AddToPath => crate::t!("action-add-to-path"),
            AddCurrentView => crate::t!("action-add-current-view"),
            AutoPath => crate::t!("action-auto-path"),
            ClearPath => crate::t!("action-clear-path"),
            PresentStart => crate::t!("action-present-start"),
            PresentCurrent => crate::t!("action-present-current"),
            TogglePresenterView => crate::t!("action-toggle-presenter-view"),
            PlaybackOptions => crate::t!("action-playback-options"),
            ToggleAiControl => crate::t!("action-toggle-ai-control"),
            ToggleActivity => crate::t!("action-toggle-activity"),
            PauseAgents => crate::t!("action-pause-agents"),
            StopAgents => crate::t!("action-stop-agents"),
            FollowAgent => crate::t!("action-follow-agent"),
            RevertAgent => crate::t!("action-revert-agent"),
            SetAgentPermission(p) => match p {
                zebra_core::view::Permission::Observe => crate::t!("action-agent-permission-observe"),
                zebra_core::view::Permission::Edit => crate::t!("action-agent-permission-edit"),
                zebra_core::view::Permission::Full => crate::t!("action-agent-permission-full"),
            },
            SetTheme(t) => crate::t!("action-set-theme", name = crate::words::theme_name(&zebra_core::Theme::resolve(t))),
            ThemeEditor => crate::t!("action-theme-editor"),
            About => crate::t!("action-about"),
            Acknowledgements => crate::t!("action-acknowledgements"),
        }
    }

    /// Human-readable shortcut, macOS glyphs.
    pub fn shortcut_text(&self) -> Option<String> {
        crate::keymap::binding_for(self).map(|(m, k)| fmt_shortcut(m, k))
    }

    /// Actions listed in the ⌘K palette.
    pub fn palette() -> Vec<Action> {
        use Action::*;
        let mut v = vec![
            New, NewFromSample, ShowTemplates, SaveAsTemplate, Open, Save, SaveAs, Import, ExportPdf, ExportPng, ExportHtml, Undo, Redo, Cut, Copy, Paste, Duplicate, Delete,
            SelectAll, Deselect,
        ];
        for t in TOOLS {
            v.push(SetTool(t.0));
        }
        v.extend([
            InsertFrame, InsertImage, InsertLayout, InsertDiagram, InsertSymbols, WrapInFrame, BringForward, SendBackward, BringToFront, SendToBack, ToggleLock, ZoomIn, ZoomOut,
            FitAll, ZoomToSelection, ZoomReset, ToggleSidebar, ToggleProperties, ShowShortcuts, Settings, ShapeLibrary,
            InsertVideo, InsertMusic, ReplaceImage, CropImage, PlayPauseVideo, Group, Ungroup,
            TogglePathEdit, AddToPath, AddCurrentView, AutoPath, ClearPath, PresentStart, PresentCurrent, TogglePresenterView, PlaybackOptions, ToggleAiControl, ToggleActivity,
            PauseAgents, StopAgents, FollowAgent, RevertAgent, ThemeEditor, Acknowledgements,
        ]);
        for t in zebra_core::Theme::ids() {
            v.push(SetTheme(t));
        }
        v
    }
}

pub const TOOLS: [(Tool, char); 10] = [
    (Tool::Select, 'V'),
    (Tool::Hand, 'H'),
    (Tool::Text, 'T'),
    (Tool::Frame, 'F'),
    (Tool::Rectangle, 'R'),
    (Tool::Ellipse, 'O'),
    (Tool::Line, 'L'),
    (Tool::Arrow, 'A'),
    (Tool::Pen, 'D'),
    (Tool::Highlighter, 'I'), // placeholder slot: the toolbar shows Image here
];

pub fn tool_name(t: Tool) -> String {
    match t {
        Tool::Select => crate::t!("tool-select"),
        Tool::Hand => crate::t!("tool-hand"),
        Tool::Text => crate::t!("tool-text"),
        Tool::Frame => crate::t!("tool-frame"),
        Tool::Rectangle => crate::t!("tool-rectangle"),
        Tool::Ellipse => crate::t!("tool-ellipse"),
        Tool::Triangle => crate::t!("tool-triangle"),
        Tool::Line => crate::t!("tool-line"),
        Tool::Arrow => crate::t!("tool-arrow"),
        Tool::Pen => crate::t!("tool-pen"),
        Tool::Highlighter => crate::t!("tool-highlighter"),
    }
}

/// Display name of a frame style, in the language in use now. Stays
/// `&'static str` (callers hand it to widgets that keep it), so each
/// distinct name is kept for good: four per language used. It used to be
/// looked up once, and stayed in the launch language after a switch.
pub fn frame_name(s: FrameStyle) -> &'static str {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static SEEN: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let name = match s {
        FrameStyle::Bracket => crate::t!("frame-style-bracket"),
        FrameStyle::Circle => crate::t!("frame-style-circle"),
        FrameStyle::Rectangle => crate::t!("frame-style-rectangle"),
        FrameStyle::Invisible => crate::t!("frame-style-invisible"),
    };
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = seen.get_or_insert_with(HashSet::new);
    match seen.get(name.as_str()) {
        Some(kept) => kept,
        None => {
            let kept: &'static str = Box::leak(name.into_boxed_str());
            seen.insert(kept);
            kept
        }
    }
}

const NONE: Modifiers = Modifiers::NONE;
const CMD: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;
const CMD_SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };
const CMD_ALT: Modifiers = Modifiers { alt: true, ..Modifiers::COMMAND };
const CMD_ALT_SHIFT: Modifiers = Modifiers { alt: true, shift: true, ..Modifiers::COMMAND };
const CMD_CTRL: Modifiers = Modifiers { ctrl: true, ..Modifiers::COMMAND };

/// The shortcuts Zebra comes with (present mode has its own keys). Most
/// specific first. Matched exactly (Shift/Alt must match), with ⌘ = Ctrl
/// off-Mac. What is live is [`crate::keymap`]: these plus the person's
/// changes.
pub static DEFAULTS: &[(Modifiers, Key, Action)] = &[
    // Keynote: Play ⌥⌘P; from the current step adds ⇧.
    (CMD_ALT_SHIFT, Key::P, Action::PresentCurrent),
    (CMD_ALT, Key::P, Action::PresentStart),
    (CMD_ALT, Key::I, Action::ToggleProperties),
    (CMD_ALT, Key::G, Action::Group),
    (CMD_ALT_SHIFT, Key::G, Action::Ungroup),
    (CMD_CTRL, Key::S, Action::ToggleSidebar),
    (CMD_SHIFT, Key::S, Action::SaveAs),
    (CMD_SHIFT, Key::I, Action::Import),
    (CMD_SHIFT, Key::N, Action::ShowTemplates),
    (CMD_SHIFT, Key::Z, Action::Redo),
    (CMD_SHIFT, Key::A, Action::AddToPath),
    (CMD_SHIFT, Key::P, Action::CommandPalette),
    (CMD_ALT, Key::Enter, Action::PresentCurrent),
    (CMD, Key::Enter, Action::PresentStart),
    (CMD, Key::S, Action::Save),
    (CMD, Key::W, Action::Close),
    (CMD, Key::Comma, Action::Settings),
    (CMD, Key::B, Action::Bold),
    (CMD, Key::I, Action::Italic),
    (CMD, Key::U, Action::Underline),
    (CMD, Key::O, Action::Open),
    (CMD, Key::N, Action::New),
    (CMD, Key::E, Action::ExportPdf),
    (CMD, Key::Z, Action::Undo),
    (CMD, Key::Y, Action::Redo),
    (CMD, Key::D, Action::Duplicate),
    (CMD, Key::A, Action::SelectAll),
    (CMD, Key::G, Action::WrapInFrame),
    (CMD, Key::L, Action::ToggleLock),
    (CMD, Key::K, Action::CommandPalette),
    (CMD, Key::J, Action::ToggleActivity),
    (CMD, Key::Period, Action::StopAgents),
    (CMD, Key::Num0, Action::ZoomReset),
    (CMD, Key::Equals, Action::ZoomIn),
    (CMD, Key::Plus, Action::ZoomIn),
    (CMD, Key::Minus, Action::ZoomOut),
    (CMD, Key::CloseBracket, Action::BringToFront),
    (CMD, Key::OpenBracket, Action::SendToBack),
    (CMD, Key::Backslash, Action::ToggleSidebar),
    (CMD, Key::Q, Action::Quit),
    (SHIFT, Key::F5, Action::PresentCurrent),
    (SHIFT, Key::Num1, Action::FitAll),
    (SHIFT, Key::Exclamationmark, Action::FitAll),
    (SHIFT, Key::Num2, Action::ZoomToSelection),
    (SHIFT, Key::Questionmark, Action::ShowShortcuts),
    (NONE, Key::Questionmark, Action::ShowShortcuts),
    (NONE, Key::Exclamationmark, Action::FitAll),
    (NONE, Key::F5, Action::PresentStart),
    // The Mac's delete key is Backspace; it comes first so menus show ⌫.
    (NONE, Key::Backspace, Action::Delete),
    (NONE, Key::Delete, Action::Delete),
    (NONE, Key::Escape, Action::Deselect),
    (NONE, Key::CloseBracket, Action::BringForward),
    (NONE, Key::OpenBracket, Action::SendBackward),
    (NONE, Key::P, Action::TogglePathEdit),
    (NONE, Key::V, Action::SetTool(Tool::Select)),
    (NONE, Key::H, Action::SetTool(Tool::Hand)),
    (NONE, Key::T, Action::SetTool(Tool::Text)),
    (NONE, Key::F, Action::FrameTool),
    (NONE, Key::R, Action::SetTool(Tool::Rectangle)),
    (NONE, Key::O, Action::SetTool(Tool::Ellipse)),
    (NONE, Key::L, Action::SetTool(Tool::Line)),
    (NONE, Key::A, Action::SetTool(Tool::Arrow)),
    (NONE, Key::D, Action::SetTool(Tool::Pen)),
    (NONE, Key::I, Action::InsertImage),
];

pub fn fmt_shortcut(m: Modifiers, k: Key) -> String {
    fmt_shortcut_for(cfg!(target_os = "macos"), m, k)
}

/// [`fmt_shortcut`] as one platform shows it (the README shows the Mac's).
pub fn fmt_shortcut_for(mac: bool, m: Modifiers, k: Key) -> String {
    let mut s = String::new();
    if m.ctrl && (!m.command || mac) {
        s.push_str(&if mac { "⌃".to_string() } else { format!("{}+", crate::t!("key-ctrl")) });
    }
    if m.alt {
        s.push_str(&if mac { "⌥".to_string() } else { format!("{}+", crate::t!("key-alt")) });
    }
    if m.shift {
        s.push_str(&if mac { "⇧".to_string() } else { format!("{}+", crate::t!("key-shift")) });
    }
    if m.command || m.mac_cmd {
        s.push_str(&if mac { "⌘".to_string() } else { format!("{}+", crate::t!("key-ctrl")) });
    }
    let key = match k {
        Key::Enter => "↵".to_string(),
        Key::Tab => if mac { "⇥".to_string() } else { crate::t!("key-tab") },
        Key::Escape => crate::t!("key-escape"),
        Key::Backspace => "⌫".to_string(),
        Key::ArrowRight => "→".into(),
        Key::ArrowLeft => "←".into(),
        Key::ArrowUp => "↑".into(),
        Key::ArrowDown => "↓".into(),
        Key::Delete => crate::t!("key-delete"),
        Key::Space => crate::t!("key-space"),
        Key::Num0 => "0".into(),
        Key::Num1 => "1".into(),
        Key::Num2 => "2".into(),
        Key::Period => ".".into(),
        Key::Equals => "=".into(),
        Key::Plus => "+".into(),
        Key::Minus => "−".into(),
        Key::OpenBracket => "[".into(),
        Key::CloseBracket => "]".into(),
        Key::Backslash => "\\".into(),
        Key::Comma => ",".into(),
        Key::Questionmark => "?".into(),
        Key::Exclamationmark => "!".into(),
        k => k.name().to_string(),
    };
    s.push_str(&key);
    s
}

/// Does a key event's modifier state match a shortcut pattern exactly
/// (with ⌘ on macOS == Ctrl elsewhere)?
pub fn mods_match(ev: Modifiers, pat: Modifiers) -> bool {
    // egui sets `command` = ⌘ on macOS and = Ctrl elsewhere.
    let mac = cfg!(target_os = "macos");
    // ⌃⌘ shortcuts exist only on the Mac (elsewhere ⌘ already is Ctrl).
    if !mac && pat.ctrl && pat.command {
        return false;
    }
    let mac_ctrl_ok = !mac || ev.ctrl == pat.ctrl;
    ev.shift == pat.shift && ev.alt == pat.alt && ev.command == pat.command && mac_ctrl_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape toolbar menu and the canvas right-click menu open the
    /// same window as Insert › Symbols & Shapes, under the same name.
    #[test]
    fn the_symbols_window_has_one_name() {
        assert_eq!(Action::ShapeLibrary.label(), Action::InsertSymbols.label());
    }
}
