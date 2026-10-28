#!/usr/bin/env bash
# The "done" check for Swift changes under apps/ios/:
#   0. acpira-mobile's XCFramework (build-net.sh), when missing or older than its Rust sources; ACPIRA_NET=0 skips it
#      and checks the package as a checkout without it sees it
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

# acpira-mobile's XCFramework, rebuilt when its Rust sources changed (AcpiraNet and AcpiraRemote build against it).
# SwiftPM and Xcode cache Package.swift's evaluation by environment, so the stamp of the build in the environment makes
# them see a fresh one
# ACPIRA_NET=0 checks the package without them, as a checkout that never built them sees it (Package.swift)
if [ "${ACPIRA_NET:-}" = 0 ]; then
  export ACPIRA_NET_STAMP=none
else
  "$ios/scripts/build-net.sh" --if-stale
  export ACPIRA_NET_STAMP="$(cat "$ios/Packages/AcpiraKit/Artifacts/stamp" 2>/dev/null || echo none)"
fi
# With a second library product (AcpiraRemote) Xcode names the scheme holding every test target AcpiraKit-Package
scheme=AcpiraKit
[ "$ACPIRA_NET_STAMP" = none ] || scheme=AcpiraKit-Package
# Builds without them keep their own derived data: a stale AcpiraRemote module left there by a build with them makes
# the app's `canImport(AcpiraRemote)` true while its FFI module is gone
dd=""
[ "$ACPIRA_NET_STAMP" = none ] && dd="-nonet"

echo "== AcpiraKit on macOS"
(cd "$ios/Packages/AcpiraKit" && swift test)

sim_acquire "$sim"

echo "== AcpiraKit on $sim"
# SwiftTerm's build-tool plugin (its version stamp) is never trusted in Xcode on a fresh checkout or machine, and
# xcodebuild refuses an untrusted plugin; every iOS script passes this flag
(cd "$ios/Packages/AcpiraKit" && xcodebuild -scheme "$scheme" -destination "$destination" \
  -derivedDataPath "$ios/build/PackageDerivedData$dd" -skipPackagePluginValidation -quiet test)

echo "== Acpira app on $sim"
xcodebuild -project "$ios/Acpira.xcodeproj" -scheme Acpira -destination "$destination" \
  -derivedDataPath "$ios/build/DerivedData$dd" -skipPackagePluginValidation -quiet test

echo "== all green"
