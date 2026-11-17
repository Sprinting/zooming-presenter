# Zebra: build status (17 Nov 2026)

Zebra is a placeholder name. Built in a Linux cloud container; the target is macOS only.

## Works and verified here (Linux, software GPU)
- Full workspace builds; every test passes (`cargo test --workspace`): core model/undo/present logic, renderer pixel checks, API end-to-end over the real Unix socket, `zebractl` + MCP over stdio, and egui_kittest UI tests (double-click to write, drag = one undo step, present keys, tools, path edit).
- The GUI ran under Xvfb with software Vulkan (lavapipe); screenshots in `out/app/`.
- Agent driving the live window with `zebractl` while the window stayed usable (violet agent cursor, dashed agent selection, status pill).
- Headless: `zebra --headless`, built a 3-step deck with `zebractl apply`, captured steps, exported PDF (`out/integration/`).
- vello GPU path rendered headless on lavapipe and matched the CPU renderer (mean diff < 0.05/channel).
- AI control is a switch in the app (AI button in the toolbar), off by default and remembered. Only the person can switch it on; an agent can switch it off with `zebractl disconnect`. Tested by a UI test that clicks the switch, connects an agent, and checks the socket and discovery file disappear after `disconnect`. `--api` is now a developer flag only.
- `cargo check --target aarch64-apple-darwin` passes for the app (muda menu code compiles for macOS).

## Not verified (needs a Mac)
- Actually running on macOS: Metal rendering, Retina, the native menu bar (and whether menu shortcuts fire twice), trackpad two-finger pan vs pinch, native full screen in present mode, IME, rfd dialogs.
- Socket path lengths / signal handling on macOS, finding `zebra` inside a `.app` bundle.
- Performance with thousands of elements.
- AI control in a real Zebra.app: `scripts/bundle-macos.sh` (puts zebractl in Contents/MacOS), the agent snippet's full path, "Install command-line tool" (admin prompt via osascript, symlink in /usr/local/bin), and the switch item in the native Agent menu.
- App Store sandbox: both binaries need the App Group entitlement and a real Team ID (placeholders now). Unverified: that the group container path works for the socket (and fits 104 bytes), that a sandboxed zebractl launched by a terminal agent can reach it, and that `screenshot -o`/`export` can write where an agent asks. If sandboxed zebractl can't write outside its container, captures should be returned inline instead.

## Known gaps
- `.app` bundling script exists but is untested; no notarization, icon, or real bundle/team ids yet.
- AI setup doesn't auto-detect installed agents or write their config files (a sandboxed app can't write other apps' settings); it offers a copyable prompt and an MCP snippet instead. The API-key option is skipped for now, as agreed.
- Not done: minimap, group/ungroup, speaker-notes editor, presenter view, App Nap prevention, system Reduce Motion (it is a View menu toggle), 3D parallax backgrounds, templates gallery, PDF/video import.
- Known bugs: the text edit box is upright on rotated text; restoring an autosave doesn't mark the doc edited.
- Capture and PDF export run on the UI thread (a big capture briefly stalls the window).
