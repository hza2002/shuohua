#!/usr/bin/env bash
# Launch all 24 NSGlassEffectView variants tiled across the screen at once.
# ⌘Q in any window quits that instance; close-all helper at the bottom.
set -euo pipefail

cd "$(dirname "$0")"
cargo build --release >/dev/null

BIN=./target/release/liquid-glass-demo
PIDS=()

for i in $(seq 0 23); do
  JT_VARIANT="$i" JT_TILE_INDEX="$i" "$BIN" >/dev/null 2>&1 &
  PIDS+=($!)
done

echo "Launched 24 instances (variants 0..23). PIDs: ${PIDS[*]}"
echo "Press Ctrl+C here to kill all at once, or ⌘Q each window."

trap 'echo "Killing..."; kill "${PIDS[@]}" 2>/dev/null; wait' INT TERM
wait
