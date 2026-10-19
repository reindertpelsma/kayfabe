#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# ★ DERIVE the display capabilities page's FAMILY ROWS (docs/design/V3_DISPLAY_CAPS_DERIVATION_20261014.md).
#
# Two jobs, neither of which parses C or reads a captured page:
#   1. CHECK every register/field named by crates/kf-disp/data/caps_rows.tsv (the hand-kept window-layer
#      family rows) against ogkm's own class headers, by asking the preprocessor (`gcc -E -dM`, as
#      tools/derive_display_classes.sh does): a required row whose name is missing from any family's caps
#      class is a failure, an optional row only a note.
#   2. REGENERATE crates/kf-disp/data/caps_family_pages.tsv — the page each family's caps class gets from
#      the T1 rules alone (no measured row), with the tier and reason of every word — by running
#      kf_disp::capsderive (the one implementation) through `cargo test -- --ignored emit_family_rows`.
#      A GPU-free test (`the_committed_family_rows_are_the_derivation`) holds the committed file to it.
#
# The measured (tier 3) words are NOT generated: they are read from a real die with
# tools/read_display_caps_page.py and added to crates/kf-disp/data/caps_measured.tsv by hand, with a source.
#
# usage: tools/derive_display_caps.sh [path-to-ogkm]
set -euo pipefail
export LC_ALL=C
OG=${1:-/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04}
HERE="$(cd "$(dirname "$0")/.." && pwd)"
INC="$OG/src/common/sdk/nvidia/inc"
ROWS="$HERE/crates/kf-disp/data/caps_rows.tsv"
[ -f "$INC/class/clc773.h" ] || { echo "⊘ no class headers at $INC" >&2; exit 2; }
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
CLASSES="c573 c673 c773 ca73"
{ echo '#include "nvtypes.h"'; for c in $CLASSES; do echo "#include \"class/cl$c.h\""; done; } > "$tmp/all.h"
gcc -E -dM -I"$INC" "$tmp/all.h" | sed -n 's/^#define \(NV[A-Z0-9]*_[A-Za-z0-9_]*\).*/\1/p' | sort -u > "$tmp/names"
bad=0
while IFS=$'\t' read -r kind layer reg field value req basis; do
    case "$kind" in LAYER) ;; *) continue ;; esac
    for c in $CLASSES; do
        C=$(echo "$c" | tr a-z A-Z)
        n="NV${C}_${reg}_${field}"
        if grep -qx "$n" "$tmp/names"; then
            case "$value" in TRUE|FALSE) grep -qx "${n}_${value}" "$tmp/names" || { echo "⊘ $n lacks _$value"; bad=1; } ;; esac
        elif [ "$req" = required ]; then
            echo "⊘ required row $n is not in ogkm's class header"; bad=1
        else
            echo "  note: optional row $n absent from NV${C} (skipped for that family)"
        fi
    done
done < "$ROWS"
[ "$bad" = 0 ] || exit 1
echo "caps rows: every required name resolves in ogkm $(sed -n 's/^NVIDIA_VERSION = //p' "$OG/version.mk")"
out="$HERE/crates/kf-disp/data/caps_family_pages.tsv"
( cd "$HERE" && cargo test --release -q -p kf-disp --test caps_derive -- --ignored emit_family_rows --nocapture 2>/dev/null ) \
    | sed -n '/^KF_FAMILY_ROWS_BEGIN$/,/^KF_FAMILY_ROWS_END$/p' | sed '1d;$d' > "$tmp/rows.tsv"
[ -s "$tmp/rows.tsv" ] || { echo "⊘ the emitter printed nothing" >&2; exit 1; }
cp "$tmp/rows.tsv" "$out"
echo "wrote $out ($(grep -vc '^#' "$out") rows)"
