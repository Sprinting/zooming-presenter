# Zebra: every string a person sees in the app (English, the source).
#
# Syntax: https://projectfluent.org/fluent/guide/
#   key = Text
#   key = { $name } joined
#   key = { $count ->
#       [one] { $count } step
#      *[other] { $count } steps
#   }
# Keys are lower-case-with-dashes, prefixed by where they appear
# (toolbar-, menu-, inspector-, sheet-, ...). Shortcuts are never written
# here: the app appends the live key binding.
#
# Sections below follow the source files. Keep each file's strings inside
# its own section.


## ---- app.rs ----

# This language's name in itself, shown in Settings › General › Language.
language-name = English

app-untitled = Untitled
# The second and later new documents of a run: "Untitled 2".
app-untitled-numbered = { $title } { $no }
app-agent-default-name = Agent
app-window-title-edited = { $name } — Edited
app-file-filter-zebra = Zebra presentation
app-file-filter-image = Image
app-file-filter-video = Video
app-file-filter-audio = Audio
app-file-filter-pdf = PDF
app-file-filter-png = PNG
app-ai-start-failed = Couldn't start AI control: { $error }
# When something a person did can't be done (details go to the log).
app-error-locked = It's locked. Unlock it first.
app-error-busy = An agent is in the middle of a change. Try again in a moment.
app-error-io = Couldn't read or write the file.
app-error-not-empty = The canvas isn't empty.
app-error-not-found = That's no longer there.
app-error-other = Couldn't do that.
app-toast-nothing-to-play = Nothing to play yet. Double-click the canvas to write something first.
app-toast-presenter-view-on = Presenter view will open when you present.
app-toast-presenter-view-off = Presenter view is off.
app-toast-agent-stopped = Agent stopped. Resume it from the AI button in the toolbar.
app-toast-read-failed = Could not read { $path }: { $error }
app-toast-read-dropped-failed = Could not read dropped file: { $error }
app-toast-agent-switched-ai-off = { $name } switched AI control off. Turn it back on from the AI button.
app-open-failed-title = The document “{ $name }” could not be opened.
app-open-failed-newer = It was made by a newer version of Zebra.
app-open-failed-not-zebra = It isn't a Zebra presentation, or it is damaged.
app-open-failed-missing = The file no longer exists.
app-open-failed-permission = You don't have permission to read it.
app-open-failed-unreadable = The file couldn't be read.
app-save-failed-title = The document “{ $name }” could not be saved.
app-undo-lock = Lock
app-undo-unlock = Unlock
app-undo-paste = Paste
app-undo-add-to-path = Add to Path
app-undo-add-current-view = Add Current View
app-undo-nudge = Nudge

## ---- end app.rs ----


## ---- panels.rs ----

status-no-agent = No agent
status-agent-idle = { $name }: idle
status-agent-paused = { $name }: paused
status-agent-action = { $name }: { $action }
status-agent-note = { $name }: { $note }
status-hide-note = Hide
status-activity = Activity
toolbar-add-step = Add Step
toolbar-add-step-tip-view = Add the current view as a step
toolbar-add-step-tip-selection = Add the selection as a step
toolbar-format = Format
toolbar-format-tip-hide = Hide the inspector
toolbar-format-tip-show = Show the inspector
toolbar-export = Export
toolbar-export-tip = Export as PDF or PNG
toolbar-export-pdf = PDF…
toolbar-export-png = PNG of Current View…
toolbar-theme = Theme
toolbar-theme-tip = Change the presentation's colours and fonts
toolbar-view = View
toolbar-view-tip = Show or hide the path sidebar and inspector
toolbar-view-path-sidebar = Path Sidebar
toolbar-view-inspector = Inspector
toolbar-view-agent-activity = Agent Activity
toolbar-view-edit-path = Edit Path
toolbar-zoom = Zoom
toolbar-zoom-level = Zoom { $level }
# The zoom level: "100%".
toolbar-zoom-percent = { $percent }%
toolbar-zoom-to-fit = Zoom to Fit
toolbar-zoom-to-selection = Zoom to Selection
toolbar-zoom-in = Zoom In
toolbar-zoom-out = Zoom Out
toolbar-play = Play
toolbar-play-tip = Play from the start
toolbar-play-tip-detail = Right-click to play from the current step
toolbar-play-from-start = Play from Start
toolbar-play-from-current = Play from Current Step
toolbar-presenter-view = Presenter View
toolbar-playback-options = Playback Options…
toolbar-text = Text
toolbar-text-tip-detail = Or double-click anywhere on the canvas
toolbar-frame = Frame
toolbar-frame-tip-detail = Press { $key } again to change the style
toolbar-frame-selection = Frame Selection
toolbar-shape = Shape
toolbar-shape-tip = Shapes and icons
toolbar-shape-rectangle = Rectangle
toolbar-shape-ellipse = Ellipse
toolbar-line = Line
toolbar-line-tip = Lines, arrows and drawing
toolbar-line-arrow = Arrow
toolbar-line-draw = Draw
toolbar-media = Media
toolbar-media-tip =
    Images, video and audio
    Or drop files on the canvas
toolbar-media-image = Image…
toolbar-media-video = Video…
toolbar-media-audio = Audio…
toolbar-theme-my-themes = My Themes
toolbar-theme-customize = Customize Theme…
toolbar-theme-customize-tip = Colours, fonts, background picture and 3D background
ai-toolbar = AI
ai-toolbar-error = AI: Error
ai-tip-off = Let an AI agent work in Zebra
ai-tip-waiting = AI control is on, waiting for an agent
ai-tip-paused = Agent paused. Click for agent controls
ai-tip-connected = Agent connected. Click for agent controls
ai-control-switch = Let AI agents control Zebra
ai-control-switch-tip = Only you can switch this on. An agent can switch it off.
ai-caption-on = Agents on this Mac that can run a command (Claude Code, Codex, Cursor, Gemini CLI, your own scripts) can now see and edit this window. You can watch, undo, pause or stop them at any time.
ai-caption-off = Off: nothing outside Zebra can see or change your work. Switch it on to let an AI agent on this Mac build or edit presentations with you.
ai-connect-title = Connect an Agent
ai-connect-paste = Paste this into your agent's chat:
ai-zebractl-installed = ✓ zebractl is installed on this Mac
ai-install-cli = Install Command-Line Tool…
ai-install-cli-tip = Adds `zebractl` to /usr/local/bin so agents can type just `zebractl`. macOS asks for your password.
ai-toast-cli-installed = Installed zebractl. Agents can now run `zebractl --help`.
ai-toast-cli-install-failed = Couldn't install zebractl: { $error }
ai-mcp-setup = MCP Setup…
ai-mcp-caption = Optional. For apps that take an MCP server config (Claude Desktop, Cursor, and others):
ai-no-agent = No agent connected yet.
ai-pause = Pause
ai-resume = Resume
ai-pause-tip = Temporarily stop the agent from making changes
ai-revert-changes = Revert Changes…
ai-revert-tip = Undo everything the agent did since your last change
ai-follow-camera = Follow the agent's camera
ai-follow-tip = Move your view to wherever the agent is working
ai-show-activity = Show Activity
ai-permission-title = What agents may do
ai-permission-observe = Look only
ai-permission-edit = Edit the presentation (default)
ai-permission-full = Full control (also your view, files, presenting)
copy-box-copy = Copy
copy-box-copied = Copied
canvas-hint-title = Double-click anywhere to write
canvas-hint-detail = Press { $key } for a frame, drop images here, or choose File › New from Sample
canvas-back-to-content = Back to Content
canvas-back-to-content-tip = Zoom to fit everything
recover-message = Unsaved changes from an earlier session were found.
recover-dismiss = Dismiss
recover-restore = Restore
activity-title = Agent Activity
activity-close = Close
activity-caption = Every change an agent makes appears here and on your undo stack.
activity-pause = Pause
activity-resume = Resume
activity-pause-tip = Temporarily stop the agent from making changes
activity-resume-tip = Let the agent continue
activity-revert = Revert…
activity-revert-tip = Undo everything the agent did since your last change
activity-follow = Follow the agent
activity-follow-tip = Move your view to wherever the agent is working
activity-empty = No agent changes yet.
activity-connected = Connected
activity-none-connected = No agent connected.
activity-seconds-ago = { $seconds } s ago
activity-minutes-ago = { $minutes } min ago
activity-item-meta = { $origin } · { $when } · { $count ->
        [one] 1 object
       *[other] { $count } objects
    }
