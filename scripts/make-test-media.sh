#!/bin/sh
# Huge and broken files for trying Zebra by hand: see docs/TEST-MEDIA.md.
#   scripts/make-test-media.sh          make them in ~/Downloads/zebra-test-media
#   scripts/make-test-media.sh clean    remove them all
# ZEBRA_TEST_MEDIA picks another folder; ZEBRA_BIG_MB the video size (2048).
set -eu
DIR="${ZEBRA_TEST_MEDIA:-$HOME/Downloads/zebra-test-media}"
cd "$(dirname "$0")/.."
case "${1:-make}" in
  clean)
    # Only a folder this script made: it carries the marker.
    if [ ! -f "$DIR/.zebra-test-media" ]; then
      echo "Not removing $DIR: it wasn't made by this script." >&2
      exit 1
    fi
    rm -rf "$DIR"
    echo "Removed $DIR"
    ;;
  make)
    if [ -e "$DIR" ] && [ ! -f "$DIR/.zebra-test-media" ]; then
      echo "$DIR exists and wasn't made by this script; pick another folder with ZEBRA_TEST_MEDIA." >&2
      exit 1
    fi
    mkdir -p "$DIR"
    touch "$DIR/.zebra-test-media"
    cargo run --release -q -p zebra-import --example test_media -- "$DIR" "${ZEBRA_BIG_MB:-2048}"
    rm -f "$DIR/hostile/pipe.png"
    mkfifo "$DIR/hostile/pipe.png"
    du -sh "$DIR"
    ;;
  *)
    echo "usage: $0 [make|clean]" >&2
    exit 2
    ;;
esac
