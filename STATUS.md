# Zebra: build status (17 Nov 2026)

Zebra is a placeholder name. Built in a Linux cloud container; the target is macOS only.

## Works and verified here (Linux, software GPU)
- Full workspace builds; every test passes (`cargo test --workspace`): core model/undo/present logic, renderer pixel checks, API end-to-end over the real Unix socket, `zebractl` + MCP over stdio, and egui_kittest UI tests (double-click to write, drag = one undo step, present keys, tools, path edit).
- The GUI ran under Xvfb with software Vulkan (lavapipe); screenshots in `out/app/`.
- Agent driving the live window with `zebractl` while the window stayed usable (violet agent cursor, dashed agent selection, status pill).
- Headless: `zebra --headless`, built a 3-step deck with `zebractl apply`, captured steps, exported PDF (`out/integration/`).
- vello GPU path rendered headless on lavapipe and matched the CPU renderer (mean diff < 0.05/channel).
- `cargo check --target aarch64-apple-darwin` passes for the app (muda menu code compiles for macOS).

## Not verified (needs a Mac)
- Actually running on macOS: Metal rendering, Retina, the native menu bar (and whether menu shortcuts fire twice), trackpad two-finger pan vs pinch, native full screen in present mode, IME, rfd dialogs.
- Socket path lengths / signal handling on macOS, finding `zebra` inside a `.app` bundle.
- Performance with thousands of elements.

## Known gaps
- No `.app` bundle / codesigning / notarization yet (`cargo run --release -p zebra-app` for now).
- Not done: minimap, group/ungroup, speaker-notes editor, presenter view, App Nap prevention, system Reduce Motion (it is a View menu toggle), 3D parallax backgrounds, templates gallery, PDF/video import.
- Known bugs: the text edit box is upright on rotated text; restoring an autosave doesn't mark the doc edited; resizing pans instead of re-centring.
- Capture and PDF export run on the UI thread (a big capture briefly stalls the window).
