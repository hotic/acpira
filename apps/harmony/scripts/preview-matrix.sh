#!/usr/bin/env bash
# One page in the Previewer at every window a foldable, a tablet or a phone gives it, both ways up, one JPEG per form.
# The sizes are the full-screen windows in vp of Huawei's best-practice pages "阔折叠/阔直板应用开发" and "三折叠应用开发"
# (the tablet is a round 800 x 1280). The layout follows the window's breakpoints alone, so these sizes are the whole
# check short of an unfold on a device (scripts/preview.sh for one size).
# Usage: apps/harmony/scripts/preview-matrix.sh <out-dir> [page] [dark|light]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="$1"; page="${2:-pages/PreviewSplit}"; mode="${3:-dark}"
mkdir -p "$out"
# name width height: breakpoints (width x height) in the comment
forms=(
  "phone 360 780"               # sm x lg
  "phone-landscape 780 360"     # md x sm
  "purax-outer 326 326"         # sm x md, the small square
  "purax 440 707"               # sm x lg, the wide slab
  "purax-landscape 707 440"     # md x sm
  "puraxmax-folded 459 672"     # sm x lg
  "puraxmax-folded-landscape 672 459" # md x sm
  "puraxmax 939 664"            # lg x sm
  "puraxmax-portrait 664 939"   # md x lg
  "matext-single 350 776"       # sm x lg
  "matext-double 712 776"       # md x md, the large square
  "matext-triple 1107 776"      # lg x sm
  "matext-triple-portrait 776 1107" # md x lg
  "tablet 1280 800"             # lg x sm
  "tablet-portrait 800 1280"    # md x lg
)
for form in "${forms[@]}"; do
  read -r name w h <<<"$form"
  # 320 dpi: two px a vp, which keeps the widest form under the Previewer's cap
  "$here/preview.sh" "$out/$name.jpg" $((w * 2)) $((h * 2)) "$mode" "$page" 320 8 >/dev/null
  echo "$out/$name.jpg"
done
