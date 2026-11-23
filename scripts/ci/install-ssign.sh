#!/usr/bin/env bash
# Puts `ssign` (scripts/release/sign-windows.mjs) on a Windows runner's PATH, next to cargo: the upstream release binary
# of the pinned version, checked against its SHA-256. Building it from source costs minutes in every job that signs,
# and a Rust cache cannot spare them: a release runs on its tag, which reads no cache another tag saved. A download
# that fails or does not match falls back to the source build.
set -euo pipefail
version=0.1.9
sha256=c5406a993d4a952aa76bea494c173708103d1ac33817151b1d5dfd0faa16252a
bin=$(dirname "$(command -v cargo)")
cd "$RUNNER_TEMP"
if curl -fsSL --retry 3 -o ssign.zip "https://github.com/Le-Syl21/ssign/releases/download/v$version/ssign-windows-x86_64.zip" \
  && [ "$(sha256sum ssign.zip | cut -d' ' -f1)" = "$sha256" ] \
  && unzip -oq ssign.zip ssign.exe -d "$bin"; then
  echo "ssign $version: release binary"
else
  echo "::warning::The ssign $version release binary could not be fetched or did not match its checksum; building from source"
  cargo install ssign --version "$version" --locked
fi
command -v ssign