activity-item-meta-plain = { $origin } · { $when }
activity-item-tip = Click to show these objects
# Who made a change, when it was the person using this Mac.
origin-you = You
palette-hint = Type a command
shortcuts-title = Keyboard Shortcuts
shortcuts-caption = Every shortcut is also a menu item or a button.
shortcuts-write-text = Write text
shortcuts-write-text-keys = Double-click anywhere
shortcuts-select-hand = Select tool · Hand tool
shortcuts-select-hand-keys = { $select } · { $hand } (or hold { $space })
shortcuts-text-frame = Text · Frame (again to change style)
shortcuts-shapes = Rectangle · Ellipse · Line · Arrow · Draw
shortcuts-insert-image = Insert image
shortcuts-pan-zoom = Pan · Zoom
shortcuts-pan-zoom-keys = Two-finger scroll · Pinch, or { $modifier }-scroll
shortcuts-zoom = Zoom to Fit · Zoom to Selection · Actual Size
shortcuts-undo-redo = Undo · Redo
shortcuts-duplicate-delete = Duplicate · Delete
shortcuts-forward-backward = Bring Forward · Send Backward
shortcuts-rotate = Rotate in 15° steps
shortcuts-rotate-keys = Hold { $modifier } while rotating
shortcuts-path = Edit Path · Add to Path
shortcuts-play = Play · Play from Current Step
shortcuts-while-playing = While playing
shortcuts-while-playing-keys = { $next } or { $space } next · { $back } back · { $home } overview · { $black } or { $white } blank · { $stop } stop
shortcuts-panels = Show Inspector · Show Sidebar
shortcuts-agent = Agent Activity · Stop Agent
shortcuts-settings = Settings
shortcuts-done = Done
about-name = Zebra
about-version = Version { $version }
about-blurb = A zooming canvas for presentations, in the spirit of classic Prezi. Drivable by AI agents through an opt-in local API.

## ---- end panels.rs ----


## ---- sidebar.rs ----

sidebar-untitled-frame = Untitled Frame
sidebar-missing-step = (missing)
sidebar-step-view = View
sidebar-step-overview = Overview
sidebar-path = Path
sidebar-step-count = { $count ->
    [one] { $count } step
   *[other] { $count } steps
}
sidebar-done = Done
sidebar-edit-path = Edit Path
sidebar-done-tip = Finish editing the path
sidebar-edit-path-tip = Click objects on the canvas to add them as steps, in order
sidebar-no-steps = No Steps
sidebar-no-steps-caption = Select a frame and click Add Step in the toolbar, or click Edit Path and then click frames in order.
sidebar-build-count = { $count ->
    [one] { $count } build
   *[other] { $count } builds
}
sidebar-step-tip =
    Step { $number }: { $label }
    Click to go there. Drag to reorder. Right-click for more.
sidebar-go-to-step = Go to Step
sidebar-play-from-here = Play from Here
sidebar-select-on-canvas = Select on Canvas
sidebar-show-fade-ins-dimmed = Show Later Fade-ins Dimmed
sidebar-duplicate-step = Duplicate Step
sidebar-move-to-start = Move to Start
sidebar-move-to-end = Move to End
sidebar-remove-from-path = Remove from Path
sidebar-add-tip-view = Add the current view as a step
sidebar-add-tip-selection = Add the selection as a step
sidebar-remove-tip = Remove the selected step
sidebar-undo-remove-step = Remove Step
sidebar-more-tip = More path actions
sidebar-path-from-frames = Path from Frames
sidebar-add-current-view = Add Current View
sidebar-clear-path = Clear Path…

## ---- end sidebar.rs ----


## ---- settings.rs ----

settings-title = Settings
settings-general = General
settings-canvas = Canvas
settings-presenting = Presenting
settings-agents = Agents
settings-keyboard = Keyboard
settings-keys-caption = Click a shortcut, then press the new keys. Esc cancels.
settings-keys-type = Type keys…
settings-keys-none = None
settings-keys-remove = Remove shortcut
settings-keys-moved = { $keys } was { $action }'s shortcut. { $action } has none now.
settings-keys-taken = { $keys } is { $action }. It can't be changed.
settings-keys-restore = Restore Defaults
settings-keys-search = Search
settings-keys-no-results = No Results
settings-keys-group-tools = Tools
settings-keys-group-other = Other
settings-default-theme = Theme for new presentations:
settings-language = Language:
settings-language-system = System ({ $language })
settings-appearance = Appearance:
settings-appearance-caption = Zebra follows the system's Light or Dark appearance and accent colour. The canvas uses the presentation's theme.
settings-scrolling = Scrolling:
settings-scroll-auto = Trackpad pans, mouse wheel zooms
settings-scroll-zoom = Always zoom (classic Prezi)
settings-scroll-pan = Always pan; { $modifier }-scroll zooms
settings-snapping = Snapping:
settings-snap-to-objects = Snap to other objects while moving
settings-snap-caption = Hold { $modifier } while dragging to move freely.
settings-reduce-motion = Reduce motion:
settings-follow-system = Follow System Setting
settings-on = On
settings-off = Off
settings-reduce-motion-caption = On: steps cross-fade instead of flying and rotating.
settings-ai-control = AI control:
settings-ai-switch = Let AI agents control Zebra
settings-ai-switch-caption = Only you can switch this on. An agent can switch it off.
settings-agents-may = Agents may:
settings-permission-observe = Look only
settings-permission-edit = Edit the presentation
settings-permission-full = Also move the view, open and save files, and present
settings-cli = Command-line tool:
settings-cli-installed = zebractl is installed in /usr/local/bin.
settings-cli-install = Install zebractl…
settings-toast-cli-installed = Installed zebractl.
settings-toast-cli-install-failed = Couldn't install zebractl: { $error }

