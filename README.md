# Zebra

A native zooming-canvas presentation app for macOS, in the spirit of classic
(2012–2014) Prezi. You get one infinite canvas, frames, a path, and a camera
that flies between steps. It's written in Rust (egui for the chrome, vello on
Metal for the canvas) and uses no web views.

You use it like Prezi: you click, drag and type, and you never need to know a
command. It can also be driven by AI agents, if you choose: click **AI: Off**
in the toolbar and switch on **Let AI agents control Zebra**. Then any agent
on the Mac that can run a command (Claude Code, Codex, Cursor, Gemini CLI, a
shell script) can read and edit the same presentation while you watch, using
the `zebractl` tool bundled inside the app (see [docs/API.md](docs/API.md)).
Only you can switch it on; an agent can switch it off. Everything an agent
changes is shown on the canvas and can be undone.

## Build and run (macOS)

```sh
# Rust 1.95+ (rustup.rs)
rustup target add wasm32-unknown-unknown         # for File › Export › Web Page
cargo run --release -p zebra-app                 # opens an empty presentation
cargo run --release -p zebra-app -- talk.zebra   # opens a file
```

To try it right away, choose **File › New from Sample** for a short tour
presentation, then press **F5**.

Developer options (end users never need these; AI control is a switch in the app):

| Flag | What it does |
|---|---|
| `--api` | Switch AI control on at launch, as if you had flipped the switch. |
| `--api-socket <path>` | Use this socket path instead of the default. |
| `--agent-permission observe\|edit\|full` | Set what a connected agent may do (default `edit`). You can change it later from the AI button in the toolbar. |
| `--headless` | Run with no window, API only (for batch generation). `zebractl` starts this mode for you. |

The first build also builds the web player: `crates/zebra-web/build.rs`
compiles that crate for `wasm32-unknown-unknown` (release, into
`target/web-player`, cached after that) and embeds it gzip-compressed, so
the app needs no wasm tools at run time. Without the wasm32 target the
build only warns and File › Export › Web Page reports that the player is
missing; `scripts/bundle-macos.sh` refuses to bundle without it.
`ZEBRA_WEB_PLAYER=/path/zebra_web.wasm` uses a player built elsewhere, and
`ZEBRA_WEB_PLAYER=none` skips it.

The binary is `target/release/zebra`. It builds and runs on Linux too, which
is how CI and the UI tests run, but macOS is the primary platform.

## Using it

**Editing**
- **Double-click anywhere** to write. A small toolbar lets you pick Title, Subtitle or Body, plus bold, italic and bullets. Press Esc or click outside when you're done.
- **Click** to select. Drag the object to move it, drag a corner to scale it, and drag the zebra-striped handle to rotate it (hold ⇧ to snap to 15°). Guides appear when edges or centres line up; hold ⌥ to turn snapping off.
- **Frames** (F, and press F again to cycle bracket, circle, rectangle, invisible): draw a frame around content to group it, and moving the frame moves what's inside. Double-click a frame, or click its name tag, to zoom into it.
- The **tool bar** at the bottom has Select, Hand, Text, Frame, Rectangle, Ellipse, Line, Arrow, Pen and Image. You can also drop image files onto the canvas.
- The **properties panel** on the right shows position, scale, rotation, text style, theme colours, shape fill and stroke, frame style, lock, order, "fade in on step N" and "add to path".
- **Themes**: Classic, Chalkboard, Blueprint, Paper and Night (top bar).
- **Navigating**: on a trackpad, two-finger scroll pans and pinch zooms. A mouse wheel zooms around the cursor. Space-drag or middle-drag pans. The + − ⌂ buttons on the right edge zoom in, zoom out and fit everything. You can change the scroll behaviour in the shortcuts sheet (`?`).

**Path**
- The left sidebar lists the steps as numbered thumbnails. Click a step to fly there, drag steps to reorder them, hover a step to delete it, and right-click for more options.
- **+ Add selection** / **+ Add current view** (⇧⌘A) adds a step. **Edit path** (P) shows numbered badges on the canvas: click objects to append steps, and drag a badge onto another object to retarget it.

**Presenting**
- **Present** (F5 or ⌘↵) starts from the beginning. ⇧F5 or ⌥⌘↵ starts from the current step.
- → ↓ Space PageDown Enter or a click goes to the next step or fade-in. ← ↑ PageUp Backspace goes back. Home or O shows the overview. B and W blank the screen black or white. Esc returns to the editor at the current step.
- You can scroll, drag or click any object to explore off the path. The next key brings you back to the path.
- View › Reduce Motion replaces the swooping flights with short, gentle ones.

**Files**: `.zebra` documents (a zip holding readable JSON plus image files).
Open Recent is in the File menu. While there are unsaved changes, Zebra
autosaves a sidecar file every 30 s and offers to restore it. Export PDF
produces one page per step.

**Web page**: File › Export To › Export Web Page (HTML)… writes one `.html`
file that presents the deck in any current browser (Safari, Chrome,
Firefox; desktop and phone), with nothing to install: the same drawing and
camera flights as the app, fade-ins, autoplay, soundtrack and videos, the
keys above, click or swipe to step, wheel or pinch to zoom, and a small
control bar. When its video and audio pass 50 MB it becomes a folder
(`index.html` plus `media/`) to share whole. Zebra then offers to open it
in your browser. Agents: `zebractl export html -o talk.html`.

