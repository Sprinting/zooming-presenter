#!/bin/bash
# Build Zebra.app with zebractl inside it (Contents/MacOS/zebractl), so an
# agent can always run it by full path and the app can offer
# "Install command-line tool".
#
#   scripts/bundle-macos.sh                 # unsigned/ad-hoc, direct download
#   SIGN_ID="Developer ID Application: …" scripts/bundle-macos.sh
#   APPSTORE=1 SIGN_ID="Apple Distribution: …" scripts/bundle-macos.sh
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
for t in $TARGETS; do cargo build --release --target "$t" -p zebra-app -p zebra-ctl; done

APP=target/Zebra.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
for bin in zebra zebractl; do
  lipo -create $(for t in $TARGETS; do echo "target/$t/release/$bin"; done) -output "$APP/Contents/MacOS/$bin"
done
sed "s/__VERSION__/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
[ -f packaging/macos/Zebra.icns ] && cp packaging/macos/Zebra.icns "$APP/Contents/Resources/"
cp assets/icons/LICENSE-lucide.txt "$APP/Contents/Resources/"
# The clip-art library (Insert > Symbols & Shapes). Not in git: unzip
# zebra-symbols.zip into assets/symbols, or rebuild it with
# tools/symbols/build_pack.py. Without it the Symbols tab says so.
if [ -f assets/symbols/sets.json ]; then
  mkdir -p "$APP/Contents/Resources/Symbols"
  cp -L assets/symbols/sets.json assets/symbols/*.idx assets/symbols/*.dat "$APP/Contents/Resources/Symbols/"
else
  echo "warning: assets/symbols is missing; the app will have no symbols"
fi

ID=${SIGN_ID:--}
if [ "${APPSTORE:-0}" = 1 ]; then
  codesign --force --options runtime --timestamp --sign "$ID" \
    --entitlements packaging/macos/zebractl.entitlements "$APP/Contents/MacOS/zebractl"
  codesign --force --options runtime --timestamp --sign "$ID" \
    --entitlements packaging/macos/Zebra.entitlements "$APP"
else
  codesign --force --options runtime --sign "$ID" "$APP/Contents/MacOS/zebractl"
  codesign --force --options runtime --sign "$ID" "$APP"
fi
codesign --verify --deep --strict "$APP"
echo "Built $APP ($VERSION)"
