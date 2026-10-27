#!/usr/bin/env bash
# Full-length render of a fixture's chat (header, every exchange, dock) at 402 pt, @2x like the LAB reference shots.
# Usage: apps/ios/scripts/render.sh <out.png> <fixture> [closed|open|all|every] [dark|light] [simulator name]
#   open: the last exchange's process fold; all: plus its thoughts and plans; every: plus commands and edits
#   SKIP_BUILD=1 reuses the last build
set -euo pipefail

ios="$(cd "$(dirname "$0")/.." && pwd)"
. "$ios/scripts/simulator.sh"
out="$1"; fixture="$2"; mode="${3:-closed}"; theme="${4:-dark}"; sim="${5:-$sim_default}"
bundle="com.hotic.acpira"

# Held across the shot and the read of its output, which the next run would overwrite
sim_acquire "$sim"

# A render left by an earlier run would be copied before this one is written
data="$(xcrun simctl get_app_container "$sim" "$bundle" data 2>/dev/null || true)"
[ -n "$data" ] && rm -f "$data/tmp/acpira-render.png"
SKIP_BUILD="${SKIP_BUILD:-}" "$ios/scripts/shot.sh" /tmp/acpira-render-screen.png "$sim" \
  -fixture "$fixture" -render "$mode" -theme "$theme" -motion none -locale zh-CN >/dev/null
data="$(xcrun simctl get_app_container "$sim" "$bundle" data)"
# A long fixture takes longer than shot.sh's settle time to lay out and draw twice
for _ in $(seq 1 60); do [ -f "$data/tmp/acpira-render.png" ] && break; sleep 1; done
sleep 1
cp "$data/tmp/acpira-render.png" "$out"
echo "$out"
