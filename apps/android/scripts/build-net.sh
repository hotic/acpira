#!/usr/bin/env bash
# Builds acpira-mobile (rust/crates/acpira-mobile, the phone's side of paired remote access) into the shared library and
# Kotlin bindings the app's `net` source set wraps (apps/ios/scripts/build-net.sh is the iOS counterpart):
#   net/jniLibs/<abi>/libacpira_mobile.so          one per ABI, linked against the NDK's libc
#   net/kotlin/uniffi/acpira_mobile/acpira_mobile.kt   the UniFFI bindings (JNA)
#   net/stamp                                      when it was built (--if-stale compares the sources against it)
# All of it is build output, git-ignored; app/build.gradle.kts compiles the paired-remote code only while it exists.
# ABIS picks the ABIs (default arm64-v8a: phones and the Apple-silicon emulator; add x86_64 for Intel emulators).
# Usage: apps/android/scripts/build-net.sh [--if-stale]   (--if-stale: do nothing when the output is newer than the sources)
set -euo pipefail

android="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$android/../.." && pwd)"
rust="$repo/rust"
out="$android/net"
work="$android/build/net"
read -r -a abis <<<"${ABIS:-arm64-v8a}"
# The app's minSdk: the NDK's clang wrappers carry the API level in their name
api=29

# Newer than the stamp: anything the library is built from (acpira-shared's i18n tables are read by none of it)
stale() {
  [ -f "$out/stamp" ] && [ -d "$out/jniLibs" ] && [ -f "$out/kotlin/uniffi/acpira_mobile/acpira_mobile.kt" ] || return 0
  for abi in "${abis[@]}"; do [ -f "$out/jniLibs/$abi/libacpira_mobile.so" ] || return 0; done
  [ -n "$(find "$rust/crates/acpira-mobile" "$rust/crates/acpira-net" "$rust/crates/acpira-shared" "$rust/patches" \
    "$rust/Cargo.toml" "$rust/Cargo.lock" "$0" "$android/scripts/uniffi.toml" -newer "$out/stamp" -type f \
    -not -path '*/i18n/*' -not -path '*/target/*' -print -quit)" ]
}

if [ "${1:-}" = "--if-stale" ]; then
  stale || exit 0
  echo "== acpira-mobile changed since the last build: rebuilding the Android library"
fi

sdk="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
ndk="${ANDROID_NDK_HOME:-$(find "$sdk/ndk" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -V | tail -1)}"
[ -d "$ndk" ] || { echo "no NDK under $sdk/ndk: install one with sdkmanager" >&2; exit 1; }
toolchain="$(echo "$ndk"/toolchains/llvm/prebuilt/*/bin)"

triple() {
  case "$1" in
    arm64-v8a) echo aarch64-linux-android ;;
    x86_64) echo x86_64-linux-android ;;
    *) echo "unknown ABI $1" >&2; exit 1 ;;
  esac
}

installed="$(rustup target list --installed)"
for abi in "${abis[@]}"; do
  t="$(triple "$abi")"
  grep -qx "$t" <<<"$installed" || rustup target add "$t"
done

export CARGO_TARGET_DIR="$work/target"
# The bindings generator reads the interface from symbols the profile's `strip` would remove: build with them, strip the
# copies that ship afterwards
export CARGO_PROFILE_MOBILE_STRIP=debuginfo
# Trap: Cargo skips LTO for a library that is also built as an rlib (the crate's `lib` type, for its tests); asking for
# the cdylib alone lets the `mobile` profile's LTO run, as the iOS script does for its staticlib
for abi in "${abis[@]}"; do
  t="$(triple "$abi")"
  # cc-rs (ring's C parts) and rustc's link step both use the NDK's clang for the app's API level
  env_t="$(tr '[:lower:]-' '[:upper:]_' <<<"$t")"
  export "CARGO_TARGET_${env_t}_LINKER=$toolchain/$t$api-clang"
  export "CC_${t//-/_}=$toolchain/$t$api-clang" "AR_${t//-/_}=$toolchain/llvm-ar"
  echo "== cargo build acpira-mobile for $abi"
  (cd "$rust" && cargo rustc --locked -p acpira-mobile --lib --crate-type cdylib --profile mobile --target "$t")
done

# The bindings generator must be the exact UniFFI release the library links (it checks a contract version at load)
uniffi="$(awk '/^name = "uniffi"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$rust/Cargo.lock")"
bindgen="$work/tools/$uniffi/bin/uniffi-bindgen"
if [ ! -x "$bindgen" ]; then
  echo "== installing uniffi-bindgen $uniffi"
  cargo install --quiet --locked uniffi --version "=$uniffi" --features cli --bin uniffi-bindgen --root "$work/tools/$uniffi"
fi

lib() { echo "$CARGO_TARGET_DIR/$(triple "$1")/mobile/libacpira_mobile.so"; }
stage="$work/stage"
rm -rf "$stage" && mkdir -p "$stage/out/jniLibs"
# Library mode reads the exported interface from the library itself; any ABI describes the same interface
(cd "$rust" && "$bindgen" generate --library "$(lib "${abis[0]}")" --language kotlin --no-format --metadata-no-deps \
  --config "$android/scripts/uniffi.toml" --out-dir "$stage/out/kotlin")
for abi in "${abis[@]}"; do
  mkdir -p "$stage/out/jniLibs/$abi"
  "$toolchain/llvm-strip" --strip-all -o "$stage/out/jniLibs/$abi/libacpira_mobile.so" "$(lib "$abi")"
done
date -u +%Y-%m-%dT%H:%M:%SZ > "$stage/out/stamp"

# Swapped in whole: a Gradle build running meanwhile sees the old output or the new one, never none
rm -rf "$out.old"
[ -d "$out" ] && mv "$out" "$out.old"
mv "$stage/out" "$out"
rm -rf "$out.old" "$stage"

echo "== acpira-mobile ready in $out"
for abi in "${abis[@]}"; do
  printf '   %-12s %s\n' "$abi" "$(du -h "$out/jniLibs/$abi/libacpira_mobile.so" | cut -f1)"
done
