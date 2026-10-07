#!/usr/bin/env bash
# ★★★★★ w329 FOLLOW-UP — the re-map guard, its control, and leg 2.
#
# ⊘⊘ Arm B measured a REGRESSION: `4,64` PASSED for w327 (`w327u4`, `w327u4b`) and FAILED with
#    leg 1 armed (`w329b1`), revoking `va=0x7d05d0200000` — the workload's OUTPUT buffer, live
#    across both rows — because `settle` emits a RE-MAP as unbind+bind in ONE settlement and
#    `RevokeWholeJoins` could not tell it from a removal. The guard is in; these arms grade it.
#
#   BC   `4,64`, release OFF, n=2   — ★ THE SAME-BINARY CONTROL, asked FIRST. Until it answers,
#        "the release broke it" and "master drifted from df3043be to d859beb1" are both live.
#   B2   `4,64`, release ON,  n=2   — does the re-map guard restore w327u4b's PASS?
#        PREDICT last_ok=64, first_fail=NONE, and `remaps_refused>0` (the guard FIRED).
#        ⊘ `remaps_refused=0` beside a green means the guard was never reached and B2 grades
#          nothing — the known-positive for this arm is the counter, not the row.
#   A2   `28,31`, release ON, n=2   — leg 1's own numbers under the guard. PREDICT still FAILS
#        (the guard only stops a revoke, it adds none) with `revoked` LOWER than arm A's 8.
#   SUP  `28,31`, supersede, n=3    — leg 2. PREDICT `already_joined` refusals fall from 21
#        toward 0 and `last_ok=31`.
#   SUP64 `4,64`, supersede, n=1    — leg 2 must not re-introduce leg 1's regression.
set -uo pipefail
TREE=${1:-/workspace/kayfabe_w329}
A="$TREE/scripts/bench/w329_arm.sh"
T() { touch "$TREE/crates/kayfabe-util/src/lib.rs"; }
echo "=== W329 FOLLOWUP START $(date -Is) tree=$TREE HEAD=$(cd "$TREE" && git rev-parse --short HEAD)"

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329bc 2 KAYFABE_BENCH_BW=4,64 KAYFABE_JOIN_RELEASE=off
echo "=== arm BC (4,64 CONTROL, release OFF, n=2) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329b2 2 KAYFABE_BENCH_BW=4,64
echo "=== arm B2 (4,64 with the re-map guard, n=2) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329a2 2 KAYFABE_BENCH_BW=28,31
echo "=== arm A2 (28,31 with the re-map guard, n=2) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329sup 3 KAYFABE_BENCH_BW=28,31 KAYFABE_JOIN_RELEASE=supersede
echo "=== arm SUP (28,31 SUPERSEDE, n=3) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329sup64 1 KAYFABE_BENCH_BW=4,64 KAYFABE_JOIN_RELEASE=supersede
echo "=== arm SUP64 (4,64 SUPERSEDE) done $(date -Is)"

echo "=== W329 FOLLOWUP TERMINATOR rc=0 $(date -Is)"