## ---- end settings.rs ----


## ---- welcome.rs ----

welcome-title = Choose a Template
welcome-my-templates = My Templates
welcome-recent = Recent
welcome-recent-empty = Nothing opened yet.
welcome-open = Open…
welcome-create = Create
welcome-cancel = Cancel
welcome-toast-template-open-failed = Couldn't open the template: { $error }
welcome-toast-templates-folder-missing = Couldn't find the templates folder.
welcome-toast-template-saved = Saved “{ $name }” to My Templates. Find it in File ▸ New from Template…
welcome-toast-template-save-failed = Couldn't save the template: { $error }
layouts-title = Layouts
layouts-caption = Click a layout to add it to the middle of the view. Its frames join the path.

## ---- end welcome.rs ----


## ---- sheets.rs ----

sheet-save-changes-title = Do you want to save the changes made to the document “{ $name }”?
sheet-save-changes-message = Your changes will be lost if you don't save them.
sheet-save = Save
sheet-dont-save = Don't Save
sheet-cancel = Cancel
sheet-ok = OK
sheet-clear-path-title = Clear the path?
sheet-clear-path-message = This removes { $count ->
    [one] the only step
   *[other] all { $count } steps
} from the path. Frames and objects stay on the canvas. You can undo this.
sheet-clear-path = Clear Path
sheet-auto-path-title = Replace the path?
sheet-auto-path-message = Steps that are not frames have speaker notes. The notes go with them. You can undo this.
sheet-auto-path = Replace Path
sheet-revert-agent-title = Revert the changes { $name } made?
sheet-revert-agent-message = This undoes everything { $name } did since your last change. Redo ({ $shortcut }) brings it back.
sheet-revert = Revert
sheet-full-control-title = Give agents full control?
sheet-full-control-message = Agents will also be able to move your view, open and save files, and start presenting.
sheet-allow-full-control = Allow Full Control
sheet-toast-nothing-to-revert = { $name } has no changes to revert.

## ---- end sheets.rs ----


## ---- files.rs ----

export-toast-exporting = Exporting…
export-toast-exported = Exported { $path }
export-toast-failed = Export failed: { $error }

## ---- end files.rs ----


## ---- cli.rs ----

cli-agent-prompt = Zebra (a zooming presentation app) is open on my Mac and I've allowed AI control. Run `{ $ctl } --help` to learn how to drive it, then its `describe` command to see what's on the canvas. Pass `--agent` with your name on every command.
cli-install-prompt = Zebra wants to install the zebractl command so AI agents can use it.
cli-error-not-bundled = zebractl isn't inside this copy of Zebra
cli-error-cancelled = cancelled

## ---- end cli.rs ----


## ---- inspector.rs ----
inspector-tab-style = Style
inspector-tab-text = Text
inspector-tab-arrange = Arrange
inspector-tab-path = Path
inspector-undo-change-properties = Change Properties
# Header: what is selected. $style is the frame style name.
inspector-kind-frame = { $style } Frame
inspector-kind-title = Title
inspector-kind-subtitle = Subtitle
inspector-kind-text = Text
inspector-kind-icon = Icon · { $name }
inspector-kind-video = Video
inspector-kind-connector = Connector
inspector-kind-image = Image
inspector-kind-symbol = Symbol
inspector-kind-drawing = Drawing
inspector-kind-object = Object
inspector-group-of = Group of { $count }
inspector-objects-count = { $count ->
    [one] { $count } Object
   *[other] { $count } Objects
}
inspector-locked = Locked
inspector-element-id = ID { $id }
# Colour swatch tooltips
inspector-swatch-theme-frame = Theme frame colour
inspector-swatch-theme-default = Theme default
inspector-swatch-none = None
inspector-swatch-accent = Accent
inspector-swatch-colour-1 = Colour 1
inspector-swatch-colour-2 = Colour 2
inspector-swatch-colour-3 = Colour 3
inspector-swatch-colour-4 = Colour 4
inspector-swatch-title = Title
inspector-swatch-subtitle = Subtitle
inspector-swatch-body = Body
inspector-swatch-text = Text
inspector-swatch-frame = Frame
inspector-swatch-background = Background
inspector-custom-colour = Custom
# Style tab
inspector-section-frame = Frame
inspector-name = Name
inspector-frame-contains = { $count ->
    [one] Contains { $count } object.
   *[other] Contains { $count } objects.
}
inspector-section-colour = Colour
inspector-section-shape = Shape
inspector-section-fill = Fill
inspector-section-line = Line
inspector-section-border = Border
inspector-width = Width
inspector-section-image = Image
inspector-description = Description
inspector-alt-text-hint = Alt text
inspector-alt-text-caption = Read aloud by VoiceOver and used in exported PDFs.
inspector-section-drawing = Drawing
# Text tab
inspector-para-title = Title
inspector-para-subtitle = Subtitle
inspector-para-body = Body
inspector-section-font = Font
inspector-font-from-theme = { $font } (from the theme)
inspector-bold = Bold
inspector-italic = Italic
inspector-section-alignment = Alignment
inspector-align-left = Align Left
inspector-align-centre = Centre
inspector-align-right = Align Right
inspector-bullets = Bullets
inspector-bullets-none = None
inspector-bullets-on = Bullets
inspector-undo-format-paragraphs = Format Paragraphs
inspector-edit-text = Edit Text
# $key is the Return key glyph.
inspector-edit-text-tip = Edit the text in place  { $key } or double-click
# Arrange tab
inspector-section-size = Size
inspector-height = Height
inspector-size-caption = Width and height keep the proportions, as in Prezi.
inspector-section-rotate = Rotate
inspector-angle = Angle
inspector-section-position = Position
inspector-pos-x = X
inspector-pos-y = Y
inspector-position-caption = Canvas coordinates of the centre.
inspector-section-layer = Layer
inspector-forward = Forward
inspector-forward-tip = Bring forward one layer
inspector-backward = Backward
inspector-backward-tip = Send backward one layer
inspector-to-front = To Front
inspector-to-front-tip = Bring to front
inspector-to-back = To Back
inspector-to-back-tip = Send to back
inspector-unlock = Unlock
inspector-unlock-tip = Unlock so it can be moved and edited
inspector-lock = Lock
inspector-lock-tip = Lock in place
inspector-duplicate = Duplicate
inspector-duplicate-tip = Duplicate the selection
inspector-group = Group
inspector-group-tip = Select, move and resize these together
inspector-ungroup = Ungroup
inspector-ungroup-tip = Separate the group
inspector-frame-selection = Frame Selection
inspector-frame-selection-tip = Draw a frame around the selection
inspector-section-align = Align
inspector-align-edge-left = Left
inspector-align-edge-centre = Centre
inspector-align-edge-right = Right
inspector-align-edge-top = Top
inspector-align-edge-middle = Middle
inspector-align-edge-bottom = Bottom
inspector-section-arrange = Arrange
inspector-arrange-row = Row
inspector-arrange-row-tip = Arrange in a row
inspector-arrange-column = Column
inspector-arrange-column-tip = Arrange in a column
inspector-arrange-grid = Grid
inspector-arrange-grid-tip = Arrange in a grid
inspector-arrange-circle = Circle
inspector-arrange-circle-tip = Arrange in a circle
# Path tab
inspector-section-path = Path
inspector-not-on-path-multi = None of these objects is on the path.
inspector-not-on-path = Not on the path.
inspector-add-to-path = Add to Path
inspector-add-to-path-tip = Add a step that zooms to the selection
# $list is the step numbers, e.g. "2, 5".
inspector-on-path-steps = { $count ->
    [one] On the path as step { $list }.
   *[other] On the path as steps { $list }.
}
inspector-remove-from-path = Remove from Path
inspector-remove-from-path-tip = Remove every step that zooms to this
inspector-undo-remove-from-path = Remove from Path
inspector-section-animation = Animation
inspector-fade-in-on-step = Fade in on step { $step }
inspector-fade-in-tip = Appears with a click after the camera arrives
# Nothing selected: presentation settings
inspector-presentation = Presentation
inspector-doc-title = Title
inspector-untitled = Untitled
inspector-section-theme = Theme
inspector-path-steps = { $count ->
    [0] No steps yet. Select a frame and choose Add to Path, or use Edit Path.
    [one] { $count } step.
   *[other] { $count } steps.
}
inspector-edit-path = Edit Path
inspector-edit-path-tip = Click objects to add steps
inspector-from-frames = From Frames
inspector-from-frames-tip = Replace the path with every frame, in reading order