## Keyboard shortcuts

Change any of these in Settings › Keyboard. Arrow keys nudge the selection
(⇧ for 10×). The table below is generated from the keymap's defaults; a test
fails when it drifts (`ZEBRA_BLESS=1 cargo test -p zebra-app --test readme_shortcuts`
rewrites it).

<!-- BEGIN SHORTCUTS -->
| Action | Keys |
|---|---|
| New | ⌘N |
| New from Template… | ⇧⌘N |
| Open… | ⌘O |
| Save | ⌘S |
| Save As… | ⇧⌘S |
| Export PDF… | ⌘E |
| Duplicate | ⌘D |
| Delete | ⌫ or Del |
| Deselect | Esc |
| Select Tool | V |
| Hand Tool | H |
| Text Tool | T |
| Rectangle Tool | R |
| Ellipse Tool | O |
| Line Tool | L |
| Arrow Tool | A |
| Pen Tool | D |
| Image… | I |
| Frame Selection | ⌘G |
| Bring Forward | ] |
| Send Backward | [ |
| Bring to Front | ⌘] |
| Send to Back | ⌘[ |
| Lock | ⌘L |
| Zoom In | ⌘= or ⌘+ |
| Zoom Out | ⌘− |
| Zoom to Fit All | ⇧1 or ! |
| Zoom to Selection | ⇧2 |
| Actual Size (100%) | ⌘0 |
| Show Sidebar | ⌃⌘S or ⌘\ |
| Show Inspector | ⌥⌘I |
| Keyboard Shortcuts | ? |
| Settings… | ⌘, |
| Group | ⌥⌘G |
| Ungroup | ⌥⇧⌘G |
| Edit Path | P |
| Add Selection to Path | ⇧⌘A |
| Play Presentation | ⌥⌘P or ⌘↵ or F5 |
| Play from Current Step | ⌥⇧⌘P or ⌥⌘↵ or ⇧F5 |
| Agent Activity | ⌘J |
| Stop Agent | ⌘. |
| Command Palette… | ⇧⌘P or ⌘K |
| Close | ⌘W |
| Frame Tool (cycle style) | F |
| Redo | ⇧⌘Z or ⌘Y |
| Bold | ⌘B |
| Italic | ⌘I |
| Underline | ⌘U |
| Undo | ⌘Z |
| Select All | ⌘A |
| Quit Zebra | ⌘Q |
| Cut | ⌘X |
| Copy | ⌘C |
| Paste | ⌘V |
<!-- END SHORTCUTS -->

## Working with an AI agent

Click the **AI** button in the toolbar and switch on **Let AI agents control
Zebra**. The popover gives you a sentence to paste into any agent's chat. It
points the agent at `zebractl --help`, by the tool's full path inside
`Zebra.app`, so nothing has to be installed. Optional extras in the same
popover: **Install command-line tool** (puts `zebractl` on your PATH, direct
download builds only) and **Use with MCP** (a JSON snippet for apps that take
an MCP server config). The switch is remembered between launches. Switching it
off, or an agent running `zebractl disconnect`, closes the socket at once.

A connected agent uses the same commands that the buttons and gestures use:

- You see the agent's **violet cursor** with its name, a **dashed violet
  selection** that is separate from yours, and a brief **violet flash** on
  everything it changes.
- The **AI button** in the top bar lets you pause or stop the agent (⌘. stops
  it), follow its camera, change its permission, or **revert everything it
  did**. The **activity panel** (⌘J) lists each change it made. Click an entry
  to zoom to the objects involved.
- The agent can't move your camera, change your selection, take keyboard focus
  or present unless you give it `full` permission. While you are dragging or
  typing, the agent's edits wait. Each of the agent's changes is a normal,
  labelled undo step.

See [docs/API.md](docs/API.md) for the protocol, `zebractl` and the MCP setup.

## Repository layout

| Crate | Role |
|---|---|
| `crates/zebra-core` | Document model, the command bus (the only way to change state), undo, camera flights, themes, file format |
| `crates/zebra-render` | Display list, text layout, vello GPU canvas, CPU captures, PDF export |
| `crates/zebra-api` | JSON-RPC automation server, headless mode |
| `crates/zebra-ctl` | `zebractl` CLI and MCP bridge (shipped inside Zebra.app) |
| `crates/zebra-web` | Web player (core + renderer built for wasm32, page script in `web/`) and the HTML exporter; browser check in `tests/browser/check.js` |
| `scripts/bundle-macos.sh`, `packaging/macos/` | Builds Zebra.app with zebractl inside; Info.plist and App Store entitlements |
| `crates/zebra-app` | This app (`zebra` binary) |

Tests: `cargo test --workspace`. The UI tests (`crates/zebra-app/tests/ui.rs`)
drive the real app through `egui_kittest`. They cover double-click to write,
drag-to-move as a single undo step, present-mode keys, tool shortcuts, drawing
a frame around content, and path editing.
