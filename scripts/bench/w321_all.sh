#!/usr/bin/env bash
# ★★★★★ w321 — every arm, IN SEQUENCE, ONE LAUNCHER.
#
# ⚠ ONE launcher on purpose, and it is the third rung to say so. `[measured w317]` two detached
#   batches were launched for the same work; they `pkill`ed each other's QEMU and emitted a full
#   log of `⊘UNMEASURED` rows in 50 s, and only the TIMESTAMPS (7 s apart, against ~2.5 min per
#   boot) revealed it. GPU runs are STRICTLY SERIAL, so the serialization lives here rather than
#   in six hopeful invocations.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOTS — the grade is the DRAIN'S STATE, not the outcome
#
#   ⚠ **A faster drain makes the w319 truncation rarer whether or not it was fixed**, because
#   the defect is COST-DRIVEN. A green run is therefore NOT evidence. The property is:
#       `complete=true` AND `pinned == asked`   (both in ROWS — see `drain_pinned`'s comment)
#   and the defect's signature is `complete=false` with the fault ABOVE `last_pinned_va`.
#
#   C  coalesce, cup3, n=3         PREDICT complete=true 3/3, DRAIN_MS well under 3000,
#                                          rows_per_chain ≈ 11, ^CUP3_VAL=43 3/3
#   X  coalesce + ROW_LIMIT=11800  PREDICT 3/3 RED, `last_pinned_va` unchanged from w319's
#      n=3                                 `0x203217000`. ★ THE INSTRUMENT MUST STILL WORK:
#                                          the row limit is applied to `vas_guest_ram_rows`
#                                          BEFORE the coalescer sees a row, so the reproducer
#                                          is preserved BY CONSTRUCTION at the SAME 11 800.
#                                          A fix that broke it would have broken the
#                                          instrument, not proved itself.
#   E  cup8, coalesce, n=3         PREDICT ^CUP8_BAD=0 ^CUP8_MAXERR=0 3/3 — the only oracle
#                                          that fails QUIETLY-WRONG rather than loudly-absent.
#   R  R33 arm 1, coalesce, n=3    PREDICT the arm-1 COPY line, 3/3. A DIFFERENT mapping path;
#                                          `relaxation_inert_gate.sh` exists because a
#                                          single-workload grade let a regression in.
#   O  off-arm control, cup3, n=1  PREDICT master's numbers on the SAME BINARY — the only
#                                          thing separating C from O is one word in the
#                                          environment.
set -uo pipefail
TREE=${1:-/workspace/kayfabe_w321}
A="$TREE/scripts/bench/w321_arm.sh"
echo "=== W321 ALL START $(date -Is) tree=$TREE HEAD=$(cd "$TREE" && git rev-parse --short HEAD)"

bash "$A" "$TREE" w321c 3 KAYFABE_DRAIN_BATCH=coalesce
echo "=== arm C (coalesce/cup3) done $(date -Is)"
bash "$A" "$TREE" w321x 3 KAYFABE_DRAIN_BATCH=coalesce KAYFABE_VAS_DRAIN_ROW_LIMIT=11800
echo "=== arm X (reproducer under the fix) done $(date -Is)"
W321_WORKLOAD=w308_cup8.sh bash "$A" "$TREE" w321e 3 KAYFABE_DRAIN_BATCH=coalesce
echo "=== arm E (cup8) done $(date -Is)"
W321_WORKLOAD="w309_crit1.sh fresh" bash "$A" "$TREE" w321r 3 KAYFABE_DRAIN_BATCH=coalesce
echo "=== arm R (R33 arm 1) done $(date -Is)"
bash "$A" "$TREE" w321o 1
echo "=== arm O (off control) done $(date -Is)"

echo "=== W321 ALL TERMINATOR rc=0 $(date -Is)"
