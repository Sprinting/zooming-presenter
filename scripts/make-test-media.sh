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
    rm -rf "$DIR"
    echo "Removed $DIR"
    ;;
  make)
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
