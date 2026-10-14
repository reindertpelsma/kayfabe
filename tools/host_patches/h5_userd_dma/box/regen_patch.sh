#!/usr/bin/env bash
# Regenerate the patch from two kernel-open trees (a = pristine, b = edited), deterministic headers.
#   bash regen_patch.sh <dir-containing-a-and-b> > patch/nvidia_h5_userd_dma_<ver>.patch
set -euo pipefail
cd "${1:?dir containing a/ and b/}"
diff -ruN a b | sed -E 's/^(---|\+\+\+) ((a|b)\/[^\t]*)\t.*/\1 \2/'
