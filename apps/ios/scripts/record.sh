#!/usr/bin/env bash
# Records the app on a simulator: the motion counterpart of shot.sh (row entrances, paced text, the Orb).
# Usage: apps/ios/scripts/record.sh [output.mp4] [seconds] [launch arguments…]
#   defaults /tmp/acpira-ios.mp4, 12 s; e.g. apps/ios/scripts/record.sh /tmp/r.mp4 12 -fixture streaming -replay
#   SIM picks the simulator (default the scripts' own "Acpira Scripts", simulator.sh); SKIP_BUILD=1 reuses the last build.
#   A contact sheet of the recording: ffmpeg -i r.mp4 -vf "fps=4,scale=402:-1,tile=8x2" sheet%d.png
set -euo pipefail

ios="$(cd "$(dirname "$0")/.." && pwd)"
. "$ios/scripts/simulator.sh"
out="${1:-/tmp/acpira-ios.mp4}"
seconds="${2:-12}"
shift $(( $# > 2 ? 2 : $# ))
sim="${SIM:-$sim_default}"
bundle="com.hotic.acpira"
derived="$ios/build/DerivedData"
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"

# Held through the recording, so another session's launch cannot cut into it
sim_acquire "$sim"

if [ -z "${SKIP_BUILD:-}" ]; then
  xcodebuild -project "$ios/Acpira.xcodeproj" -scheme Acpira -configuration Debug \
    -destination "platform=iOS Simulator,name=$sim" -derivedDataPath "$derived" -quiet build
fi

xcrun simctl bootstatus "$sim" -b >/dev/null
xcrun simctl install "$sim" "$derived/Build/Products/Debug-iphonesimulator/Acpira.app"
xcrun simctl terminate "$sim" "$bundle" 2>/dev/null || true

# The recorder finalises the file on SIGINT
xcrun simctl io "$sim" recordVideo --codec h264 --force "$out" >/dev/null 2>&1 &
recorder=$!
sleep 1.5
xcrun simctl launch "$sim" "$bundle" "$@" >/dev/null
sleep "$seconds"
kill -INT "$recorder"
wait "$recorder" 2>/dev/null || true
echo "$out"
