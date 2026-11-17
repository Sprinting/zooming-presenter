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


## ---- end app.rs ----


## ---- panels.rs ----

status-no-agent = No agent
status-agent-idle = { $name }: idle
status-agent-paused = { $name }: paused
status-agent-action = { $name }: { $action }
status-activity = Activity

## ---- end panels.rs ----


## ---- sidebar.rs ----


## ---- end sidebar.rs ----


## ---- settings.rs ----


## ---- end settings.rs ----


## ---- welcome.rs ----


## ---- end welcome.rs ----


## ---- sheets.rs ----


## ---- end sheets.rs ----


## ---- files.rs ----


## ---- end files.rs ----


## ---- cli.rs ----


## ---- end cli.rs ----


## ---- inspector.rs ----


## ---- end inspector.rs ----


## ---- inspector_s3.rs ----


## ---- end inspector_s3.rs ----


## ---- shape_library.rs ----


## ---- end shape_library.rs ----


## ---- theme_editor.rs ----


## ---- end theme_editor.rs ----


## ---- widgets.rs ----


## ---- end widgets.rs ----


## ---- library.rs ----


## ---- end library.rs ----


## ---- style.rs ----


## ---- end style.rs ----


## ---- icons.rs ----


## ---- end icons.rs ----


## ---- actions.rs ----
action-new = New
action-new-from-sample = New from Sample
action-show-templates = New from Template…
action-save-as-template = Save as Template…
action-open = Open…
action-save = Save
action-save-as = Save As…
action-export-pdf = Export PDF…
action-export-png = Export View as PNG…
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
action-deselect = Deselect
action-frame-tool = Frame Tool (cycle style)
action-insert-image = Image…
action-insert-layout = Layout…
action-insert-music = Background Music…
action-replace-image = Replace Image…
action-crop-image = Crop Image
action-play-pause-video = Play / Pause Video
action-wrap-in-frame = Frame Selection
action-shape-library = Shapes and Icons…
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
action-fit-all = Zoom to Fit All
action-zoom-to-selection = Zoom to Selection
action-zoom-reset = Actual Size (100%)
action-toggle-sidebar = Show Sidebar
action-toggle-properties = Show Inspector
action-toggle-reduce-motion = Reduce Motion
action-command-palette = Command Palette…
action-show-shortcuts = Keyboard Shortcuts
action-settings = Settings…
action-toggle-path-edit = Edit Path
action-add-to-path = Add Selection to Path
action-add-current-view = Add Current View to Path
action-auto-path = Path from Frames
action-clear-path = Clear Path…
action-present-start = Play Presentation
action-present-current = Play from Current Step
action-toggle-presenter-view = Presenter View
action-playback-options = Playback Options…
action-toggle-ai-control = Let AI Agents Control Zebra
action-toggle-activity = Agent Activity
action-pause-agents = Pause / Resume Agent
action-stop-agents = Stop Agent
action-follow-agent = Follow Agent
action-revert-agent = Revert Agent's Changes…
action-theme-editor = Customize Theme…
action-about = About Zebra
action-new-from-template = New from Template: { $name }
action-set-tool = { $tool } Tool
action-set-frame-style = { $style } Frame
action-agent-permission-observe = Agents May Look Only
action-agent-permission-edit = Agents May Edit
action-agent-permission-full = Agents Have Full Control…
action-set-theme = Theme: { $name }
tool-select = Select
tool-hand = Hand
tool-text = Text
tool-frame = Frame
tool-rectangle = Rectangle
tool-ellipse = Ellipse
tool-triangle = Triangle
tool-line = Line
tool-arrow = Arrow
tool-pen = Pen
tool-highlighter = Highlighter
frame-style-bracket = Bracket
frame-style-circle = Circle
frame-style-rectangle = Rectangle
frame-style-invisible = Invisible

## ---- end actions.rs ----


## ---- menus.rs ----
menu-file = File
menu-export-to = Export To
menu-edit = Edit
menu-insert = Insert
menu-frame = Frame
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

## ---- end menu_mac.rs ----


## ---- canvas.rs ----
canvas-chip-zoom-into-frame = Zoom into Frame
canvas-chip-rename-hint = Double-click to rename
canvas-toast-rename-failed = Couldn't rename the frame.
canvas-undo-move-connector-end = Move connector end
canvas-undo-rotate = Rotate
canvas-undo-resize = Resize
canvas-undo-move = Move
canvas-undo-add-text = Add text
canvas-undo-edit-text = Edit text
# $step is the 1-based step number.
canvas-toast-step-retargeted = Step { $step } now points here
canvas-toast-framed-objects = { $count ->
    [one] Framed { $count } object
   *[other] Framed { $count } objects
}
canvas-toast-added-step = Added step { $step }
canvas-toast-shape-locked = This shape is locked — unlock it to type on it
canvas-toast-text-locked = This text is locked — unlock it to edit
canvas-text-placeholder = Type something
canvas-text-style-title = Title
canvas-text-style-subtitle = Subtitle
canvas-text-style-body = Body
canvas-text-bold = Bold
canvas-text-italic = Italic
canvas-text-underline = Underline
canvas-text-strikethrough = Strikethrough
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
present-undo-change-step-timing = Change step timing
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
present-presenter-overview = Overview · { $count } steps
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
present-hud-overview = Overview  ·  { $count } steps
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
media-undo-change-soundtrack = Change soundtrack
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


## ---- end sample.rs ----


## ---- thumbs.rs ----


## ---- end thumbs.rs ----