## ---- end inspector.rs ----


## ---- inspector_s3.rs ----
inspector-dash-solid = Solid
inspector-dash-dashed = Dashed
inspector-dash-dotted = Dotted
inspector-section-outline = Outline
inspector-corners = Corners
inspector-section-icon = Icon
inspector-replace-icon = Replace…
inspector-replace-icon-tip = Pick another icon from the library
inspector-weight = Weight
inspector-section-route = Route
inspector-route-straight = Straight
inspector-route-elbow = Elbow
inspector-route-curved = Curved
inspector-head-none = None
inspector-head-arrow = Arrow
inspector-head-open-arrow = Open Arrow
inspector-head-dot = Dot
inspector-head-start = Start
inspector-head-end = End
inspector-connector-label = Label
inspector-connector-label-hint = Text on the line
inspector-connector-attached = { $attached } of 2 ends attached. Drag an end onto an object to stick it there.
inspector-section-video = Video

## ---- end inspector_s3.rs ----


## ---- shape_library.rs ----
library-window-title = Symbols & Shapes
library-window-replace-icon = Replace Icon
library-tab-shapes = Shapes
library-tab-icons = Icons
library-tab-symbols = Symbols
library-search-shapes-hint = Search shapes
library-search-icons-hint = Search icons: idea, chart, arrow…
library-search-symbols-hint = Search symbols: rocket, cat, trophy…
library-no-shapes = No shapes match.
library-no-icons = No icons match.
library-icons-count = { $count ->
    [one] { $count } icon · Lucide (ISC licence)
   *[other] { $count } icons · Lucide (ISC licence)
}

## ---- end shape_library.rs ----


## ---- theme_editor.rs ----
# Name of a built-in theme copied for editing.
theme-editor-custom-name = { $name } (custom)
theme-editor-window-title = Theme
theme-editor-undo-edit = Edit Theme
theme-editor-name = Name
theme-editor-start-from = Start from
theme-editor-choose-theme = Choose a theme…
theme-editor-section-colours = Colours
theme-editor-colour-background = Background
theme-editor-colour-title = Title
theme-editor-colour-subtitle = Subtitle
theme-editor-colour-body-text = Body text
theme-editor-colour-frames = Frames
theme-editor-colour-frame-fill = Frame fill
theme-editor-colour-accent = Accent
theme-editor-colour-1 = Colour 1
theme-editor-colour-2 = Colour 2
theme-editor-colour-3 = Colour 3
theme-editor-colour-4 = Colour 4
theme-editor-section-fonts = Fonts
theme-editor-font-title = Title
theme-editor-font-subtitle = Subtitle
theme-editor-font-body = Body
theme-editor-font-tip = Any installed font. It's embedded in the file when you save, so it shows on other Macs too.
# The four built-in font families, in the font list and as the chosen font.
theme-editor-font-sans = Sans
theme-editor-font-serif = Serif
theme-editor-font-mono = Mono
theme-editor-font-hand = Hand
theme-editor-font-sans-shown = Sans (Inter)
theme-editor-font-serif-shown = Serif (Lora)
theme-editor-font-mono-shown = Mono (JetBrains Mono)
theme-editor-font-hand-shown = Handwriting (Caveat)
theme-editor-search-fonts = Search fonts
# Unit after the font size (points).
theme-editor-unit-pt = pt
theme-editor-embedded-fonts = Embedded in the file: { $fonts }
theme-editor-section-background = Background
theme-editor-picture = Picture
theme-editor-replace = Replace…
theme-editor-choose = Choose…
theme-editor-remove = Remove
theme-editor-3d-background = 3D background
theme-editor-3d-background-tip = Up to three pictures behind the canvas that move slower than it as you zoom and pan, for depth.
theme-editor-layer = Layer { $number }
theme-editor-depth = depth
theme-editor-depth-tip = Left: far away, barely moves. Right: moves with the canvas.
theme-editor-opacity = opacity
theme-editor-remove-layer = Remove layer
theme-editor-remove-layer-button = ✕
theme-editor-add-layer = Add Layer…
theme-editor-save-as-theme = Save as Theme…
theme-editor-save-as-theme-tip = Keep this theme for other presentations
theme-editor-saved = Saved “{ $name }” to My Themes.
theme-editor-save-failed = Couldn't save the theme: { $error }
theme-editor-no-themes-folder = Couldn't find the themes folder.
theme-editor-done = Done
theme-editor-read-failed = Couldn't read { $path }: { $error }

## ---- end theme_editor.rs ----


## ---- widgets.rs ----
# Accessibility name of a colour well.
widgets-colour-well = Colour
# Units after numbers in number fields.
widgets-unit-pt = pt
widgets-unit-seconds = s
widgets-unit-percent = %
widgets-unit-degrees = °

## ---- end widgets.rs ----


## ---- library.rs ----
# File name used when a theme or template has no name.
library-file-untitled = Untitled

