#!/usr/bin/env bash
# The "done" check for Swift changes under apps/ios/:
#   1. AcpiraKit's tests on the Mac (`swift test`): the fast loop, and proof every module still builds for macOS
#   2. AcpiraKit's tests on an iOS simulator: the same suites against the iOS SDK
#   3. The app scheme on the simulator: builds the app and runs its hosted tests
# Usage: apps/ios/scripts/check.sh [simulator name]   (default the scripts' own "Acpira Scripts", simulator.sh)
set -euo pipefail

ios="$(cd "$(dirname "$0")/.." && pwd)"
. "$ios/scripts/simulator.sh"
sim="${1:-$sim_default}"
destination="platform=iOS Simulator,name=$sim"

# xcode-select may still point at the Command Line Tools, which ship no iOS SDK
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"

echo "== AcpiraKit on macOS"
(cd "$ios/Packages/AcpiraKit" && swift test)

sim_acquire "$sim"

echo "== AcpiraKit on $sim"
(cd "$ios/Packages/AcpiraKit" && xcodebuild -scheme AcpiraKit -destination "$destination" \
  -derivedDataPath "$ios/build/PackageDerivedData" -quiet test)

echo "== Acpira app on $sim"
xcodebuild -project "$ios/Acpira.xcodeproj" -scheme Acpira -destination "$destination" \
  -derivedDataPath "$ios/build/DerivedData" -quiet test

echo "== all green"
