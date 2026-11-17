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
