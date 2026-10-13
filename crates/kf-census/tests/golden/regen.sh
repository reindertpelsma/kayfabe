#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Rebuild (or check) tests/golden/conversation.kfgspcap with the Windows capture module's own
# writer.
#
#   KFWC=/path/to/tools/windows-capture tests/golden/regen.sh          # rewrite the golden
#   KFWC=/path/to/tools/windows-capture tests/golden/regen.sh --check  # exit 1 if it differs
#
# --check is what the module's CI runs (docs/design/V3_GSP_CAPTURE_FORMAT.md section 9): the
# module's writer must keep producing exactly the bytes this crate's decoder is tested on.
# Build output goes to $OUT (default /tmp/kfwc-golden) and is removed afterwards.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
: "${KFWC:?set KFWC to the tools/windows-capture directory of the module checkout}"
OUT=${OUT:-/tmp/kfwc-golden}
mkdir -p "$OUT"
trap 'rm -rf "$OUT"' EXIT
LIB="$KFWC/lib"
${CC:-gcc} -std=c99 -O1 -g -Wall -Wextra -Wshadow -Wno-unused-function -I"$LIB" -I"$KFWC/tests" \
    -fsanitize=address,undefined -fno-sanitize-recover=undefined \
    "$here/gen_golden.c" "$LIB/kfwc_record.c" "$LIB/kfwc_elem.c" "$LIB/kfwc_discover.c" "$LIB/kfwc_engine.c" \
    -o "$OUT/gen_golden"
"$OUT/gen_golden" "$here/conversation.runs" "$OUT/conversation.kfgspcap"
if [[ ${1:-} == --check ]]; then
    cmp "$OUT/conversation.kfgspcap" "$here/conversation.kfgspcap" \
        && echo "golden capture reproduced byte for byte by the module's writer"
else
    cp "$OUT/conversation.kfgspcap" "$here/conversation.kfgspcap"
    echo "wrote $here/conversation.kfgspcap"
fi
