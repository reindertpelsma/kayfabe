#!/usr/bin/env bash
# ★★★★★ w329 — every arm, IN SEQUENCE, ONE LAUNCHER.
#
# ⚠ ONE launcher on purpose, and it is the fifth rung to say so. `[measured w317, and again
#   twice on 2026-08-14]` two detached batches launched for the same work `pkill` each other's
#   QEMU and emit a full log of `⊘UNMEASURED` rows in 50 s, and only the TIMESTAMPS reveal it.
#   GPU runs are STRICTLY SERIAL, so the serialization lives here.
#
# ⚠ `build_qom_shim.sh` REFUSES an archive more than 30 minutes old when cargo has nothing to
#   rebuild, so a sweep longer than 30 min on an unchanged tree fails its LATER boots as a
#   *build* refusal that looks nothing like a workload problem. ⇒ `touch` between arms.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOTS — every outcome, so none reads as the favourable one
#
#   A  fix ON,  KAYFABE_BENCH_BW=28,31, n=3
#      PREDICT  last_ok=31 first_fail=NONE 3/3, and JOINTRAJ `falls>0`.
#      ⊘ A green `28,31` with `falls=0` is NOT the target — it means the failure was masked
#        (outcome C), and only the trajectory separates the two.
#   C  fix OFF, KAYFABE_BENCH_BW=28,31, n=3   — ★ THE SAME-BINARY NEGATIVE CONTROL.
#      PREDICT  last_ok=28 first_fail=31 3/3, `falls=0`, and `rc=0/719` at byte 0x800000.
#      ⊘ If this PASSES, the fix is not what made arm A pass and every A row is unattributable.
#   B  fix ON,  KAYFABE_BENCH_BW=4,64, n=1    — w327u4b's row must not regress.
#   N  fix ON,  BENCH_NOLAUNCH=1, n=1         — ★ THE KNOWN-POSITIVE. `BENCH_MODE=NOLAUNCH`
#      must be PRESENT and `BENCH_NOLAUNCH_TOTAL_BAD` > 0, or every `bad=0` here is VACUOUS.
#   G  fix ON,  cup3, n=3                     — `^CUP3_VAL=43`
#   E  fix ON,  cup8, n=2                     — `^CUP8_BAD=0 ^CUP8_MAXERR=0`
#   S  fix ON,  cup8 at N=3072 (36 MiB), n=1  — w327big's row; a regression there is ours.
#   X  fix ON,  R33 arm 1, n=2                — a DIFFERENT mapping path. ⚠ State its liveness
#      from the arm-1 COPY line, never by counting it as run.
set -uo pipefail
TREE=${1:-/workspace/kayfabe_w329}
A="$TREE/scripts/bench/w329_arm.sh"
T() { touch "$TREE/crates/kayfabe-util/src/lib.rs"; }
echo "=== W329 ALL START $(date -Is) tree=$TREE HEAD=$(cd "$TREE" && git rev-parse --short HEAD)"

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329a 3 KAYFABE_BENCH_BW=28,31
echo "=== arm A (28,31 FIX ON, n=3) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329c 3 KAYFABE_BENCH_BW=28,31 KAYFABE_JOIN_RELEASE=off
echo "=== arm C (28,31 NEGATIVE CONTROL, n=3) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh bw" bash "$A" "$TREE" w329b 1 KAYFABE_BENCH_BW=4,64
echo "=== arm B (4,64 must still pass) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh bwneg" bash "$A" "$TREE" w329n 1
echo "=== arm N (known-positive, NOLAUNCH) done $(date -Is)"; T

bash "$A" "$TREE" w329g 3
echo "=== arm G (cup3, n=3) done $(date -Is)"; T

W329_WORKLOAD="w308_cup8.sh cup8" bash "$A" "$TREE" w329e 2
echo "=== arm E (cup8, n=2) done $(date -Is)"; T

W329_WORKLOAD="w322_operands.sh sizes" bash "$A" "$TREE" w329s 1 KAYFABE_BENCH_SIZES=3072 KAYFABE_BENCH_ITERS=3
echo "=== arm S (cup8 at N=3072, 36 MiB operands) done $(date -Is)"; T

W329_WORKLOAD="w309_crit1.sh fresh" bash "$A" "$TREE" w329x 2
echo "=== arm X (R33 arm 1, n=2) done $(date -Is)"

echo "=== W329 ALL TERMINATOR rc=0 $(date -Is)"
