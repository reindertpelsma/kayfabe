#!/usr/bin/env bash
# ★★★★★ w299 — TWO CONCURRENT CUDA PROCESSES ON THE COMPUTE PLANE.
#
#   $1 = concurrent | staggered   (⊘ REQUIRED — never defaulted. A defaulted arm makes an
#                                  evidence run and its own control indistinguishable at the
#                                  call site, which is the shape w290p already names.)
#
# `cup3` crossed at `^CUP3_VAL=43` (w297, master `91f8b34b`) — FIRST COMPUTE, ONE process, ONE
# context. This rung asks the next question and no other: **does it survive a second concurrent
# process?** That is the `#14` shape from the C era (two CUDA apps hang at `cuCtxCreate`),
# explicitly deferred to this Rust rewrite, never tested at the compute plane.
#
# ## ⊘ THE ARMING DOES NOT MOVE — byte for byte w297's, which is byte for byte w294's
#
# `w290p_run.sh drain` supplies all eleven. Nothing is added, nothing relaxed further. The ONLY
# variables are the PROCESS COUNT and the START OFFSET. Changing arming and process count
# together would make any outcome unattributable.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none reads as the favourable one
#
#   (A) BOTH `43` ⇒ multi-process compute works FOR THIS SHAPE. ⊘ Two identical short
#       workloads is ONE SHAPE — it is NOT a multi-tenancy claim. Report every relaxation.
#   (B) one `43`, one hang/timeout ⇒ name WHICH, and name THE WAIT. ⊘ This must NOT read as
#       "mostly working".
#   (C) BOTH hang ⇒ the second process broke the first. STRONGER than (B), not weaker.
#   (D) the second process fails at ALLOCATION / `cuCtxCreate` rather than at compute ⇒ the
#       C's `#14` IS REPRODUCING. Say so and name the refusal.
#   (E) `SystemDataPlane` or another named refusal blocks the setup ⇒ report and STOP. It is
#       an open owner ruling, not this rung's to route around.
#   (F) cup3 does not pass single-process on this box ⇒ THE BOX, not the code. Run
#       `w297_cup3.sh` first; if it does not print 43, everything after it is unattributable.
#
# ★★ (B), (C) and (D) are ENTIRELY HONEST OUTCOMES and this is the first look.
#    ⊘ DO NOT ITERATE TOWARD A GREEN.
#
# ## ★★★★★ THE BQL SHARPENING — what the beacon is for
#
# The doorbell path runs UNDER THE QEMU BQL (`crates/kayfabe-qemu-raw/src/shim.rs:4877`,
# `:6146`, `:6046`), and the kernel-CE completion runs synchronously inline off the doorbell
# (`kayfabe-abi/src/eventnotify.rs:191-193`). ⇒ blocking there stalls **every vCPU and QEMU's
# main loop**, not just the ringing vCPU. So the predicted symptom is NOT "B is slow while A
# runs" — it is **both stopping together and the guest freezing**. `cup3x2_hook.sh` therefore
# runs a GPU-free beacon in the guest; a GAP in it is a GLOBAL freeze (BQL), a TICKING beacon
# beside a stalled process is a PER-PROCESS wait. ⊘ A bare timeout cannot separate these.
set -uo pipefail
MODE="${1:-}"
case "$MODE" in
  concurrent|staggered) ;;
  *) echo "usage: $0 concurrent|staggered" >&2; exit 64 ;;
esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w299}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w299}
export KAYFABE_TAG=${KAYFABE_TAG:-w299$MODE}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/cup3x2_hook.sh"
export KAYFABE_CUP3X2_MODE="$MODE"
# Two processes, each bounded at 300 s, plus the beacon analysis and two ladders.
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

rm -f /workspace/bench/qemu-build/qemu-system-x86_64

"$REPO/scripts/bench/w290p_run.sh" drain
BRC=$?

OUT=/workspace/${KAYFABE_TAG}.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log

