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
