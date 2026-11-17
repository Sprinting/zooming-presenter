#!/usr/bin/env bash
# Memory and speed budgets (BUILD_PLAN.md, "Memory and speed budgets").
#
#   scripts/perf.sh            # build release, run every scenario, fail on a broken budget
#
# Linux CI can't measure the Mac footprint (lavapipe alone is ~380 MB RSS),
# so here it checks what it can: the app heap from the counting allocator,
# cache sizes, idle repaints and undo history. Frame times are printed but
# only enforced on macOS, where the GPU is real. Needs xvfb-run on Linux.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p zebra-app -p zebra-ctl >/dev/null
cargo build --release -p zebra-core --examples >/dev/null
BIN=target/release
WORK=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$WORK"' EXIT
FAIL=0
MAC=0; [[ "$(uname)" == "Darwin" ]] && MAC=1

run_app() { # $1 = file or "", sets SOCK
  SOCK="$WORK/z$RANDOM.sock"
  local args=(--api --api-socket "$SOCK" --size 1440x900)
  [[ -n "$1" ]] && args+=("$1")
  if [[ $MAC == 1 ]]; then "$BIN/zebra" "${args[@]}" & else xvfb-run -a -s "-screen 0 1600x1000x24" "$BIN/zebra" "${args[@]}" & fi
  for _ in $(seq 100); do [[ -S "$SOCK" ]] && break; sleep 0.2; done
}
perf() { "$BIN/zebractl" --socket "$SOCK" perf --json; }
stop() { kill %1 2>/dev/null || true; wait 2>/dev/null || true; }

check() { # name, value, op, budget, [mac-only]
  local name=$1 v=$2 op=$3 budget=$4 maconly=${5:-}
  if [[ "$v" == "null" || -z "$v" ]]; then echo "  ?    $name: not measured"; return; fi
  if python3 -c "import sys; sys.exit(0 if float('$v') $op float('$budget') else 1)"; then
    echo "  ok   $name: $v (budget $op $budget)"
  elif [[ -n "$maconly" && $MAC == 0 ]]; then
    echo "  info $name: $v (budget $op $budget, enforced on macOS only)"
  else
    echo "  FAIL $name: $v (budget $op $budget)"; FAIL=1
  fi
}
field() { python3 -c "import json,sys; d=json.load(sys.stdin); print(eval('d'+sys.argv[1]))" "$1"; }

echo "== Empty document, idle"
run_app ""
sleep 14   # past launch, so the last 10 s are pure idle
P=$(perf)
check "heap MB" "$(echo "$P" | field '["heap_mb"]')" "<=" 40
check "repaints in 10 s idle" "$(echo "$P" | field '["repaints_last_10s"]')" "<=" 2
echo "       repaint causes: $(echo "$P" | field '.get("repaint_causes_last_10s", {})')"
stop

echo "== Dense deck (2,000 elements, 50 photos)"
"$BIN/examples/dense_deck" "$WORK/dense.zebra"
run_app "$WORK/dense.zebra"
sleep 6
for i in $(seq 60); do "$BIN/zebractl" --socket "$SOCK" view '{"op":"pan","dx":-40,"dy":-25}' >/dev/null; done
for i in $(seq 20); do "$BIN/zebractl" --socket "$SOCK" view '{"op":"zoom","factor":1.15}' >/dev/null; done
P=$(perf)
echo "$P" | python3 -c "import json,sys; d=json.load(sys.stdin); print('  ', {k: d[k] for k in ('frame_ms_p50','frame_ms_p95','caches','document')})"
check "heap MB" "$(echo "$P" | field '["heap_mb"]')" "<=" 150
check "decoded image cache MB" "$(echo "$P" | field '["caches"]["image_kb"]/1024')" "<=" 256
check "frame p95 ms (pan/zoom)" "$(echo "$P" | field '["frame_ms_p95"]')" "<=" 16.7 mac-only
check "undo history KB" "$(echo "$P" | field '["document"]["history_kb"]')" "<=" 5120
stop

echo "== Undo history, 500 edits"
H=$("$BIN/examples/history_mem" | grep "after 500 edits")
echo "  $H"
HMB=$(echo "$H" | sed -E 's/.*history ([0-9.]+) MB.*/\1/')
check "history MB after 500 edits" "$HMB" "<=" 5

if [[ $FAIL == 0 ]]; then echo "All budgets met."; else echo "Some budgets were broken."; fi
exit $FAIL
