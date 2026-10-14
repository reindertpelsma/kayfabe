#!/usr/bin/env bash
# GPU-free tests of the H5 patch: (1) include/ equals the headers the patch adds, (2) the patch
# applies cleanly (dry run) to every source tree given in H5_TREES (space separated; kernel-open
# layout), (3) the address-constraint model test.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
patch=$(ls "$root"/patch/nvidia_h5_userd_dma_*.patch | tail -1)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
# (1) the new files in the patch are exactly include/*
for f in nv-kf-host-patch.h nv-kf-dma-window.h; do
  awk -v F="common/inc/$f" '
    /^diff -ruN /{on=($3=="a/" F)} on&&/^@@/{body=1;next} on&&body&&/^\+/{print substr($0,2)}
    /^diff -ruN /&&!on{body=0}' "$patch" > "$tmp/$f"
  diff -u "$tmp/$f" "$root/include/$f" || { echo "include/$f differs from the patch"; exit 1; }
done
echo "include/ matches the patch"
# (2) dry runs
for t in ${H5_TREES:-}; do
  ( cd "$t" && patch -p1 --dry-run < "$patch" >/dev/null ) && echo "dry-run ok: $t" || { echo "DRY-RUN FAILED: $t"; exit 1; }
done
# (3) model test
gcc -std=gnu11 -Wall -Wextra -Werror -O1 -I "$here/stub" -I "$root/include" -include stddef.h \
    "$here/test_dma_window.c" -o "$tmp/test_dma_window"
"$tmp/test_dma_window"
