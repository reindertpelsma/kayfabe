#!/usr/bin/env bash
# ★★★★★ w328 — every arm, IN SEQUENCE, ONE LAUNCHER.
#
# ⚠ ONE launcher on purpose, and it is the fifth rung to say so. `[measured w317]` two detached
#   batches were launched for the same work; they `pkill`ed each other's QEMU and emitted a full
#   log of `⊘UNMEASURED` rows in 50 s, and only the TIMESTAMPS revealed it. GPU runs are
#   STRICTLY SERIAL, so the serialization lives here.
#
# ⚠ `build_qom_shim.sh` REFUSES an archive more than 30 minutes old when cargo has nothing to
#   rebuild, so a sweep longer than 30 min on an unchanged tree fails its LATER boots as a
#   BUILD refusal that looks nothing like a workload problem. ⇒ `touch` a crate source between
#   arms. It is one line and it has cost two rungs.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOTS — every outcome, so none reads as the favourable one
#
# ### ⊘⊘ THE BRIEF'S PREMISE IS THE FIRST THING UNDER TEST, AND I EXPECT IT TO FAIL
#
# The brief's (A) is *"the sweep's breadth is vestigial, scoping it drops the worst trap far
# below budget"*. Read from the source (`shim.rs:8409`), the pass that owns 95.8–97.0 % of the
# 2 879 349 µs worst trap — `measure_guest_ram_pin_rate`'s `DRAIN_MS` — is assigned ONLY inside
# `if doorbelled`, i.e. it is ALREADY scoped to the ringing channel's VAS. The unscoped breadth
# is the CENSUS pass (`publish_vas_rows`, the "229 passes / 2 529 ms" figure, CUMULATIVE over
# the boot) plus the 256-row samples of the non-doorbelled VASes. ⇒
#
#   PREDICT (arm A): `W328SCOPE breadth_share` and `W328PIN other_us / drain_ms` are SMALL —
#   the breadth is a few per cent of the worst trap, NOT 97 % of it. If `breadth_share` comes
#   back ≥ 50 % on the first doorbell, MY reading is wrong and the brief's is right; say so.
#
#   PREDICT (arm S): scoping works, `complete=true`, `CUP3_VAL=43` — and `worst_trap_us` is
#   ESSENTIALLY UNCHANGED from arm A. A scoping arm that moved the worst trap would refute the
#   paragraph above.
#
#   PREDICT (arm C): the COALESCER is what moves it. w321 measured 2.21×–34.9 % of budget on
#   this same drain and merged it DEFAULT-OFF. This is the ≥3-point margin measurement.
#
# ⚠ Anything that makes the doorbell faster makes w319's truncation RARER WITHOUT FIXING IT, so
#   a green run is NOT evidence. The margin is reported as a MULTIPLE OF BUDGET at ≥3 points.
#
#   A  scope OFF, batch OFF, cup3, n=3   THE CONTROL + THE BREADTH MEASUREMENT. Master's
#                                        behaviour on THIS binary. PREDICT ^CUP3_VAL=43,
#                                        DRAIN_MS 2700–2900, complete=true, worst_trap ≈2.8–2.9 M
#   S  scope ON,  batch OFF, cup3, n=3   PREDICT scoped=true, scoped_out>0, complete=true,
#                                        pinned==asked, ^CUP3_VAL=43, worst_trap UNCHANGED.
#                                        ⊘ A RED here is INFORMATIVE — run the attributor.
#   C  scope ON,  batch ON,  cup3, n=3   PREDICT worst_trap DOWN by the coalescing factor and
#                                        DRAIN_MS well below 3000. THE MARGIN POINTS.
#   E  scope ON,  batch ON,  cup8, n=3   PREDICT ^CUP8_BAD=0 ^CUP8_MAXERR=0 — the only oracle
#                                        that fails QUIETLY-WRONG rather than loudly-absent.
#   X  scope ON,  batch ON,  R33,  n=3   A DIFFERENT mapping path. ⚠ R33 was VACUOUS for w321
#                                        (`asked=0`) AND for w326 (`working_ticks=0`). It is
#                                        graded here on `scoped_out` AND `other_vases`: an arm
#                                        with ONE VAS scopes nothing and tests nothing. SAY SO
#                                        rather than counting it.
set -uo pipefail
TREE=${1:-/workspace/kayfabe_w328}
A="$TREE/scripts/bench/w328_arm.sh"
echo "=== W328 ALL START $(date -Is) tree=$TREE"

# ★★★ THE INSTRUMENT'S OWN SELFTEST, FIRST AND OFFLINE. It once passed 5/5 while broken on
#     every real log, so it is run BEFORE any boot rather than consulted after one.
bash "$TREE/scripts/bench/w319_attribute.sh" --selftest
echo "=== w319_attribute --selftest rc=$? $(date -Is)"

bash "$A" "$TREE" w328a 3
echo "=== arm A (control: scope off, batch off, cup3) done $(date -Is)"
touch "$TREE/crates/kayfabe-util/src/lib.rs"

bash "$A" "$TREE" w328s 3 KAYFABE_PUBLISH_SCOPE=doorbelled
echo "=== arm S (scope on, batch off, cup3) done $(date -Is)"
touch "$TREE/crates/kayfabe-util/src/lib.rs"

bash "$A" "$TREE" w328c 3 KAYFABE_PUBLISH_SCOPE=doorbelled KAYFABE_DRAIN_BATCH=coalesce
echo "=== arm C (scope on, batch on, cup3) done $(date -Is)"
touch "$TREE/crates/kayfabe-util/src/lib.rs"

W328_WORKLOAD="w308_cup8.sh cup8" bash "$A" "$TREE" w328e 3 \
  KAYFABE_PUBLISH_SCOPE=doorbelled KAYFABE_DRAIN_BATCH=coalesce
echo "=== arm E (cup8) done $(date -Is)"
touch "$TREE/crates/kayfabe-util/src/lib.rs"

W328_WORKLOAD="w309_crit1.sh fresh" bash "$A" "$TREE" w328x 3 \
  KAYFABE_PUBLISH_SCOPE=doorbelled KAYFABE_DRAIN_BATCH=coalesce
echo "=== arm X (R33 arm 1) done $(date -Is)"

echo "=== W328 ALL TERMINATOR rc=0 $(date -Is)"
