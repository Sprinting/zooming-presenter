#!/bin/bash
# Build Zebra.app with zebractl inside it (Contents/MacOS/zebractl), so an
# agent can always run it by full path and the app can offer
# "Install command-line tool".
#
#   scripts/bundle-macos.sh                 # signed "Zebra Dev" (test builds)
#   ADHOC=1 scripts/bundle-macos.sh         # ad-hoc (no certificate)
#   SIGN_ID="Developer ID Application: …" scripts/bundle-macos.sh
#   APPSTORE=1 SIGN_ID="Apple Distribution: …" scripts/bundle-macos.sh
#   DSYM=1 scripts/bundle-macos.sh          # also target/Zebra.app.dSYM
#
# APPSTORE=1 signs both binaries with the sandbox + App Group entitlements.
# Needs a Mac (lipo, codesign). Output: target/Zebra.app
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
# Universal (Apple Silicon + Intel) when both standard libraries are
# installed (`rustup target add x86_64-apple-darwin`), else this Mac's own.
if [[ -z "${TARGETS:-}" ]]; then
  HOST=$(rustc -vV | sed -n 's/^host: //p')
  SYSROOT=$(rustc --print sysroot)
  if [[ -d "$SYSROOT/lib/rustlib/x86_64-apple-darwin" && -d "$SYSROOT/lib/rustlib/aarch64-apple-darwin" ]]; then
    TARGETS="aarch64-apple-darwin x86_64-apple-darwin"
  else
    TARGETS="$HOST"
    echo "Building for $HOST only (add x86_64-apple-darwin with rustup for a universal app)."
  fi
fi
# The release profile strips symbols. DSYM=1 builds with line tables
# instead; rustc writes a dSYM per architecture at link time (it can see
# the LTO object then), which is kept in target/Zebra.app.dSYM for crash
# reports, and the copies that go into the app are stripped.
if [ "${DSYM:-0}" = 1 ]; then
  export CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed
fi
# File > Export > Web Page embeds the web player (crates/zebra-web, built
# for wasm32 by its build.rs). A shipped app must have it.
if ! rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then
  echo "The web player needs the wasm32 target: rustup target add wasm32-unknown-unknown" >&2
  exit 1
fi
export ZEBRA_REQUIRE_WEB_PLAYER=1
for t in $TARGETS; do cargo build --release --target "$t" -p zebra-app -p zebra-ctl; done

APP=target/Zebra.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
for bin in zebra zebractl; do
  lipo -create $(for t in $TARGETS; do echo "target/$t/release/$bin"; done) -output "$APP/Contents/MacOS/$bin"
  if [ "${DSYM:-0}" = 1 ]; then
    for t in $TARGETS; do
      rm -rf "target/Zebra.app.dSYM/$t/$bin.dSYM"
      mkdir -p "target/Zebra.app.dSYM/$t"
      cp -R "target/$t/release/$bin.dSYM" "target/Zebra.app.dSYM/$t/"
    done
    strip -x "$APP/Contents/MacOS/$bin"
  fi
