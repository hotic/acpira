#!/usr/bin/env bash
# Builds the app, runs it on a simulator and saves a screenshot: the iOS counterpart of the LAB screenshots.
# The simulator stays booted afterwards, so `open -a Simulator` shows the same screen, until it has been idle
# for ACPIRA_SIM_IDLE seconds (simulator.sh: runs take turns on one device, which shuts down when unused).
# Usage: apps/ios/scripts/shot.sh [output.png] [simulator name] [launch arguments…]
#   defaults /tmp/acpira-ios.png and the scripts' own "Acpira Scripts" (simulator.sh); arguments go to the app, e.g. -fixture running -theme light -motion none
#   SKIP_BUILD=1 reuses the last build (several shots in a row)
set -euo pipefail

ios="$(cd "$(dirname "$0")/.." && pwd)"
. "$ios/scripts/simulator.sh"
out="${1:-/tmp/acpira-ios.png}"
sim="${2:-$sim_default}"
shift $(( $# > 2 ? 2 : $# ))
bundle="com.hotic.acpira"
derived="$ios/build/DerivedData"

# xcode-select may still point at the Command Line Tools, which ship no iOS SDK
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"

sim_acquire "$sim"

if [ -z "${SKIP_BUILD:-}" ]; then
  xcodebuild -project "$ios/Acpira.xcodeproj" -scheme Acpira -configuration Debug \
    -destination "platform=iOS Simulator,name=$sim" -derivedDataPath "$derived" -quiet build
fi

# Boots the device when it is shut down and waits until it is ready
xcrun simctl bootstatus "$sim" -b >/dev/null
xcrun simctl install "$sim" "$derived/Build/Products/Debug-iphonesimulator/Acpira.app"
xcrun simctl terminate "$sim" "$bundle" 2>/dev/null || true
xcrun simctl launch "$sim" "$bundle" "$@" >/dev/null

# Give the first frame time to settle before capturing
sleep 3
xcrun simctl io "$sim" screenshot "$out" >/dev/null
echo "$out"
