#!/usr/bin/env bash
# One screenshot of a page in the SDK's headless Previewer, from this tree's last build (scripts/build.sh first).
# Each run unpacks the HAP into its own temp folder, serves on a free port and stops only its own Previewer, so several
# runs (worktrees) can go at once. The Previewer has no input, no network and no ability (docs/dev/harmony.md).
# Usage: apps/harmony/scripts/preview.sh <out.jpg> [width height] [dark|light] [page] [density] [wait-s]
#   page: pages/Index (the demo list; a wide size shows the split) or pages/Preview (chat fixtures, ets/pages/Preview.ets)
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
out="$1"; w="${2:-1080}"; h="${3:-2340}"; mode="${4:-dark}"; page="${5:-pages/Index}"; dpi="${6:-480}"; wait="${7:-6}"
sdk="${OHOS_SDK:-$HOME/Library/OpenHarmony/sdk}"
bin="$(ls -d "$sdk"/*/previewer/common/bin | sort -V | tail -1)"
hap="$here/entry/build/default/outputs/default/entry-default-unsigned.hap"
[ -f "$hap" ] || { echo "no $hap: run scripts/build.sh first" >&2; exit 1; }
work="$(mktemp -d /tmp/acpira-harmony-preview-XXXX)"
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
pid=""
trap '[ -n "$pid" ] && kill "$pid" 2>/dev/null; wait 2>/dev/null; rm -rf "$work"' EXIT
unzip -q "$hap" -d "$work/hap"
orientation=portrait; [ "$w" -gt "$h" ] && orientation=landscape
(cd "$bin" && exec ./Previewer -refresh region -projectID 1 -ts "trace_$port" -j "$work/hap/ets" -s "acpira_$port" \
  -cpm false -device phone -shape rect -sd "$dpi" -or "$w" "$h" -cr "$w" "$h" -n entry -av ACE_2_0 -url "$page" \
  -pages main_pages -arp "$here/entry/build/default/intermediates/res/default" -pm Stage -l zh_CN -cm "$mode" \
  -o "$orientation" -lws "$port" -ljPath "$here/entry/build/default/intermediates/loader/default/loader.json" \
  -abn EntryAbility -abp ets/entryability/EntryAbility.ts -staticCard false -foldable false) >"$work/previewer.log" 2>&1 &
pid=$!
sleep "$wait"
node "$here/scripts/preview-frame.mjs" "$port" "$out" 2000 || { tail -20 "$work/previewer.log" >&2; exit 1; }
