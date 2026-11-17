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
TARGETS=${TARGETS:-"aarch64-apple-darwin x86_64-apple-darwin"}
for t in $TARGETS; do cargo build --release --target "$t" -p zebra-app -p zebra-ctl; done

APP=target/Zebra.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
for bin in zebra zebractl; do
  lipo -create $(for t in $TARGETS; do echo "target/$t/release/$bin"; done) -output "$APP/Contents/MacOS/$bin"
done
sed "s/__VERSION__/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
[ -f packaging/macos/Zebra.icns ] && cp packaging/macos/Zebra.icns "$APP/Contents/Resources/"

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
