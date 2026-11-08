#!/usr/bin/env bash
# Builds acpira-mobile-ohos (rust/crates/acpira-mobile-ohos: rust/crates/acpira-mobile, the phone's side of paired remote
# access, exported through N-API) into the shared library the app's remote/ code imports, one per ABI
# (apps/android/scripts/build-net.sh is the Android counterpart):
#   entry/libs/<abi>/libacpira_mobile_ohos.so   arm64-v8a for devices, x86_64 for the emulator; hvigor packs libs/ into the HAP
#   entry/libs/stamp                            when it was built (--if-stale compares the sources against it)
# All of it is build output, git-ignored. Without it the app builds and runs, with the pairing pages saying the build
# has no remote access.
#   OHOS_SDK  the SDK root holding <api>/native (default ~/Library/OpenHarmony/sdk); OHOS_API the API level (default 23)
#   ABIS      the ABIs to build (default "arm64-v8a x86_64")
#   PROFILE   the Cargo profile (default mobile: size first, as the iOS and Android libraries; `dev` builds faster)
# Usage: apps/harmony/scripts/build-net.sh [--if-stale | --clippy]   (--clippy: lint the crate, N-API layer included, for each ABI instead)
set -euo pipefail

harmony="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$harmony/../.." && pwd)"
rust="$repo/rust"
out="$harmony/entry/libs"
# Outside apps/harmony/build, which hvigor clears
work="$rust/target/ohos"
read -r -a abis <<<"${ABIS:-arm64-v8a x86_64}"
profile="${PROFILE:-mobile}"
sdk="${OHOS_SDK:-$HOME/Library/OpenHarmony/sdk}/${OHOS_API:-23}"
llvm="$sdk/native/llvm/bin"
[ -x "$llvm/clang" ] || { echo "no OpenHarmony native toolchain at $sdk/native (OHOS_SDK / OHOS_API)" >&2; exit 1; }

triple() {
  case "$1" in
    arm64-v8a) echo aarch64-unknown-linux-ohos ;;
    x86_64) echo x86_64-unknown-linux-ohos ;;
    *) echo "unknown ABI $1" >&2; exit 1 ;;
  esac
}

# Newer than the stamp: anything the library is built from (acpira-shared's i18n tables are read by none of it)
stale() {
  [ -f "$out/stamp" ] || return 0
  for abi in "${abis[@]}"; do [ -f "$out/$abi/libacpira_mobile_ohos.so" ] || return 0; done
  [ -n "$(find "$rust/crates/acpira-mobile-ohos" "$rust/crates/acpira-mobile" "$rust/crates/acpira-net" \
    "$rust/crates/acpira-shared" "$rust/patches" "$rust/Cargo.toml" "$rust/Cargo.lock" "$0" -newer "$out/stamp" -type f \
    -not -path '*/i18n/*' -not -path '*/target/*' -print -quit)" ]
}

mode="${1:-}"
if [ "$mode" = "--if-stale" ]; then
  stale || exit 0
  echo "== acpira-mobile changed since the last build: rebuilding the HarmonyOS library"
fi

installed="$(rustup target list --installed)"
for abi in "${abis[@]}"; do
  t="$(triple "$abi")"
  grep -qx "$t" <<<"$installed" || rustup target add "$t"
done

export CARGO_TARGET_DIR="$work/target"
# build.rs links the system's N-API from this SDK's sysroot
export OHOS_NDK_HOME="$sdk"
for abi in "${abis[@]}"; do
  t="$(triple "$abi")"
  # The SDK's clang wrappers carry the target and the sysroot: cc-rs (ring's C parts) and rustc's link step both use them
  env_t="$(tr '[:lower:]-' '[:upper:]_' <<<"$t")"
  export "CARGO_TARGET_${env_t}_LINKER=$llvm/$t-clang"
  export "CC_${t//-/_}=$llvm/$t-clang" "AR_${t//-/_}=$llvm/llvm-ar"
  if [ "$mode" = "--clippy" ]; then
    echo "== cargo clippy acpira-mobile-ohos for $abi"
    (cd "$rust" && cargo clippy --locked --no-deps -p acpira-mobile-ohos --all-targets --target "$t" -- -D warnings)
    continue
  fi
  echo "== cargo build acpira-mobile-ohos for $abi ($profile)"
  # Trap (as on Android): Cargo skips LTO for a library also built as an rlib; asking for the cdylib alone lets it run
  (cd "$rust" && cargo rustc --locked -p acpira-mobile-ohos --lib --crate-type cdylib --profile "$profile" --target "$t")
done
[ "$mode" = "--clippy" ] && exit 0

dir="$profile"
[ "$profile" = "dev" ] && dir=debug
stage="$work/stage"
rm -rf "$stage" && mkdir -p "$stage/libs"
for abi in "${abis[@]}"; do
  mkdir -p "$stage/libs/$abi"
  "$llvm/llvm-strip" --strip-all -o "$stage/libs/$abi/libacpira_mobile_ohos.so" \
    "$CARGO_TARGET_DIR/$(triple "$abi")/$dir/libacpira_mobile_ohos.so"
done
date -u +%Y-%m-%dT%H:%M:%SZ > "$stage/libs/stamp"

# Swapped in whole: a hvigor build running meanwhile sees the old output or the new one, never none
rm -rf "$out.old"
[ -d "$out" ] && mv "$out" "$out.old"
mv "$stage/libs" "$out"
rm -rf "$out.old" "$stage"

echo "== acpira-mobile-ohos ready in $out"
for abi in "${abis[@]}"; do
  printf '   %-12s %s\n' "$abi" "$(du -h "$out/$abi/libacpira_mobile_ohos.so" | cut -f1)"
done
