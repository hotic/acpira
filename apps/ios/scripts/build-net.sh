#!/usr/bin/env bash
# Builds acpira-mobile (rust/crates/acpira-mobile, the phone's side of paired remote access) into the XCFramework and
# Swift bindings AcpiraKit's `AcpiraNet` target wraps:
#   Packages/AcpiraKit/Artifacts/AcpiraNetFFI.xcframework   static libraries for the iPhone, the simulator and this Mac
#                                                            (the Mac slice keeps `swift test` linking), plus the C header
#   Packages/AcpiraKit/Artifacts/AcpiraNet/acpira_mobile.swift   the UniFFI bindings
#   Packages/AcpiraKit/Artifacts/stamp                        when it was built (check.sh compares the sources against it)
# All of it is build output, git-ignored; Package.swift declares AcpiraNet and AcpiraRemote only while it exists.
# The `mobile` Cargo profile (rust/Cargo.toml) trades build time for size: whole-program LTO in one codegen unit.
# Usage: apps/ios/scripts/build-net.sh [--if-stale]   (--if-stale: do nothing when the artifacts are newer than the sources)
set -euo pipefail

ios="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$ios/../.." && pwd)"
rust="$repo/rust"
out="$ios/Packages/AcpiraKit/Artifacts"
work="$ios/build/net"
targets=(aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin)

# Newer than the stamp: anything the library is built from. acpira-shared's i18n tables are left out, since the library
# reads none of them and other work edits them all day
stale() {
  [ -f "$out/stamp" ] && [ -d "$out/AcpiraNetFFI.xcframework" ] && [ -f "$out/AcpiraNet/acpira_mobile.swift" ] || return 0
  [ -n "$(find "$rust/crates/acpira-mobile" "$rust/crates/acpira-net" "$rust/crates/acpira-shared" \
    "$rust/Cargo.toml" "$rust/Cargo.lock" "$0" -newer "$out/stamp" -type f \
    -not -path '*/i18n/*' -not -path '*/target/*' -print -quit)" ]
}

if [ "${1:-}" = "--if-stale" ]; then
  stale || exit 0
  echo "== acpira-mobile changed since the last build: rebuilding the XCFramework"
fi

# xcode-select may still point at the Command Line Tools, which ship no iOS SDK
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
# The package's platforms (Package.swift): the C parts (ring) and rustc's own objects target the same minimum
export IPHONEOS_DEPLOYMENT_TARGET=26.0 MACOSX_DEPLOYMENT_TARGET=26.0

installed="$(rustup target list --installed)"
for t in "${targets[@]}"; do
  grep -qx "$t" <<<"$installed" || rustup target add "$t"
done

# The library builds in a copy of its three crates with a lock of its own, so the pin below never touches rust/Cargo.lock
# (the engine and the desktop app stay on the libc they test). Trap: libc 0.2.190 limited `in6_ifreq` to macOS, and
# netdev 0.46 / 0.47 (iroh's netwatch) still use it on every Apple target, so the iOS slices need libc 0.2.189 until
# one of them moves
ws="$work/ws"
mkdir -p "$ws/crates"
for c in acpira-shared acpira-net acpira-mobile; do
  rsync -a --delete --exclude target "$rust/crates/$c/" "$ws/crates/$c/"
done
sed -E 's|^members = \[.*\]$|members = ["crates/acpira-shared", "crates/acpira-net", "crates/acpira-mobile"]|' "$rust/Cargo.toml" > "$ws/Cargo.toml"
# acpira-mobile's tests reach the gateway and the engine, which the copy leaves out
awk '/^\[/ { skip = ($0 == "[dev-dependencies]") } !skip' "$rust/crates/acpira-mobile/Cargo.toml" > "$ws/crates/acpira-mobile/Cargo.toml"
cp "$rust/Cargo.lock" "$ws/Cargo.lock"
(cd "$ws" && { cargo update --quiet --offline -p libc --precise 0.2.189 2>/dev/null || cargo update --quiet -p libc --precise 0.2.189; })
export CARGO_TARGET_DIR="$work/target"

# Trap: Cargo skips LTO for a library that is also built as an rlib (the crate's `lib` type, for its tests), leaving
# every crate's whole object in the archive; asking for the staticlib alone lets the profile's LTO run
for t in "${targets[@]}"; do
  echo "== cargo build acpira-mobile for $t"
  (cd "$ws" && cargo rustc --locked -p acpira-mobile --lib --crate-type staticlib --profile mobile --target "$t")
done

# The bindings generator must be the exact UniFFI release the library links (it checks a contract version at load)
uniffi="$(awk '/^name = "uniffi"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$rust/Cargo.lock")"
bindgen="$work/tools/$uniffi/bin/uniffi-bindgen-swift"
if [ ! -x "$bindgen" ]; then
  echo "== installing uniffi-bindgen-swift $uniffi"
  cargo install --quiet --locked uniffi --version "=$uniffi" --features cli --bin uniffi-bindgen-swift --root "$work/tools/$uniffi"
fi

lib() { echo "$CARGO_TARGET_DIR/$1/mobile/libacpira_mobile.a"; }
stage="$work/stage"
rm -rf "$stage" && mkdir -p "$stage/headers" "$stage/AcpiraNet"
# Library mode reads the exported interface from the library itself; any slice describes the same interface. Not
# `--xcframework`: that writes a `framework module`, and this XCFramework holds plain static libraries
(cd "$ws" && "$bindgen" --swift-sources --headers --modulemap --metadata-no-deps \
  --module-name acpira_mobileFFI --modulemap-filename module.modulemap \
  "$(lib aarch64-apple-darwin)" "$stage/gen")
mv "$stage/gen/acpira_mobile.swift" "$stage/AcpiraNet/"
mv "$stage/gen/acpira_mobileFFI.h" "$stage/gen/module.modulemap" "$stage/headers/"

args=()
for t in "${targets[@]}"; do
  args+=(-library "$(lib "$t")" -headers "$stage/headers")
done
xcodebuild -create-xcframework "${args[@]}" -output "$stage/AcpiraNetFFI.xcframework" >/dev/null
date -u +%Y-%m-%dT%H:%M:%SZ > "$stage/stamp"

# Swapped in whole: a `swift test` running meanwhile sees the old artifacts or the new ones, never none
mkdir -p "$stage/out"
mv "$stage/AcpiraNetFFI.xcframework" "$stage/AcpiraNet" "$stage/stamp" "$stage/out/"
rm -rf "$out.old"
[ -d "$out" ] && mv "$out" "$out.old"
mv "$stage/out" "$out"
rm -rf "$out.old" "$stage"

echo "== AcpiraNet ready in $out"
for t in "${targets[@]}"; do
  printf '   %-22s %s\n' "$t" "$(du -h "$(lib "$t")" | cut -f1)"
done