done
sed "s/__VERSION__/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
[ -f packaging/macos/Zebra.icns ] && cp packaging/macos/Zebra.icns "$APP/Contents/Resources/"
cp assets/icons/LICENSE-lucide.txt "$APP/Contents/Resources/"
# The clip-art library (Insert > Symbols & Shapes). Not in git: unzip
# zebra-symbols-v2.zip into assets/symbols, or rebuild it with
# tools/symbols/build_pack.py. Without it the Symbols tab says so.
if [ -f assets/symbols/sets.json ]; then
  # The app reads pack format 2 (compressed, one .dict per set) only.
  if ! grep -q '"format": *2' assets/symbols/sets.json; then
    echo "error: assets/symbols is an old pack; unzip zebra-symbols-v2.zip there or run" >&2
    echo "  python3 -P tools/symbols/build_pack.py --from-pack <old pack> --out assets/symbols" >&2
    exit 1
  fi
  mkdir -p "$APP/Contents/Resources/Symbols"
  cp -L assets/symbols/sets.json assets/symbols/*.idx assets/symbols/*.dat "$APP/Contents/Resources/Symbols/"
  # A set too small to train a dictionary has none.
  for f in assets/symbols/*.dict; do [ -e "$f" ] && cp -L "$f" "$APP/Contents/Resources/Symbols/"; done
else
  echo "warning: assets/symbols is missing; the app will have no symbols"
fi

# PDFium for File > Import (PDF): bblanchon/pdfium-binaries, pinned and
# checksummed by scripts/fetch-pdfium.sh. Zebra binds it by full path from
# Contents/Frameworks, never from the system's search paths. Its licence
# and those of the libraries built into it go in Resources/Licenses/PDFium
# (Help > Acknowledgements lists them). Checked 19 Nov 2026 (build 8125):
# needs macOS 13.0 (our LSMinimumSystemVersion), links only AppKit,
# CoreGraphics, CoreFoundation, Foundation and libSystem, and uses only
# their public symbols; arm64 7.4 MB, x86_64 7.8 MB, universal 15.2 MB.
case " $TARGETS " in
  *aarch64-apple-darwin*x86_64-apple-darwin*|*x86_64-apple-darwin*aarch64-apple-darwin*) PDFIUM=mac-univ ;;
  *aarch64-apple-darwin*) PDFIUM=mac-arm64 ;;
  *) PDFIUM=mac-x64 ;;
esac
PDFIUM_LIB=$(scripts/fetch-pdfium.sh "$PDFIUM")
PDFIUM_DIR=$(dirname "$(dirname "$PDFIUM_LIB")")
mkdir -p "$APP/Contents/Frameworks" "$APP/Contents/Resources/Licenses/PDFium"
cp "$PDFIUM_LIB" "$APP/Contents/Frameworks/libpdfium.dylib"
install_name_tool -id @rpath/libpdfium.dylib "$APP/Contents/Frameworks/libpdfium.dylib"
cp "$PDFIUM_DIR/LICENSE" "$APP/Contents/Resources/Licenses/PDFium/pdfium-binaries.txt"
cp "$PDFIUM_DIR"/licenses/* "$APP/Contents/Resources/Licenses/PDFium/"

# Test builds are always signed with the "Zebra Dev" certificate in the
# login keychain, so macOS permissions survive rebuilds (Kartik, 19 Nov).
# Ad-hoc signing only when asked for: ADHOC=1.
if [ "${ADHOC:-0}" = 1 ]; then
  ID=-
else
  ID=${SIGN_ID:-Zebra Dev}
  if ! security find-identity -p codesigning | grep -qF "\"$ID\""; then
    echo "No code-signing identity \"$ID\" in the keychain. Set SIGN_ID, or ADHOC=1 for an ad-hoc build." >&2
    exit 1
  fi
fi
# Nested code is signed first, with the same identity (library validation).
codesign --force --options runtime ${SIGN_ID:+--timestamp} --sign "$ID" "$APP/Contents/Frameworks/libpdfium.dylib"
if [ "${APPSTORE:-0}" = 1 ]; then
  codesign --force --options runtime --timestamp --sign "$ID" \
    --entitlements packaging/macos/zebractl.entitlements "$APP/Contents/MacOS/zebractl"
  codesign --force --options runtime --timestamp --sign "$ID" \
    --entitlements packaging/macos/Zebra.entitlements "$APP"
else
  codesign --force --options runtime --sign "$ID" "$APP/Contents/MacOS/zebractl"
  case "$ID" in
    "Developer ID Application:"*) codesign --force --options runtime --timestamp --sign "$ID" "$APP" ;;
    # Ad-hoc or self-signed: no team, so let the app load its own PDFium.
    *) codesign --force --options runtime --sign "$ID" --entitlements packaging/macos/Local.entitlements "$APP" ;;
  esac
fi
codesign --verify --deep --strict "$APP"
echo "Built $APP ($VERSION)"