## ---- end library.rs ----


## ---- style.rs ----


## ---- end style.rs ----


## ---- icons.rs ----
# The letter drawn on the Bold button (some languages use another, e.g. F).
icons-bold-letter = B

## ---- end icons.rs ----


## ---- actions.rs ----
action-new = New
action-new-from-sample = New from Sample
action-show-templates = New from Template…
action-save-as-template = Save as Template…
action-open = Open…
action-save = Save
action-save-as = Save As…
action-export-pdf = Export as PDF…
action-export-png = Export Current View as PNG…
action-close = Close
action-quit = Quit Zebra
action-undo = Undo
action-redo = Redo
action-cut = Cut
action-copy = Copy
action-paste = Paste
action-duplicate = Duplicate
action-delete = Delete
action-select-all = Select All
action-deselect = Deselect All
action-frame-tool = Frame Tool
action-insert-frame = Frame
action-insert-image = Image…
action-insert-layout = Layout…
action-insert-diagram = Diagram…
action-insert-symbols = Shapes, Icons & Symbols…
action-insert-music = Background Music…
action-replace-image = Replace Image…
action-crop-image = Crop Image
action-play-pause-video = Play/Pause Video
action-wrap-in-frame = Frame Selection
action-insert-video = Video…
action-bold = Bold
action-italic = Italic
action-underline = Underline
action-bring-forward = Bring Forward
action-send-backward = Send Backward
action-bring-to-front = Bring to Front
action-send-to-back = Send to Back
action-toggle-lock = Lock
action-group = Group
action-ungroup = Ungroup
action-zoom-in = Zoom In
action-zoom-out = Zoom Out
action-fit-all = Zoom to Fit
action-zoom-to-selection = Zoom to Selection
action-zoom-reset = Actual Size
action-toggle-sidebar = Show Sidebar
action-toggle-properties = Show Inspector
action-toggle-reduce-motion = Reduce Motion
action-command-palette = Command Palette…
action-show-shortcuts = Keyboard Shortcuts
action-settings = Settings…
action-toggle-path-edit = Edit Path
action-add-to-path = Add Selection to Path
action-add-current-view = Add Current View to Path
action-auto-path = Create Path from Frames
action-clear-path = Clear Path…
action-present-start = Play from Start
action-present-current = Play from Current Step
action-toggle-presenter-view = Presenter View
action-playback-options = Playback Options…
action-toggle-ai-control = Allow AI Control
action-toggle-activity = Agent Activity
action-pause-agents = Pause or Resume Agent
action-stop-agents = Stop Agent
action-follow-agent = Follow Agent
action-revert-agent = Revert Agent's Changes…
action-theme-editor = Customise Theme…
action-about = About Zebra
action-new-from-template = New from Template: { $name }
action-set-tool = { $tool } Tool
action-set-frame-style = { $style } Frame
action-agent-permission-observe = Agents Can Only View
action-agent-permission-edit = Agents Can Edit
action-agent-permission-full = Give Agents Full Control…
action-set-theme = Theme: { $name }
tool-select = Select
tool-hand = Hand
tool-text = Text
tool-frame = Frame
tool-rectangle = Rectangle
tool-ellipse = Oval
tool-triangle = Triangle
tool-line = Line
tool-arrow = Arrow
tool-pen = Pen
tool-highlighter = Highlighter
frame-style-bracket = Bracket
frame-style-circle = Circle
frame-style-rectangle = Rectangle
frame-style-invisible = Invisible

# Key names in shortcuts ("Esc", "Ctrl+S" off the Mac).
key-escape = Esc
key-delete = Del
key-tab = Tab
key-space = Space
key-ctrl = Ctrl
key-alt = Alt
key-shift = Shift

## ---- end actions.rs ----


## ---- menus.rs ----
menu-file = File
menu-export-to = Export To
menu-edit = Edit
menu-insert = Insert
menu-frame = Frame Style
menu-shape = Shape
menu-format = Format
menu-font = Font
menu-theme = Theme
menu-arrange = Arrange
menu-view = View
menu-path = Path
menu-play = Play
menu-agent = Agent
menu-help = Help
# $label is the undo entry's name, e.g. "Rotate"; $agent the agent's name.
menu-undo-named = Undo { $label }
menu-undo-by-agent = Undo { $label } ({ $agent })
menu-redo-named = Redo { $label }
menu-redo-by-agent = Redo { $label } ({ $agent })
menu-unlock = Unlock
menu-hide-sidebar = Hide Sidebar
menu-hide-inspector = Hide Inspector
menu-show-agent-activity = Show Agent Activity
menu-hide-agent-activity = Hide Agent Activity
menu-resume-agent = Resume Agent
menu-pause-agent = Pause Agent
menu-open-recent = Open Recent
menu-no-recent-documents = No Recent Documents

## ---- end menus.rs ----


## ---- menu_mac.rs ----
menu-mac-app = Zebra
menu-mac-window = Window
# The standard macOS items (AppKit does what they do; these are their names).
menu-mac-services = Services
menu-mac-hide = Hide Zebra
menu-mac-hide-others = Hide Others
menu-mac-show-all = Show All
menu-mac-minimize = Minimise
menu-mac-zoom = Zoom
menu-mac-full-screen = Enter Full Screen
menu-mac-bring-all-to-front = Bring All to Front

## ---- end menu_mac.rs ----


## ---- canvas.rs ----
canvas-chip-zoom-into-frame = Zoom into Frame
canvas-chip-rename-hint = Double-click to rename
canvas-toast-rename-failed = Couldn't rename the frame.
canvas-undo-move-connector-end = Move Connector End
canvas-undo-rotate = Rotate
canvas-undo-resize = Resize
canvas-undo-move = Move
canvas-undo-add-text = Add Text
canvas-undo-edit-text = Edit Text
# $step is the 1-based step number.
canvas-toast-step-retargeted = Step { $step } now points here
canvas-toast-framed-objects = { $count ->
    [one] Framed { $count } object
   *[other] Framed { $count } objects
}
canvas-toast-added-step = Added step { $step }
canvas-toast-shape-locked = This shape is locked — unlock it to type on it
canvas-toast-text-locked = This text is locked — unlock it to edit
canvas-toast-text-kept = Text box was removed. Your text is kept as a new one.
canvas-text-placeholder = Type something
canvas-text-style-title = Title
canvas-text-style-subtitle = Subtitle
canvas-text-style-body = Body
canvas-text-bold = Bold
canvas-text-italic = Italic
canvas-text-underline = Underline
canvas-text-strikethrough = Strikethrough
# Letters drawn on the text-format buttons, styled as bold, italic,
# underlined and struck through.
canvas-text-bold-glyph = B
canvas-text-italic-glyph = I
canvas-text-underline-glyph = U
canvas-text-strikethrough-glyph = S
canvas-text-bigger-glyph = A+
canvas-text-smaller-glyph = A−
canvas-link-hint = https://…
canvas-text-bigger = Bigger
canvas-text-smaller = Smaller
canvas-text-colour = Colour
canvas-colour-default = Default
canvas-colour-title = Title
canvas-colour-accent = Accent
canvas-colour-n = Colour { $n }
canvas-text-link = Link
canvas-text-bulleted-list = Bulleted list
canvas-text-numbered-list = Numbered list
canvas-text-decrease-indent = Decrease indent
canvas-text-increase-indent = Increase indent
canvas-text-done = Done
canvas-link-to = Link to
canvas-link-apply = Apply
canvas-link-remove = Remove
canvas-agent-editing-here = { $name } is editing here
canvas-locked = Locked
# $modifier is the modifier key glyph (⇧).
canvas-rotate-hint = Rotate  ·  { $modifier } snaps to 15°
# Beside the rotate handle while turning: "45°".
canvas-rotate-angle = { $degrees }°
canvas-menu-add-text-here = Add Text Here
canvas-menu-add-text = Add Text
canvas-menu-edit-text = Edit Text
canvas-menu-replace-icon = Replace Icon…

