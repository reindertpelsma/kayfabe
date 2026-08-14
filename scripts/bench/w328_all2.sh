#!/usr/bin/env bash
# ★★★★★ w328 SWEEP 2 — THE LEVER THE FIRST SWEEP FOUND, ISOLATED.
#
# ## ⊘⊘ WHY THERE IS A SECOND SWEEP: SWEEP 1's ARM A ANSWERED THE BRIEF AND NAMED A DIFFERENT SITE
#
# `[measured w328, boot w328a1, vh, real GA106]` on the CONTROL arm (master's behaviour):
#
#   229 publication passes.  CUMULATIVE:  target_us = 2 666 358   other_us = 24 349
#                                         target_published = 66   other_published = 0
#   W328PIN over all 229:                 other_us = 0            other_pinned = 0
#   DRAIN_MS = 2960 (complete=true, 13313/13313)   worst_trap = 3 048 658 us
#
# ⇒ THE BREADTH IS 24 349 us OF 2 690 707 us — **0.90 %** — AND IT PUBLISHES **ZERO** ALL BOOT.
#   The brief's (A) — "the sweep's breadth is vestigial, scoping it drops the worst trap far
#   below budget" — is HALF RIGHT AND HALF REFUTED: vestigial in YIELD, negligible in COST.
#   99.1 % of the 2 529–2 690 ms is the DOORBELLED VAS's OWN census, re-run 229 times.
#
# ## ★★★★★ AND THE REASON IT IS RE-RUN 229 TIMES IS NOT THE WORKLOAD — IT IS `gate=off`
#
# `the_publish_trigger_measured.md:201-203` reads the same `fired=4 skipped=0` and attributes it
# to the workload: *"so `w318`'s dirty gate skipped nothing on this workload"*. The boot's own
# line says otherwise:
#
#     ... arm=drain W328SCOPE[...] gate=off this_doorbell[fired=4 skipped=0] ...
#     DIRTY-GATE publish[fired=912 skipped=0 0.0% skipped] witness[fired=229 skipped=0 ...]
#
# ⇒ **`gate=off`. `KAYFABE_DIRTY_GATE_PUBLISH` ships DISARMED** (`shim.rs:14261`, `None => false`),
#   deliberately and for a stated reason. The gate skipped nothing because IT WAS NEVER ASKED,
#   not because the epoch kept moving. ⊘ A disarmed gate and a gate that never fires produce the
#   SAME COUNTER, and only the arm word tells them apart — this tree's own
#   `a_census_zero_needs_a_known_positive` shape.
#
# ## ★★★ PRE-REGISTERED, BEFORE THESE BOOTS
#
# ⊘⊘ AND THE FIRST THING I DID WAS CHECK WHETHER THE QUESTION IS ALREADY ANSWERED — IT IS,
#    HALF OF IT, AND THE CHECK CHANGED THIS SWEEP. `w318_the_dirty_gate.md:127` already
#    measured the gate ARMED against its own control, one variable, twelve matched doorbells:
#
#        segment            ms/launch (gates off)   ms/launch (GATED)   ratio
#        vas_publish              45.849                 0.201          228x
#
#    ⇒ A "gate ON only" arm here would RE-DERIVE w318 at the cost of three boots. It is cited
#      instead, and sweep 2 spends its boots on what w318 did NOT measure: whether the gate
#      COMPOSES with the scope and the coalescer, and what it does to `worst_trap_us`, which
#      w318 never reported. ★ Together with sweep 1's arms the ladder adds ONE LEVER PER ARM:
#      A(none) → S(scope) → C(scope+batch) → G(scope+batch+gate).
#
#   G  gate + scope + batch,  cup3, n=3   EVERYTHING THIS RUNG CAN TURN ON. PREDICT worst_trap
#                                         well under the 3 000 ms budget with margin, cumulative
#                                         publication BQL down, ^CUP3_VAL=43, complete=true.
#                                         ⊘ `skipped` MUST go non-zero; if it stays 0 with the
#                                         gate ARMED then the epoch really does move every
#                                         doorbell and w326's reading was right after all. That
#                                         is the falsifier, and it is pre-registered here.
#   GE gate + scope + batch,  cup8, n=3   The oracle that fails QUIETLY-WRONG. ^CUP8_BAD=0.
#   GX gate + scope + batch,  R33,  n=3   ⚠ R33 was VACUOUS for w321 (`asked=0`) and for w326
#                                         (`working_ticks=0`). Graded on `scoped_out`/`skipped`:
#                                         if BOTH are 0 the arm tests nothing and SAYS SO.
#
# ⚠⚠ THE GATE IS THE MORE DANGEROUS DIRECTION AND ITS OWN DOC SAYS SO: arming it makes a
#    correctness-relevant pass STOP RUNNING on a doorbell it judges clean. ⇒ graded on
#    `complete=`/`pinned==asked` and the attributor, never on `CUP3_VAL` alone, and NOT proposed
#    as a new default by this rung.
set -uo pipefail
TREE=${1:-/workspace/kayfabe_w328}
A="$TREE/scripts/bench/w328_arm.sh"
echo "=== W328 ALL2 START $(date -Is) tree=$TREE"
# ⚠ `build_qom_shim.sh` REFUSES an archive >30 min old on an unchanged tree. Sweep 1 ran for
#   ~45 min immediately before this, so the FIRST arm here would fail as a BUILD refusal that
#   looks nothing like a workload problem. One line, and it has cost two rungs.
touch "$TREE/crates/kayfabe-util/src/lib.rs"

bash "$A" "$TREE" w328g 3 KAYFABE_DIRTY_GATE_PUBLISH=on \
  KAYFABE_PUBLISH_SCOPE=doorbelled KAYFABE_DRAIN_BATCH=coalesce
echo "=== arm G (gate + scope + batch, cup3) done $(date -Is)"
touch "$TREE/crates/kayfabe-util/src/lib.rs"

W328_WORKLOAD="w308_cup8.sh cup8" bash "$A" "$TREE" w328ge 3 KAYFABE_DIRTY_GATE_PUBLISH=on \
  KAYFABE_PUBLISH_SCOPE=doorbelled KAYFABE_DRAIN_BATCH=coalesce
echo "=== arm GE (gate + scope + batch, cup8) done $(date -Is)"
touch "$TREE/crates/kayfabe-util/src/lib.rs"

W328_WORKLOAD="w309_crit1.sh fresh" bash "$A" "$TREE" w328gx 3 KAYFABE_DIRTY_GATE_PUBLISH=on \
  KAYFABE_PUBLISH_SCOPE=doorbelled KAYFABE_DRAIN_BATCH=coalesce
echo "=== arm GX (gate + scope + batch, R33 arm 1) done $(date -Is)"

echo "=== W328 ALL2 TERMINATOR rc=0 $(date -Is)"
