#!/usr/bin/env bash
set -euo pipefail
# Build on the GPU host. Point OGKM_ROOT at an open 595.91.07 source tree.
: "${OGKM_ROOT:?set OGKM_ROOT to the open GPU kernel module source}"
source_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
output=${1:-./e5_alloc_map}
nice -n 19 cc -std=gnu11 -O2 -Wall -Wextra -Werror \
    -I"$OGKM_ROOT/src/common/sdk/nvidia/inc" \
    -I"$OGKM_ROOT/src/common/inc" \
    -I"$OGKM_ROOT/src/nvidia/arch/nvalloc/unix/include" \
    -I"$OGKM_ROOT/src/common/inc/swref/published/turing/tu102" \
    "$source_dir/e5_alloc_map.c" -o "$output"