## ---- end canvas.rs ----


## ---- presenting.rs ----
present-notes-hint = Speaker notes: what to say at this step
# $step is the 1-based step number.
present-notes-step = Notes · step { $step }
present-path-step = Path step { $step }
present-transition = Transition
present-transition-auto = Auto
present-transition-auto-tip = Zebra picks a time from the distance flown
present-autoplay = Autoplay
# $time is a duration such as "10 s".
present-dwell-default-time = Default ({ $time })
present-dwell-default-off = Default (off)
present-dwell-own = Own time
present-dwell-presentation-default = Presentation default
present-dwell-own-for-step = Own time for this step
present-undo-change-step-timing = Change Step Timing
# A number of seconds, $secs already formatted ("10", "1.5").
present-seconds = { $secs } s
# Unit after a seconds field.
present-seconds-unit = s
present-autoplay-off = Off
# $key is the pause key.
present-autoplay-help = Steps with their own time keep it. Press { $key } while presenting to pause.
present-loop = Loop: start again after the last step
present-presenting = Presenting
present-show-presenter-view = Show presenter view
present-show-presenter-view-tip = Notes, the next step and a timer in a second window, on another display
present-this-display = This window's display
present-play-on = Play on
present-connect-second-display = Connect a second display to present on one and see the presenter view on the other.
present-playback-window = Playback
present-built-in-display = Built-in display
present-display-n = Display { $n }
present-toast-autoplay-paused = Autoplay paused ({ $key } to resume)
present-toast-autoplay-resumed = Autoplay resumed
present-presenter-window-title = Presenter View
present-presenter-step-of = Step { $step } of { $count }
present-presenter-overview = Overview · { $count ->
    [one] 1 step
   *[other] { $count } steps
}
present-presenter-autoplay-paused = Autoplay paused
present-presenter-autoplay = Autoplay { $time }
present-timer-reset = Reset
present-timer-reset-tip = Restart the timer
present-timer-resume = Resume
present-timer-pause = Pause
present-timer-resume-tip = Resume the timer
present-timer-pause-tip = Pause the timer
# $build of $count fade-ins on the current step.
present-next-build = Next: fade-in { $build } of { $count }
present-next-step = Next: step { $step }
present-next = Next
present-back-to-start = Back to step 1
present-end-of-presentation = End of presentation
present-no-notes = No notes for this step.
present-previous = Previous
present-notes-smaller = Smaller notes
present-notes-larger = Larger notes
present-end-show = End Show
present-rendering = Rendering…

## ---- end presenting.rs ----


## ---- present.rs ----
present-toast-opened-link = Opened { $url }
# Presentation HUD. $key is the key glyph that returns to the path (→).
present-hud-off-path = { $step } of { $count }  ·  { $key } returns to the path
present-hud-step-of = { $step } of { $count }
present-hud-overview = Overview  ·  { $count ->
    [one] 1 step
   *[other] { $count } steps
}
# $key is the exit key name (Esc).
present-hud-exit = { $key } to exit
present-hud-previous-step = Previous Step
present-hud-next-step = Next Step

## ---- end present.rs ----


## ---- media_ui.rs ----
media-toast-video-macos-only = Videos play in Zebra on macOS. Here you see the poster frame.
# $error is the reason from the media player.
media-error-play-video = Couldn't play the video: { $error }
media-error-play-soundtrack = Couldn't play the soundtrack: { $error }
media-error-play-voiceover = Couldn't play the voice-over: { $error }
# Text put on the system clipboard when copying objects that have no text.
media-clipboard-objects = { $count ->
    [one] { $count } Zebra object
   *[other] { $count } Zebra objects
}
media-pasted-image = Pasted image
media-toast-unlock-to-crop = Unlock the image to crop it.
media-undo-crop = Crop
# $done_key and $cancel_key are key glyphs (↵, Esc).
media-crop-bar = Drag edges to crop · { $done_key } Done · { $cancel_key } Cancel
media-crop-done = Done
media-crop-finish = Finish cropping
media-crop = Crop…
media-crop-tip = Crop the picture (or double-click it)
media-uncrop = Uncrop
media-replace = Replace…
media-replace-image-tip = Swap the picture, keeping its place and size
media-replace-video-tip = Swap the video, keeping its place, size and settings
media-mask = Mask
media-mask-square = Square
media-mask-rounded = Rounded
media-mask-circle = Circle
media-image-size = { $width } × { $height } px
media-image-size-gif = { $width } × { $height } px · animated GIF plays on the canvas
media-pause = Pause
media-play = Play
media-play-tip = Play in place (or double-click the video)
media-video-macos-only = Video plays on macOS; the poster shows here.
media-autoplay-on-step = Auto-play on its step
media-loop = Loop
media-mute = Mute
media-trim-start = Start
media-trim-end = End
media-seconds-unit = s
media-trim-clear-end-tip = Play to the end
# Small button that clears a trim end or removes the poster.
media-clear-button = ×
media-poster = Poster
media-poster-use-frame = Use This Frame
media-poster-use-frame-tip = Show this frame until the video plays
media-poster-choose = Choose…
media-poster-remove-tip = Remove the poster
media-soundtrack-heading = SOUNDTRACK
media-add-music = Add Music…
media-add-music-tip = Background music that plays while presenting
media-music = Music
media-music-name = Music: { $name }
media-remove = Remove
media-volume = Volume
media-undo-change-soundtrack = Change Background Music
media-voiceover-heading = STEP { $step } VOICE-OVER
media-add-recording = Add Recording…
media-add-recording-tip = Audio that plays when this step is reached
media-recorded = Recorded

## ---- end media_ui.rs ----


## ---- fallback.rs ----


## ---- end fallback.rs ----


## ---- mac.rs ----
# Reason given to macOS for keeping the display awake while presenting.
mac-activity-presenting = Presenting

## ---- end mac.rs ----