{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W299 GRADING — TWO PROCESSES, mode=$MODE  inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
echo ""
echo "=== (F) ATTRIBUTION PRECONDITIONS"
echo "    JIT present       = [$(grep -oE 'CUP3X2_JIT_PRESENT=(yes|no)' "$P" 2>/dev/null | tail -1)]"
echo "    stagger achieved  = [$(grep -oE 'CUP3X2_STAGGER_ACHIEVED=[a-zA-Z-]+' "$P" 2>/dev/null | tail -1)]"
echo "    ⊘ on mode=staggered a value of 'no-A-already-past' means B did NOT start inside A's"
echo "      cuCtxCreate — the arm did not land, and a green here does not answer the #14 shape."
echo ""
echo "=== ★★★★★ THE METRICS — ^ANCHORED and SEPARATELY IDENTIFIABLE PER PROCESS"
echo "===     ⊘ a grader that cannot tell A's value from B's is the substitution failure itself."
AV=$(grep -oE '^CUP3A_VAL=[0-9]+' "$P" 2>/dev/null | tail -1)
BV=$(grep -oE '^CUP3B_VAL=[0-9]+' "$P" 2>/dev/null | tail -1)
AR=$(grep -oE '^CUP3A_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
BR=$(grep -oE '^CUP3B_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
echo "--- ★★★★★ ${AV:-⊘ NO LINE BEGINS WITH CUP3A_VAL= — A's MEASUREMENT DID NOT HAPPEN. ⊘ NOT 0.}"
echo "--- ★★★★★ ${BV:-⊘ NO LINE BEGINS WITH CUP3B_VAL= — B's MEASUREMENT DID NOT HAPPEN. ⊘ NOT 0.}"
echo "--- ★★★★  ${AR:-⊘ NO LINE BEGINS WITH CUP3A_RC= — no terminator was written for A.}"
echo "--- ★★★★  ${BR:-⊘ NO LINE BEGINS WITH CUP3B_RC= — no terminator was written for B.}"
echo "    kernel lines, verbatim:"
grep -hE '^CUP3[AB]_KERNEL_LINE=' "$P" 2>/dev/null | sed 's/^/      /'
echo "    UNANCHORED, for contrast = [$(grep -oh 'CUP3[AB]_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
echo "    ⊘ a bare CUP3_VAL= is deliberately NEVER emitted by this rung's hook."
echo ""
echo "=== ★★★★★ THE VERDICT, stated once, in the pre-registered vocabulary"
A=${AV#CUP3A_VAL=}; B=${BV#CUP3B_VAL=}
CTXA=$(grep -c '^    ✔ A CTX OK' "$P" 2>/dev/null)
CTXB=$(grep -c '^    ✔ B CTX OK' "$P" 2>/dev/null)
SDP=$(grep -c 'SystemDataPlane' "$Q" 2>/dev/null)
if [ "$SDP" -gt 0 ]; then
  echo "    ⚠ SystemDataPlane mentions in the qemu log = $SDP — see the (E) section below"
fi
if [ "$A" = 43 ] && [ "$B" = 43 ]; then
  echo "    (A) ★★★★★ BOTH PROCESSES COMPUTED. 43 twice, from two separately-identified processes."
  echo "        ⊘ ONE SHAPE ONLY: two identical short workloads. NOT a multi-tenancy claim."
elif { [ "$A" = 43 ] && [ "$B" != 43 ]; } || { [ "$B" = 43 ] && [ "$A" != 43 ]; }; then
  W=B; [ "$B" = 43 ] && W=A
  echo "    (B) ★★★ ONE CROSSED, ONE DID NOT. The one that did NOT is [$W]."
  echo "        ⊘ THIS IS NOT 'mostly working'. Its wait is named in the DIAG block below."
  if [ "$W" = B ] && [ "${CTXB:-0}" = 0 ]; then
    echo "    (D) ★★★★★ AND B NEVER REACHED 'CTX OK' — it failed at cuCtxCreate/allocation."
    echo "        ⇒ THIS IS THE C's #14 SHAPE REPRODUCING. Name the refusal."
  fi
elif [ -z "$A" ] && [ -z "$B" ]; then
  echo "    (C) ★★★★★ BOTH UNMEASURED — the second process broke the first."
  echo "        ⊘ STRONGER than (B), not weaker. Read the beacon: global freeze or two waits?"
else
  echo "    ⚠ MIXED / NON-LADDER VALUES — A=[${A:-UNMEASURED}] B=[${B:-UNMEASURED}]. Reported raw."
fi
echo "    reached CTX OK: A=[$([ "${CTXA:-0}" != 0 ] && echo yes || echo NO)] B=[$([ "${CTXB:-0}" != 0 ] && echo yes || echo NO)]"
echo ""
echo "=== ★ THE STAGE LADDERS, BOTH PROCESSES, as the hook recorded them"
grep -E '^    [✔✘] [AB] ' "$P" 2>/dev/null | sed 's/^/    /'
echo "=== ★ each process's own FAIL line, if it named its failure"
grep -hE '^ *FAIL ' "$P" 2>/dev/null | sed 's/^/    /'
echo ""
echo "=== ★★★★★ THE BEACON VERDICT — GLOBAL FREEZE (BQL) vs PER-PROCESS WAIT"
echo "===     ⊘ this is the whole diagnostic value of the rung; a bare timeout cannot separate them."
sed -n '/THE BEACON — DID THE WHOLE GUEST STOP/,/^=== ★★ GUEST dmesg/p' "$P" 2>/dev/null | sed 's/^/    /'
echo ""
echo "=== ★★ THE LIVE DIAGNOSTIC — what the stalled process was waiting on"
echo "===     ⊘ a hang with a NAMED wait beats a hang with a timeout."
sed -n '/ONE PROCESS TERMINATED AND THE OTHER DID NOT/,/beacon state AT THIS MOMENT/p' "$P" 2>/dev/null | head -80 | sed 's/^/    /'
echo "    DIAG blocks present = [$(grep -c 'DIAG for cup3' "$P" 2>/dev/null)] (0 ⇒ never fired: either both finished or both hung)"
echo ""
echo "=== ★★ DIRECT PER-PROCESS LIVENESS (⊘ never inferred from an empty file)"
grep -hE 'CUP3[AB]_WRAPPER=|CUP3[AB]_OUT_BYTES=|LIVE_CUP3_PIDS=' "$P" 2>/dev/null | sed 's/^/    /'
echo ""
echo "=== ★★ GUEST dmesg — soft-lockup / RCU stall / hung task / NVRM / Xid"
echo "===     ★ a soft-lockup or RCU stall is STRONG evidence for a BQL-held stall."
grep -iE 'soft lockup|softlockup|rcu.*stall|hung task|blocked for more than|watchdog:' "$P" 2>/dev/null | head -20 | sed 's/^/    /'
echo "    matches = [$(grep -icE 'soft lockup|softlockup|rcu.*stall|hung task|blocked for more than' "$P" 2>/dev/null)]"
echo ""
echo "=== (E) REGRESSION CHECK — cup2/cup3's established values are Xid=0 and host_rows 18295/18309"
echo "===     ⚠ printed EVEN ON A GREEN: a pass that regressed the address plane is not a pass."
if [ -e "$D" ]; then
  echo "    Xid count = [$(grep -c Xid "$D")]   (host dmesg DELTA file exists, $(stat -c%s "$D") bytes)"
  grep -E 'Xid' "$D" 2>/dev/null | head -6 | sed 's/^/      /'
else
  echo "    Xid count = [⊘ NO HOST DMESG FILE AT ALL — UNMEASURED. ⊘ NOT zero.]"
fi
echo "    host_rows, every distinct reading:"
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
echo ""
echo "=== (E) NAMED REFUSALS — SystemDataPlane and every other FwdFault, by name"
echo "    SystemDataPlane mentions = [$SDP]"
grep -o 'SystemDataPlane[^ ]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -10 | sed 's/^/      /'
echo "    every FwdFault variant seen:"
grep -oE 'FwdFault::[A-Za-z]+' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -20 | sed 's/^/      /'
echo "    ⊘REFUSED, by name:"
grep -o '⊘REFUSED `[^`]*`' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -20 | sed 's/^/      /'
echo ""
echo "=== ★ THE UNSERVICED LEDGER — cup3's single-process baseline was 40 distinct ids"
grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -30 | sed 's/^/      /'
echo "      distinct ids = [$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)]"
echo "    ⊘ an id appearing HERE and not in the single-process baseline is a MULTI-PROCESS-"
echo "      specific demand, which is the most likely shape of a new wall."
echo ""
echo "=== ★ DOORBELLS BY ENGINE — two processes should show MORE work, not the same"
grep -o 'by engine: .*' "$Q" 2>/dev/null | tail -2 | sed 's/^/      /'
echo "      ⊘ if the line above is ABSENT the summary never printed — UNMEASURED, not zero."
echo "      per-doorbell routing tally (independent of the summary line):"
grep -ho 'engine=[A-Za-z]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | sed 's/^/        /'
echo ""
echo "=== ★★ HOW MANY DISTINCT GUEST PROCESSES DID THE DEVICE SEE?"
echo "===     ⊘ if this is 1, the two guest processes were NOT distinguished by us and every"
echo "===       per-process claim below is unattributable."
grep -oE 'proc=[0-9]+' "$Q" 2>/dev/null | sort -u | head -20 | sed 's/^/      /'
echo "      distinct proc= values = [$(grep -oE 'proc=[0-9]+' "$Q" 2>/dev/null | sort -u | wc -l)]"
grep -oE 'pdb=0x[0-9a-f]+' "$Q" 2>/dev/null | sort -u | head -20 | sed 's/^/      /'
echo "      distinct pdb= values  = [$(grep -oE 'pdb=0x[0-9a-f]+' "$Q" 2>/dev/null | sort -u | wc -l)]"
echo ""
echo "=== ⊘ EVERY RELAXATION THAT WAS ON — a relaxed green is a MAP, not the milestone"
for v in KAYFABE_PT_SWEEP KAYFABE_OPERAND_JOIN KAYFABE_FB_JOIN KAYFABE_VAS_PUBLISH \
         KAYFABE_GR_ROUTE KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF KAYFABE_GUEST_SEMA \
         KAYFABE_GUEST_OPERAND KAYFABE_ISOLATES KAYFABE_CE_EXECUTOR; do
  echo "    $v = [$(grep -oE "$v=[a-z]+" "$OUT" 2>/dev/null | tail -1)]"
done
echo ""
echo "=== ★★ HARNESS SELF-CHECK — assert THIS block's own output exists"
echo "    w299 grading lines = [$(grep -c 'W299 GRADING' "$OUT" 2>/dev/null)]  (MUST be >= 1)"
echo "    qemu log bytes     = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)]"
echo "    probe log bytes    = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W299 EXIT rc=$BRC mode=$MODE at $(date -Is) ==="
} >>"$OUT" 2>&1

exit "$BRC"
