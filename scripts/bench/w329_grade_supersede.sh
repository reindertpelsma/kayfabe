#!/usr/bin/env bash
# ★★★★★ w329 — GRADE THE ARM THAT WOULD SHIP.
#
# The three workloads and the 36 MiB correctness row were measured on the `on` (leg 1) arm,
# which arm A2 then showed to be a **no-op** on this workload once re-maps are excluded. The
# arm that actually fixes `28,31` is `supersede`, and ⊘ a green measured on a different arm is
# not a green for this one. Same reason `a_flag_is_not_progress` exists.
#
#   PREDICT  ^CUP3_VAL=43 x3, ^CUP8_BAD=0 ^CUP8_MAXERR=0 x2, R33 arm-1 COPY x2 byte-identical,
#            B3072_BAD=0 B3072_MAXERR=0, Xid=0 everywhere, 40 unserviced ids.
#   ⊘ A red on ANY of these makes `supersede` unshippable regardless of what it does to `28,31`.
set -uo pipefail
TREE=${1:-/workspace/kayfabe_w329}
A="$TREE/scripts/bench/w329_arm.sh"
T() { touch "$TREE/crates/kayfabe-util/src/lib.rs"; }
echo "=== W329 GRADE-SUPERSEDE START $(date -Is) tree=$TREE HEAD=$(cd "$TREE" && git rev-parse --short HEAD)"

bash "$A" "$TREE" w329sg 3 KAYFABE_JOIN_RELEASE=supersede
echo "=== cup3 n=3 done $(date -Is)"; T

W329_WORKLOAD="w308_cup8.sh cup8" bash "$A" "$TREE" w329se 2 KAYFABE_JOIN_RELEASE=supersede
echo "=== cup8 n=2 done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh sizes" bash "$A" "$TREE" w329ss 1 KAYFABE_JOIN_RELEASE=supersede KAYFABE_BENCH_SIZES=3072 KAYFABE_BENCH_ITERS=3
echo "=== cup8 N=3072 done $(date -Is)"; T

W329_WORKLOAD="w309_crit1.sh fresh" bash "$A" "$TREE" w329sx 2 KAYFABE_JOIN_RELEASE=supersede
echo "=== R33 arm 1 n=2 done $(date -Is)"

echo "=== W329 GRADE-SUPERSEDE TERMINATOR rc=0 $(date -Is)"