## ---- sample.rs ----
# File › New from Sample: the sample presentation (data/sample.json).
# Frame names (sample-frame-*) show in the path sidebar.
sample-a-new-angle = A new angle
sample-a-tiny-tour-of-a = a tiny tour of a zooming canvas
sample-brackets-circles-rectangles-invisible = brackets · circles · rectangles · invisible
sample-frame-a-new-angle = A new angle
sample-frame-a-secret = A secret
sample-frame-circles = Circles
sample-frame-why-zoom = Why zoom?
sample-frames-come-in-four-styles = Frames come in four styles
sample-hidden-in-plain-sight = hidden in plain sight!
sample-look-closer = Look closer…
sample-notes-click-to-reveal-each-point = Click to reveal each point.
sample-rotate-a-frame-and-the =
    Rotate a frame and the camera
    rolls to meet it when presenting.
sample-show-the-big-picture-first = Show the big picture first
sample-then-dive-into-the-details = Then dive into the details
sample-thinking-in-zoom = Thinking in Zoom
sample-why-zoom = Why zoom?
sample-zoom-out-again-to-connect = Zoom out again to connect them
## ---- end sample.rs ----


## ---- thumbs.rs ----


## ---- end thumbs.rs ----

## Shape names (shape library, inspector, history)
shape-rectangle = Rectangle
shape-rounded-rect = Rounded Rectangle
shape-ellipse = Oval
shape-triangle = Triangle
shape-right-triangle = Right Triangle
shape-diamond = Diamond
shape-pentagon = Pentagon
shape-hexagon = Hexagon
shape-octagon = Octagon
shape-parallelogram = Parallelogram
shape-trapezoid = Trapezoid
shape-star = Star
shape-sparkle = Four-Point Star
shape-burst = Burst
shape-heart = Heart
shape-plus = Plus
shape-ring = Ring
shape-moon = Moon
shape-lightning = Lightning
shape-cloud = Cloud
shape-speech-bubble = Speech Bubble
shape-callout = Callout
shape-thought-bubble = Thought Bubble
shape-sticky-note = Sticky Note
shape-document = Document
shape-cylinder = Cylinder
shape-cube = Cube
shape-pill = Pill
shape-delay = Half Round
shape-chevron = Chevron
shape-tag = Tag
shape-block-arrow-right = Block Arrow Right
shape-block-arrow-left = Block Arrow Left
shape-block-arrow-up = Block Arrow Up
shape-block-arrow-down = Block Arrow Down
shape-block-arrow-both = Block Arrow Both Ways
shape-line = Line
shape-arrow = Arrow
shape-double-arrow = Double Arrow

## Object types, where an object has no name or text of its own
element-frame = Frame
element-text = Text
element-shape = Shape
element-image = Image
element-video = Video
element-ink = Drawing
element-icon = Icon
element-connector = Connector

## History: the Edit menu's Undo/Redo, the Activity panel, the status bar
history-add-element = Add { $kind }
history-add-text = Add Text “{ $text }”
history-add-frame = Add Frame
history-add-shape = Add { $shape }
history-insert-image = Insert Image
history-add-icon = Add Icon: { $name }
history-add-connector = Add Connector
history-insert-video = Insert Video
history-replace-image = Replace Image
history-replace-video = Replace Video
history-set-poster = Set Poster Frame
history-add-soundtrack = Add Background Music
history-change-soundtrack = Change Background Music
history-remove-soundtrack = Remove Background Music
history-remove-voiceover = Remove Voice-Over
history-add-voiceover = Add Voice-Over
history-crop = Crop
history-highlight = Highlight
history-draw = Draw
history-edit-text = Edit Text
history-format-text = Format Text
history-format-paragraphs = Format Paragraphs
history-change-properties = Change Properties
history-move = Move
history-resize = Resize
history-rotate = Rotate
history-delete = Delete { $count ->
        [one] 1 Object
       *[other] { $count } Objects
    }
history-duplicate = Duplicate
history-change-order = Change Order
history-frame-selection = Frame Selection
history-arrange = Arrange
history-align = Align
history-group = Group
history-ungroup = Ungroup
history-add-step = Add Step
history-remove-step = Remove Step
history-reorder-path = Reorder Steps
history-edit-step = Edit Step
history-clear-path = Clear Path
history-path-from-frames = Create Path from Frames
history-theme = Apply Theme: { $theme }
history-rename = Rename
history-change-autoplay = Change Autoplay
history-edit-theme = Edit Theme
history-delete-theme = Delete Theme
history-add-picture = Add Image
history-template = Apply Template: { $template }
history-insert-layout = Insert Layout: { $layout }
history-insert-symbol = Insert Symbol: { $name }
history-insert-diagram = Insert Diagram: { $diagram }
history-changes = { $count ->
    [one] 1 Change
   *[other] { $count } Changes
}

## Built-in themes, layouts and templates (names and descriptions)
theme-name-classic = Classic
theme-name-chalkboard = Chalkboard
theme-name-blueprint = Blueprint
theme-name-paper = Paper
theme-name-night = Night
layout-name-title = Title
layout-description-title = A frame with a title and a subtitle.
layout-name-title-bullets = Title and bullets
layout-description-title-bullets = A heading over a bulleted list.
layout-name-two-columns = Two columns
layout-description-two-columns = A heading over two framed columns.
layout-name-three-circles = Three circles
layout-description-three-circles = Three circle frames in a row.
layout-name-grid = Grid of four
layout-description-grid = Four frames in a two-by-two grid.
layout-name-zoom-detail = Zoom into detail
layout-description-zoom-detail = A big frame with three small frames to zoom into.
layout-name-timeline = Timeline
layout-description-timeline = Five stops along an arrow.
layout-name-hub = Hub and spokes
layout-description-hub = A central idea with six around it.
layout-name-comparison = Comparison
layout-description-comparison = Two frames side by side.
layout-name-picture = Picture and caption
layout-description-picture = Room for a picture next to a heading and text.
template-name-blank = Blank
template-description-blank = An empty canvas.
template-name-big-picture = Big picture
template-description-big-picture = A central idea and four topics around it, with an overview at the start and end.
template-name-timeline = Timeline
template-description-timeline = Six moments along an arrow, from left to right.
template-name-journey = Journey
template-description-journey = A winding road of stops, each a little deeper than the last.
template-name-mind-map = Mind map
template-description-mind-map = A central topic with six branches.
template-name-pitch = Pitch
template-description-pitch = Problem, solution, market, model, team and the ask.
template-name-lesson = Lesson
template-description-lesson = Objectives, three chapters with details to zoom into, and a recap.
template-name-story = Story
template-description-story = Beginning, middle and end, each zooming deeper into the last.

## Spelling menu (right-click a word with a red dotted line)
spell-no-guesses = No Guesses Found
spell-learn = Learn Spelling
spell-ignore = Ignore Spelling

