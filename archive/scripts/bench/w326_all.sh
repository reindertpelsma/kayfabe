#!/usr/bin/env bash
# ★★★★★ w326 — every arm, IN SEQUENCE, ONE LAUNCHER.
#
# ⚠ ONE launcher on purpose, and it is the fourth rung to say so. `[measured w317]` two
#   detached batches were launched for the same work; they `pkill`ed each other's QEMU and
#   emitted a full log of `⊘UNMEASURED` rows in 50 s, and only the TIMESTAMPS revealed it.
#   GPU runs are STRICTLY SERIAL, so the serialization lives here.
#
# ⚠ `build_qom_shim.sh` REFUSES an archive more than 30 minutes old when cargo has nothing
#   to rebuild (`BUILD RC=1` / `rc=92`), so a sweep longer than 30 min on an unchanged tree
#   fails its LATER boots as a build refusal that looks nothing like a workload problem.
#   ⇒ `touch` a crate source between arms. It is one line and it has cost two rungs.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOTS
#
#   R  reclaim tick ON,  cup3, n=3   PREDICT ^CUP3_VAL=43 3/3, Xid=0, 40 unserviced ids,
#                                    RECLAIM-TICK working_ticks>0 AND worker_disposed>0
#                                    (⊘ if worker_disposed=0 the arm is VACUOUS — it proves
#                                    the thread ran, NOT that it drained anything), and
#                                    vcpu_skipped>0 (the gate is reached at all).
#   O  reclaim tick OFF, cup3, n=1   PREDICT master's numbers on the SAME BINARY —
#                                    RECLAIM-TICK armed=false working_ticks=0 vcpu_skipped=0,
#                                    and MMUINVAL identical in SHAPE to arm R's.
#   E  reclaim tick ON,  cup8, n=1   PREDICT ^CUP8_BAD=0 ^CUP8_MAXERR=0 — the only oracle
#                                    that fails QUIETLY-WRONG rather than loudly-absent.
#   X  reclaim tick ON,  R33,  n=1   PREDICT the arm-1 COPY line. A DIFFERENT mapping path.
#                                    ⚠ w321's R33 arm was VACUOUS for its fix (`asked=0`);
#                                    this one is graded on `worker_disposed` for the same
#                                    reason — a workload that retires nothing tests nothing.
set -uo pipefail
TREE=${1:-/workspace/kayfabe_w326}
A="$TREE/scripts/bench/w326_arm.sh"
echo "=== W326 ALL START $(date -Is) tree=$TREE"

bash "$A" "$TREE" w326r 3 KAYFABE_RECLAIM_TICK=on
echo "=== arm R (reclaim tick ON, cup3, n=3) done $(date -Is)"
touch "$TREE/crates/kayfabe-util/src/lib.rs"

bash "$A" "$TREE" w326o 1
echo "=== arm O (control, tick OFF, cup3) done $(date -Is)"
touch "$TREE/crates/kayfabe-util/src/lib.rs"

W326_WORKLOAD="w308_cup8.sh cup8" bash "$A" "$TREE" w326e 1 KAYFABE_RECLAIM_TICK=on
echo "=== arm E (cup8) done $(date -Is)"
touch "$TREE/crates/kayfabe-util/src/lib.rs"

W326_WORKLOAD="w309_crit1.sh fresh" bash "$A" "$TREE" w326x 1 KAYFABE_RECLAIM_TICK=on
echo "=== arm X (R33 arm 1) done $(date -Is)"

echo "=== W326 ALL TERMINATOR rc=0 $(date -Is)"
