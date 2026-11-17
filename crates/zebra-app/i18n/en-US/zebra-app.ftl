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

app-untitled = Untitled
app-agent-default-name = Agent
app-window-title-edited = { $name } — Edited
app-file-filter-zebra = Zebra presentation
app-file-filter-image = Image
app-ai-start-failed = Couldn't start AI control: { $error }
app-toast-error-with-hint = { $message } — { $hint }
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
app-undo-add-to-path = Add to path
app-undo-add-current-view = Add current view
app-undo-nudge = Nudge

## ---- end app.rs ----


## ---- panels.rs ----

status-no-agent = No agent
status-agent-idle = { $name }: idle
status-agent-paused = { $name }: paused
status-agent-action = { $name }: { $action }
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
toolbar-shape-library = Shapes and Icons…
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
activity-seconds-ago = { $seconds } s ago
activity-minutes-ago = { $minutes } min ago
activity-item-meta = { $origin } · { $when } · { $count } object(s)
activity-item-tip = Click to show these objects
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
settings-default-theme = Theme for new presentations:
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

cli-agent-prompt = Zebra (a zooming presentation app) is open on my Mac and I've allowed AI control. Run `{ $ctl } --help` to learn how to drive it, then `{ $ctl } describe` to see what's on the canvas.
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
inspector-undo-format-paragraphs = Format paragraphs
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
library-window-title = Shapes & Icons
library-window-replace-icon = Replace Icon
library-tab-shapes = Shapes
library-tab-icons = Icons
library-search-shapes-hint = Search shapes
library-search-icons-hint = Search icons: idea, chart, arrow…
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
theme-editor-undo-edit = Edit theme
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

## ---- end widgets.rs ----


## ---- library.rs ----
# File name used when a theme or template has no name.
library-file-untitled = Untitled

## ---- end library.rs ----


## ---- style.rs ----


## ---- end style.rs ----


## ---- icons.rs ----


## ---- end icons.rs ----


## ---- actions.rs ----


## ---- end actions.rs ----


## ---- menus.rs ----


## ---- end menus.rs ----


## ---- menu_mac.rs ----


## ---- end menu_mac.rs ----


## ---- canvas.rs ----


## ---- end canvas.rs ----


## ---- presenting.rs ----


## ---- end presenting.rs ----


## ---- present.rs ----


## ---- end present.rs ----


## ---- media_ui.rs ----


## ---- end media_ui.rs ----


## ---- fallback.rs ----


## ---- end fallback.rs ----


## ---- mac.rs ----


## ---- end mac.rs ----


## ---- sample.rs ----


## ---- end sample.rs ----


## ---- thumbs.rs ----


## ---- end thumbs.rs ----
