# PDFium: which build Zebra ships, and when it moves

Zebra reads PDFs with PDFium, Chrome's PDF engine, in a helper process of
its own (`zebra-pdf-helper`) that is killed when it passes its memory
budget or its deadline. A PDF someone sends is hostile input, so the
PDFium inside the app has to keep up with the security fixes Chrome makes.

## What is pinned

- The library: a prebuilt build from bblanchon/pdfium-binaries,
  `chromium/<build>`, fetched by `scripts/fetch-pdfium.sh`, which checks
  the SHA-256 of each platform's archive.
- The binding: the `pdfium-render` crate's `pdfium_<build>` feature in
  `crates/zebra-pdf/Cargo.toml`. The two must name the same build; the
  test `zebra-pdf/tests/pinned.rs` fails when they don't.

Now: build **8125** (checked 21 Nov 2026: pdfium-render 0.9.5 is the
newest release, and 8125 is the newest build it binds to).

## When to move

- **At once** when a Chrome stable release notes a security fix in
  PDFium (the Chrome Releases blog lists them as "in PDFium"), as soon
  as pdfium-binaries has a build that includes it.
- **Otherwise, once a month**: check pdfium-binaries' latest release and
  pdfium-render's newest `pdfium_<build>` feature. Move to the newest
  build both support.
- **Before every release** of Zebra, the same check.

If pdfium-render doesn't bind to a build with a needed security fix yet,
use its newest feature with the newer library only if its own changelog
says the API is unchanged; otherwise wait for it, and note the gap in
`/mnt/project-files/zebra/log.md`.

## How to move

1. Pick the build. In `scripts/fetch-pdfium.sh` set `VERSION` and the four
   SHA-256s (`mac-univ`, `mac-arm64`, `mac-x64`, `linux-x64`); take them
   from the archives themselves (`shasum -a 256`), never from a web page.
2. In `crates/zebra-pdf/Cargo.toml` set the `pdfium_<build>` feature (and
   pdfium-render's version if that is needed).
3. Run the PDF tests in the cloud with the new Linux library
   (`ZEBRA_PDFIUM=<path> cargo test -p zebra-pdf -p zebra-import`), then
   the full suite. `hostile_pdf` must pass unchanged.
4. On the Mac: check what the library links against and its minimum
   macOS (`otool -L`, `vtool -show`), as `scripts/bundle-macos.sh` notes,
   and update the note there (build, date, sizes).
5. Check the licences in the archive's `licenses/` folder against Help ›
   Acknowledgements; add any new one.
6. Note the move in `log.md`, with the build numbers.
