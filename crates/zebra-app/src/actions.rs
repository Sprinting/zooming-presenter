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
    InsertImage,
    InsertLayout,
    WrapInFrame,
    /// Opens the shapes and icons library (stream 3 fills it in).
    ShapeLibrary,
    /// Insert › Video… (stream 2 fills it in).
    InsertVideo,
    /// Insert › Audio… (stream 2 fills it in).
    InsertAudio,
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
}

impl Action {
    pub fn label(&self) -> String {
        use Action::*;
        match self {
            New => "New".into(),
            NewFromSample => "New from Sample".into(),
            ShowTemplates => "New from Template…".into(),
            NewFromTemplate(t) => format!("New from Template: {}", zebra_core::templates::TEMPLATES.iter().find(|x| x.id == t).map(|x| x.name).unwrap_or(t)),
            NewFromTemplateFile(p) => p.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
            SaveAsTemplate => "Save as Template…".into(),
            Open => "Open…".into(),
            OpenPath(p) => p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
            Save => "Save".into(),
            SaveAs => "Save As…".into(),
            ExportPdf => "Export PDF…".into(),
            ExportPng => "Export View as PNG…".into(),
            Close => "Close".into(),
            Quit => "Quit Zebra".into(),
            Undo => "Undo".into(),
            Redo => "Redo".into(),
            Cut => "Cut".into(),
            Copy => "Copy".into(),
            Paste => "Paste".into(),
            Duplicate => "Duplicate".into(),
            Delete => "Delete".into(),
            SelectAll => "Select All".into(),
            Deselect => "Deselect".into(),
            SetTool(t) => format!("{} Tool", tool_name(*t)),
            FrameTool => "Frame Tool (cycle style)".into(),
            SetFrameStyle(s) => format!("{} Frame", frame_name(*s)),
            InsertImage => "Image…".into(),
            InsertLayout => "Layout…".into(),
            WrapInFrame => "Frame Selection".into(),
            ShapeLibrary => "Shapes and Icons…".into(),
            InsertVideo => "Video…".into(),
            InsertAudio => "Audio…".into(),
            Bold => "Bold".into(),
            Italic => "Italic".into(),
            Underline => "Underline".into(),
            BringForward => "Bring Forward".into(),
            SendBackward => "Send Backward".into(),
            BringToFront => "Bring to Front".into(),
            SendToBack => "Send to Back".into(),
            ToggleLock => "Lock".into(),
            Group => "Group".into(),
            Ungroup => "Ungroup".into(),
            ZoomIn => "Zoom In".into(),
            ZoomOut => "Zoom Out".into(),
            FitAll => "Zoom to Fit All".into(),
            ZoomToSelection => "Zoom to Selection".into(),
            ZoomReset => "Actual Size (100%)".into(),
            ToggleSidebar => "Show Sidebar".into(),
            ToggleProperties => "Show Inspector".into(),
            ToggleReduceMotion => "Reduce Motion".into(),
            CommandPalette => "Command Palette…".into(),
            ShowShortcuts => "Keyboard Shortcuts".into(),
            Settings => "Settings…".into(),
            TogglePathEdit => "Edit Path".into(),
            AddToPath => "Add Selection to Path".into(),
            AddCurrentView => "Add Current View to Path".into(),
            AutoPath => "Path from Frames".into(),
            ClearPath => "Clear Path…".into(),
            PresentStart => "Play Presentation".into(),
            PresentCurrent => "Play from Current Step".into(),
            TogglePresenterView => "Presenter View".into(),
            PlaybackOptions => "Playback Options…".into(),
            ToggleAiControl => "Let AI Agents Control Zebra".into(),
            ToggleActivity => "Agent Activity".into(),
            PauseAgents => "Pause / Resume Agent".into(),
            StopAgents => "Stop Agent".into(),
            FollowAgent => "Follow Agent".into(),
            RevertAgent => "Revert Agent's Changes…".into(),
            SetAgentPermission(p) => match p {
                zebra_core::view::Permission::Observe => "Agents May Look Only".into(),
                zebra_core::view::Permission::Edit => "Agents May Edit".into(),
                zebra_core::view::Permission::Full => "Agents Have Full Control…".into(),
            },
            SetTheme(t) => format!("Theme: {}", zebra_core::Theme::resolve(t).name),
            ThemeEditor => "Customize Theme…".into(),
            About => "About Zebra".into(),
        }
    }

