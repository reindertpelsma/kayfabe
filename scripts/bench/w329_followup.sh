#!/usr/bin/env bash
# ★★★★★ w329 FOLLOW-UP — the three questions arm B opened, in the order they must be asked.
#
# ⊘⊘ Arm B measured a REGRESSION: `4,64` PASSED for w327 (`w327u4`, `w327u4b`) and FAILS with
#    leg 1 armed (`w329b1`, `revoked=4 still_desired=1`). Two readings, and only a control
#    separates them:
#      (i)  the release revoked a translation the 64 MiB row still needed  ⇒ OURS;
#      (ii) master drifted between `df3043be` and `d859beb1`               ⇒ NOT OURS.
#
#   BC   `4,64` with KAYFABE_JOIN_RELEASE=off, n=2  — ★ THE SAME-BINARY CONTROL. It is asked
#        FIRST because every other number here is unreadable until it is answered.
#   SUP  `28,31` with KAYFABE_JOIN_RELEASE=supersede, n=3 — does leg 2 close what leg 1 could
#        not? PREDICT `already_joined` refusals fall from 21 toward 0, and `last_ok=31`.
#   SUP64 `4,64` with supersede, n=1 — leg 2 must not inherit leg 1's regression.
set -uo pipefail
TREE=${1:-/workspace/kayfabe_w329}
A="$TREE/scripts/bench/w329_arm.sh"
T() { touch "$TREE/crates/kayfabe-util/src/lib.rs"; }
echo "=== W329 FOLLOWUP START $(date -Is) tree=$TREE HEAD=$(cd "$TREE" && git rev-parse --short HEAD)"

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329bc 2 KAYFABE_BENCH_BW=4,64 KAYFABE_JOIN_RELEASE=off
echo "=== arm BC (4,64 CONTROL, release OFF, n=2) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329sup 3 KAYFABE_BENCH_BW=28,31 KAYFABE_JOIN_RELEASE=supersede
echo "=== arm SUP (28,31 SUPERSEDE, n=3) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329sup64 1 KAYFABE_BENCH_BW=4,64 KAYFABE_JOIN_RELEASE=supersede
echo "=== arm SUP64 (4,64 SUPERSEDE) done $(date -Is)"

echo "=== W329 FOLLOWUP TERMINATOR rc=0 $(date -Is)"
