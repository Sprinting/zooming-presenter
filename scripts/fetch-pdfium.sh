#!/bin/bash
# Fetch the prebuilt PDFium that File > Import (PDF) loads at run time.
#
#   scripts/fetch-pdfium.sh mac-univ|mac-arm64|mac-x64|linux-x64 [DIR]
#
# Unpacks bblanchon/pdfium-binaries (MIT; PDFium itself is BSD-3, with
# its third-party licences in licenses/) into DIR (default
# target/pdfium/<platform>) and prints the library's path. The archive's
# SHA-256 is checked. Pinned to the PDFium build pdfium-render's
# `pdfium_8125` feature binds to; change both together.
#
# The library must ship inside the app (scripts/bundle-macos.sh copies
# it to Contents/Frameworks); Zebra binds it by full path from there and
# never from the system's search paths. Debug builds and tests also take
# ZEBRA_PDFIUM=<path to the library>.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=8125
PLATFORM=${1:?platform: mac-univ, mac-arm64, mac-x64 or linux-x64}
DIR=${2:-target/pdfium/$PLATFORM}
case "$PLATFORM" in
  mac-univ)  SUM=0cf06803b9a4acc489fc967a1b05990622ca1cf41fa027ff9cc89eeab9b19279; LIB=libpdfium.dylib ;;
  mac-arm64) SUM=4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d; LIB=libpdfium.dylib ;;
  mac-x64)   SUM=8a5193535b80c7215d8932f6647ae5097465b6dba2811602c21e120c2af112b0; LIB=libpdfium.dylib ;;
  linux-x64) SUM=e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659; LIB=libpdfium.so ;;
  *) echo "unknown platform $PLATFORM" >&2; exit 1 ;;
esac
if [ -f "$DIR/lib/$LIB" ] && [ -f "$DIR/.sha256" ] && [ "$(cat "$DIR/.sha256")" = "$SUM" ]; then
  echo "$DIR/lib/$LIB"; exit 0
fi
mkdir -p "$DIR"
TGZ="$DIR/pdfium-$PLATFORM.tgz"
curl -fsSL -o "$TGZ" "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F$VERSION/pdfium-$PLATFORM.tgz"
GOT=$( (command -v sha256sum >/dev/null && sha256sum "$TGZ" || shasum -a 256 "$TGZ") | cut -d' ' -f1)
if [ "$GOT" != "$SUM" ]; then
  echo "error: pdfium-$PLATFORM.tgz has SHA-256 $GOT, expected $SUM" >&2
  rm -f "$TGZ"; exit 1
fi
tar xzf "$TGZ" -C "$DIR"
rm -f "$TGZ"
echo "$SUM" > "$DIR/.sha256"
echo "$DIR/lib/$LIB"