## Help › Acknowledgements
action-acknowledgements = Acknowledgements
credits-title = Acknowledgements
credits-clip-art = Symbols
credits-icons = Icons
credits-fonts = Fonts
credits-by = { $author } · { $licence }
credits-licence-text = Licence text

## Symbols tab (Insert › Symbols & Shapes)
symbols-not-installed = The symbol library isn't installed.
symbols-all-sets = All sets
symbols-group-emoji = Emoji
symbols-group-colour = Colour icons
symbols-group-drawn = Hand-drawn
symbols-group-people = People
symbols-group-mono = One colour
symbols-group-classic = Classic clip art
symbols-pick-a-set = Choose a set or search { $count } symbols.
symbols-none-match = No symbols match.
symbols-count = { $count ->
    [one] { $count } symbol
   *[other] { $count } symbols
}
symbols-count-many = First { $count } symbols
symbols-sticker = Sticker
symbols-sticker-tip = White outline and a soft shadow
symbols-outline-colour = Outline Colour
symbols-fill-colour = Fill Colour
symbols-swatch-dark = Dark
symbols-swatch-white = White

## Insert › Diagram
diagrams-title = Diagrams
diagrams-caption = Click a diagram to add it to the middle of the view.
diagram-name-process = Process
diagram-description-process = Steps in a row joined by arrows.
diagram-name-cycle = Circular process
diagram-description-cycle = Steps round a loop, the last feeding back to the first.
diagram-name-timeline = Timeline
diagram-description-timeline = Dated points along one line, labels above and below.
diagram-name-perspective = Perspective
diagram-description-perspective = Frames shrinking toward a vanishing point. Its frames join the path.
diagram-name-funnel = Funnel
diagram-description-funnel = Many inputs narrowing to one outcome.
diagram-name-iceberg = Iceberg
diagram-description-iceberg = A small tip above the waterline, the larger part below.
diagram-name-org-chart = Org chart
diagram-description-org-chart = A top-down tree.
diagram-name-block-list = Block list
diagram-description-block-list = Side-by-side blocks, each with a heading.
diagram-name-numbered-list = Numbered list
diagram-description-numbered-list = Rows with a numbered marker.
diagram-name-chevrons = Chevron process
diagram-description-chevrons = Interlocking arrows.
diagram-name-step-up = Step-up
diagram-description-step-up = Rising steps toward a goal.
diagram-name-segmented-ring = Segmented ring
diagram-description-segmented-ring = A ring in four parts around a centre idea.
diagram-name-tree = Tree
diagram-description-tree = Branches out to the right, for breakdowns and decisions.
diagram-name-venn2 = Venn, two circles
diagram-description-venn2 = Two overlapping sets.
diagram-name-venn3 = Venn, three circles
diagram-description-venn3 = Three overlapping sets.
diagram-name-target = Target
diagram-description-target = Rings from the outer context to the inner focus.
diagram-name-inverted-pyramid = Inverted pyramid
diagram-description-inverted-pyramid = Wide at the top, narrowing to one point.
diagram-name-mind-map = Mind map
diagram-description-mind-map = Branches from a central topic.
diagram-name-fishbone = Fishbone
diagram-description-fishbone = Causes branching into one effect.
diagram-name-columns = Columns
diagram-description-columns = Cards sorted into to do, doing and done.
diagram-name-zoom-nest = Zoom-in nest
diagram-description-zoom-nest = Frames inside frames. Its frames join the path.
diagram-name-orbit = Orbit
diagram-description-orbit = Frames round a centre. Its frames join the path.
diagram-name-big-picture = Big picture
diagram-description-big-picture = An overview frame with detail frames inside. Its frames join the path.

## Text that diagrams, layouts and templates put on the canvas: placeholders
## and frame names (content-frame-*). Filled in once, when the diagram,
## layout or template is placed; after that the text is the person's own.
## zebra-core reads the English from this file too (zebra_core::content).
content-a-detail-to-zoom-into = A detail to zoom into
content-a-line-or-two =
    A line or two
    about this topic
content-add-a-note = Add a note
content-add-a-subtitle = Add a subtitle
content-add-a-thought = Add a thought
content-add-a-title = Add a title
content-add-text = Add text
content-add-your-points =
    Add your points here
    One idea per line
    Keep it short
content-add-your-points-block =
    Add your points here
    One idea per line
content-after = After
content-all = All
content-almost-there = Almost there
content-and-in-the-end = And in the end
content-before = Before
content-both = Both
content-branch = Branch
content-business-model = Business model
content-cause = Cause
content-chapter-1 = Chapter 1
content-chapter-2 = Chapter 2
content-chapter-3 = Chapter 3
content-course-or-class = Course or class
content-date = Date
content-detail-to-zoom-into = Detail to zoom into
content-doing = Doing
content-done = Done
content-drop-a-picture-here = Drop a picture here
content-effect = Effect
content-first-stop = First stop
content-focus = Focus
content-heading = Heading
content-how-it-turned-out = How it turned out
content-idea = Idea
content-market = Market
content-name = Name
content-once-upon-a-time = Once upon a time…
content-one-line-on-what-you-do = One line on what you do
content-outcome = Outcome
content-point = Point
content-problem = Problem
content-recap = Recap
content-say-what-the-picture-shows = Say what the picture shows
content-second-stop = Second stop
content-solution = Solution
content-step = Step
content-team = Team
content-the-ask = The ask
content-the-three-things-to-remember = The three things to remember
content-then-something-changed = Then something changed
content-third-stop = Third stop
content-to-do = To do
content-topic = Topic
content-topic-1 = Topic 1
content-topic-2 = Topic 2
content-topic-3 = Topic 3
content-topic-4 = Topic 4
content-what-happened = What happened
content-what-happened-next = What happened next
content-what-lies-beneath = What lies beneath
content-what-people-see = What people see
content-what-youll-learn = What you'll learn
content-where-we-arrive = Where we arrive
content-where-we-start = Where we start
content-year = Year

content-frame-after = After
content-frame-before = Before
content-frame-beginning = Beginning
content-frame-business-model = Business model
content-frame-central-topic = Central topic
content-frame-centre = Centre
content-frame-chapter-1 = Chapter 1
content-frame-chapter-2 = Chapter 2
content-frame-chapter-3 = Chapter 3
content-frame-destination = Destination
content-frame-detail = Detail
content-frame-end = End
content-frame-left = Left
content-frame-main-idea = Main idea
content-frame-market = Market
content-frame-middle = Middle
content-frame-objectives = Objectives
content-frame-overview = Overview
content-frame-problem = Problem
content-frame-recap = Recap
content-frame-right = Right
content-frame-solution = Solution
content-frame-start = Start
content-frame-team = Team
content-frame-the-ask = The ask
content-frame-the-road-ahead = The road ahead
content-frame-title = Title
content-frame-topic-1 = Topic 1
content-frame-topic-2 = Topic 2
content-frame-topic-3 = Topic 3
content-frame-topic-4 = Topic 4
