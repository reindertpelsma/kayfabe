#!/usr/bin/env bash
set -euo pipefail
if [[ $# != 2 ]]; then
    echo "usage: $0 TRUSTED_OGKM_CHECKOUT NEW_OUTPUT_DIRECTORY" >&2
    exit 2
fi
source_root=$(realpath "$1")
script_dir=$(cd -- "$(dirname -- "$0")" && pwd)
umask 077
mkdir -- "$2"
output_dir=$(realpath "$2")
generated="$source_root/src/nvidia/generated"
sources=()
for name in all_dcl crashcat engines gr gsp journal nvdebug rc regs; do
    sources+=("$generated/g_${name}_pb.c")
done
cc -std=c11 -O2 -Wall -Wextra -DPRB_ENUM_NAMES=1 -DPRB_FIELD_NAMES=1 \
    -DPRB_MESSAGE_NAMES=1 -I"$source_root/src/common/inc" \
    -I"$source_root/src/common/sdk/nvidia/inc" -I"$generated" \
    "$script_dir/nvcd-schema.c" "${sources[@]}" -o "$output_dir/schema-emitter"
"$output_dir/schema-emitter" > "$output_dir/schema.json"
sha256sum "$script_dir/nvcd-schema.c" "$source_root/src/common/inc/prbrt.h" \
    "$source_root/src/common/sdk/nvidia/inc/nvcd.h" \
    "$source_root/src/common/sdk/nvidia/inc/rmcd.h" \
    "${sources[@]}" "$generated"/g_*_pb.h > "$output_dir/source-sha256.txt"
echo "Wrote compiler-derived schema to $output_dir/schema.json"