    /// Human-readable shortcut, macOS glyphs.
    pub fn shortcut_text(&self) -> Option<String> {
        SHORTCUTS.iter().find(|s| &s.2 == self).map(|(m, k, _)| fmt_shortcut(*m, *k))
    }

    /// Actions listed in the ⌘K palette.
    pub fn palette() -> Vec<Action> {
        use Action::*;
        let mut v = vec![
            New, NewFromSample, ShowTemplates, SaveAsTemplate, Open, Save, SaveAs, ExportPdf, ExportPng, Undo, Redo, Cut, Copy, Paste, Duplicate, Delete,
            SelectAll, Deselect,
        ];
        for t in TOOLS {
            v.push(SetTool(t.0));
        }
        v.extend([
            InsertImage, InsertLayout, WrapInFrame, BringForward, SendBackward, BringToFront, SendToBack, ToggleLock, ZoomIn, ZoomOut,
            FitAll, ZoomToSelection, ZoomReset, ToggleSidebar, ToggleProperties, ShowShortcuts, Settings, ShapeLibrary,
            InsertVideo, InsertAudio, Group, Ungroup,
            TogglePathEdit, AddToPath, AddCurrentView, AutoPath, ClearPath, PresentStart, PresentCurrent, TogglePresenterView, PlaybackOptions, ToggleAiControl, ToggleActivity,
            PauseAgents, StopAgents, FollowAgent, RevertAgent, ThemeEditor,
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

pub fn tool_name(t: Tool) -> &'static str {
    match t {
        Tool::Select => "Select",
        Tool::Hand => "Hand",
        Tool::Text => "Text",
        Tool::Frame => "Frame",
        Tool::Rectangle => "Rectangle",
        Tool::Ellipse => "Ellipse",
        Tool::Triangle => "Triangle",
        Tool::Line => "Line",
        Tool::Arrow => "Arrow",
        Tool::Pen => "Pen",
        Tool::Highlighter => "Highlighter",
    }
}

pub fn frame_name(s: FrameStyle) -> &'static str {
    match s {
        FrameStyle::Bracket => "Bracket",
        FrameStyle::Circle => "Circle",
        FrameStyle::Rectangle => "Rectangle",
        FrameStyle::Invisible => "Invisible",
    }
}

const NONE: Modifiers = Modifiers::NONE;
const CMD: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;
const CMD_SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };
const CMD_ALT: Modifiers = Modifiers { alt: true, ..Modifiers::COMMAND };
const CMD_ALT_SHIFT: Modifiers = Modifiers { alt: true, shift: true, ..Modifiers::COMMAND };
const CMD_CTRL: Modifiers = Modifiers { ctrl: true, ..Modifiers::COMMAND };

/// Editor shortcuts (present mode has its own keys). Most specific first.
/// Matched exactly (Shift/Alt must match), with ⌘ = Ctrl off-Mac.
pub static SHORTCUTS: &[(Modifiers, Key, Action)] = &[
    // Keynote: Play ⌥⌘P; from the current step adds ⇧.
    (CMD_ALT_SHIFT, Key::P, Action::PresentCurrent),
    (CMD_ALT, Key::P, Action::PresentStart),
    (CMD_ALT, Key::I, Action::ToggleProperties),
    (CMD_ALT, Key::G, Action::Group),
    (CMD_ALT_SHIFT, Key::G, Action::Ungroup),
    (CMD_CTRL, Key::S, Action::ToggleSidebar),
    (CMD_SHIFT, Key::S, Action::SaveAs),
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
    (NONE, Key::Delete, Action::Delete),
    (NONE, Key::Backspace, Action::Delete),
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
    let mac = cfg!(target_os = "macos");
    let mut s = String::new();
    if m.ctrl && (!m.command || mac) {
        s.push_str(if mac { "⌃" } else { "Ctrl+" });
    }
    if m.alt {
        s.push_str(if mac { "⌥" } else { "Alt+" });
    }
    if m.shift {
        s.push_str(if mac { "⇧" } else { "Shift+" });
    }
    if m.command || m.mac_cmd {
        s.push_str(if mac { "⌘" } else { "Ctrl+" });
    }
    let key = match k {
        Key::Enter => "↵".to_string(),
        Key::Escape => "Esc".to_string(),
        Key::Backspace => "⌫".to_string(),
        Key::Delete => "Del".to_string(),
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
