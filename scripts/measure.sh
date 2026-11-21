#!/bin/bash
# Reports the numbers behind SC-001 (idle memory, measured as phys_footprint;
# RSS is informational only), SC-002 (bundle size), SC-004 (idle CPU) and
# FR-022 (no network). Launches the bundled binary
# directly so it can be killed afterwards.
set -euo pipefail
APP=${1:-build/snapit.app}
BIN="$APP/Contents/MacOS/snapit"
SETTLE=${SETTLE:-5}
SAMPLE=${SAMPLE:-30}

pkill -x snapit 2>/dev/null || true
echo "bundle size:   $(du -sk "$APP" | cut -f1) KB   (binary $(( $(stat -f%z "$BIN") / 1024 )) KB)"

SNAPIT_NO_ONBOARDING=1 "$BIN" 2>/dev/null &
PID=$!
trap 'kill $PID 2>/dev/null || true' EXIT
sleep "$SETTLE"

echo "idle RSS:      $(( $(ps -o rss= -p $PID) / 1024 )) MB"
if command -v footprint >/dev/null; then
  echo "phys footprint: $(footprint -p $PID 2>/dev/null | awk '/phys_footprint:/ {print $2, $3; exit}')"
fi

T0=$(ps -o time= -p $PID)
sleep "$SAMPLE"
T1=$(ps -o time= -p $PID)
echo "CPU time:      $T0 -> $T1 over ${SAMPLE}s"
echo "sockets:       $(lsof -nP -a -i -p $PID 2>/dev/null | tail -n +2 | wc -l | tr -d ' ')"
