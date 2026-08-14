#!/usr/bin/env bash
# ★★★★★ w299 GRADER — standalone, so a run can be RE-GRADED without re-booting.
#
#   $1 = TAG   (e.g. w299concurrent, w299staggered, w299solo)
#
# ## ⊘⊘⊘ WHY THIS FILE EXISTS — A GRADING DEFECT THAT DELETED THE LINE IT WAS WRITTEN TO PRINT
#
# The first w299 run graded inside `w299_cup3x2.sh`. It printed `CUP3A_VAL=43` and **NO
# `CUP3B_VAL` LINE AT ALL** — while the probe log held `CUP3B_VAL=43` on its own line and the
# verdict logic read it correctly and printed *"(A) BOTH PROCESSES COMPUTED"*. The measurement
# was fine; **the report silently lost half of it.**
#
# **The cause, reproduced in isolation:**
#
#     X=one; Y=two
#     echo "L1 ${X:-has A's apostrophe}"
#     echo "L2 ${Y:-has B's apostrophe}"     # ← THIS LINE NEVER RUNS UNDER bash
#
# A single quote inside `${VAR:-default}` **within double quotes** opens a single-quoted region
# in bash. It runs to the NEXT apostrophe — which was on the following line, inside the *next*
# `echo`'s default — so the two `echo`s were parsed as ONE command and the second vanished.
# ⊘ POSIX leaves quoting inside `${...}` unspecified: **`dash` prints both lines, `bash` prints
# one.** The bug is invisible to `bash -n` (the syntax is legal) and leaves no error.
#
# ★★★ AND HERE IS THE PART THAT MATTERS: the apostrophes were in the text of the
# *"THE MEASUREMENT DID NOT HAPPEN"* fallbacks — the very strings written to make a missing
# value impossible to misread as a zero. ⇒ **Had B actually been UNMEASURED, this grader would
# have printed NOTHING for B — not the ⊘ warning, not a blank, nothing.** A silently absent row
# reads as "not applicable", which is the absent-artefact class this tree has paid for
# repeatedly, produced *by the guard against it*.
#
# ⇒ **RULE: no apostrophe inside any `${VAR:-...}`.** Every default in this file is checked.
set -uo pipefail
TAG="${1:-}"
[ -n "$TAG" ] || { echo "usage: $0 <tag>" >&2; exit 64; }

OUT=/workspace/${TAG}.log
Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

for f in "$Q" "$P"; do
  [ -e "$f" ] || echo "⊘ MISSING ARTEFACT: $f — every reading derived from it is UNMEASURED, not zero."
done

MODE=$(grep -oE 'MODE=(concurrent|staggered|solo)' "$P" 2>/dev/null | head -1 | cut -d= -f2)

echo ""
echo "================================================================================"
echo "=== ★★★★★ W299 GRADING — TAG=$TAG  mode=${MODE:-UNKNOWN}  $(date -Is)"
echo "================================================================================"
echo ""
echo "=== (F) ATTRIBUTION PRECONDITIONS"
echo "    JIT present       = [$(grep -oE 'CUP3X2_JIT_PRESENT=(yes|no)' "$P" 2>/dev/null | tail -1)]"
echo "    stagger achieved  = [$(grep -oE 'CUP3X2_STAGGER_ACHIEVED=[a-zA-Z-]+' "$P" 2>/dev/null | tail -1)]"
echo "    cup3.c md5 match  = [$(grep -cE 'md5 MATCHES w297' "$P" 2>/dev/null)]  (1 = the same program w297 ran)"
echo "    ⊘ on mode=staggered, 'no-A-already-past' means B did NOT start inside the cuCtxCreate"
echo "      of A — the arm did not land, and a green there does not answer the #14 shape."
echo ""
echo "=== ★★★★★ THE METRICS — ^ANCHORED and SEPARATELY IDENTIFIABLE PER PROCESS"
echo "===     ⊘ a grader that cannot tell A from B is the substitution failure itself."
for N in A B; do
  V=$(grep -oE "^CUP3${N}_VAL=[0-9]+" "$P" 2>/dev/null | tail -1)
  R=$(grep -oE "^CUP3${N}_RC=[0-9]+" "$P" 2>/dev/null | tail -1)
  # ⊘ NO APOSTROPHES IN THESE DEFAULTS. See this file's header for what one cost.
  echo "--- ★★★★★ ${V:-⊘ NO LINE BEGINS WITH CUP3${N}_VAL= — THE MEASUREMENT FOR ${N} DID NOT HAPPEN. ⊘ NOT 0, NOT a failure value.}"
  echo "--- ★★★★  ${R:-⊘ NO LINE BEGINS WITH CUP3${N}_RC= — no terminator was written for ${N}.}"
done
echo "    kernel lines, verbatim:"
grep -hE '^CUP3[AB]_KERNEL_LINE=' "$P" 2>/dev/null | sed 's/^/      /'
echo "    UNANCHORED, for contrast = [$(grep -oh 'CUP3[AB]_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
echo "    ⊘ a bare CUP3_VAL= is deliberately NEVER emitted by this rung."
echo ""
echo "=== ★★ THE GRADER CHECKS ITSELF — both rows must be PRESENT, whatever they say"
NROWS=$(grep -c '^--- ★★★★★ ' /dev/stdin <<<"$(for N in A B; do
  V=$(grep -oE "^CUP3${N}_VAL=[0-9]+" "$P" 2>/dev/null | tail -1); echo "--- ★★★★★ ${V:-MISSING}"; done)")
echo "    VAL rows emitted = [$NROWS]  (MUST be 2 — 1 means this grader dropped a row again)"

AV=$(grep -oE '^CUP3A_VAL=[0-9]+' "$P" 2>/dev/null | tail -1); A=${AV#CUP3A_VAL=}
BV=$(grep -oE '^CUP3B_VAL=[0-9]+' "$P" 2>/dev/null | tail -1); B=${BV#CUP3B_VAL=}
CTXA=$(grep -c '^    ✔ A CTX OK' "$P" 2>/dev/null)
CTXB=$(grep -c '^    ✔ B CTX OK' "$P" 2>/dev/null)
SDP=$(grep -c 'SystemDataPlane' "$Q" 2>/dev/null)

echo ""
echo "=== ★★★★★ THE VERDICT, stated once, in the pre-registered vocabulary"
if [ "$MODE" = solo ]; then
  echo "    ⊘ mode=solo is the BEACON CONTROL, not an arm of the question. A=[${A:-UNMEASURED}]"
  echo "      Its only job is to give the beacon a ONE-PROCESS baseline to be compared against."
elif [ "$A" = 43 ] && [ "$B" = 43 ]; then
  echo "    (A) ★★★★★ BOTH PROCESSES COMPUTED. 43 twice, from two separately-identified processes."
  echo "        ⊘ ONE SHAPE ONLY: two identical short workloads. NOT a multi-tenancy claim."
elif { [ "$A" = 43 ] && [ "$B" != 43 ]; } || { [ "$B" = 43 ] && [ "$A" != 43 ]; }; then
  W=B; [ "$B" = 43 ] && W=A
  echo "    (B) ★★★ ONE CROSSED, ONE DID NOT. The one that did NOT is [$W]."
  echo "        ⊘ THIS IS NOT 'mostly working'. Its wait is named in the DIAG block below."
  if [ "$W" = B ] && [ "${CTXB:-0}" = 0 ]; then
    echo "    (D) ★★★★★ AND B NEVER REACHED CTX OK — it failed at cuCtxCreate/allocation."
    echo "        ⇒ THIS IS THE C #14 SHAPE REPRODUCING. Name the refusal."
  fi
elif [ -z "$A" ] && [ -z "$B" ]; then
  echo "    (C) ★★★★★ BOTH UNMEASURED — the second process broke the first."
  echo "        ⊘ STRONGER than (B), not weaker. Read the beacon: global freeze or two waits?"
else
  echo "    ⚠ MIXED / NON-LADDER VALUES — A=[${A:-UNMEASURED}] B=[${B:-UNMEASURED}]. Reported raw."
fi
echo "    reached CTX OK: A=[$([ "${CTXA:-0}" != 0 ] && echo yes || echo NO)] B=[$([ "${CTXB:-0}" != 0 ] && echo yes || echo NO)]"
[ "${SDP:-0}" -gt 0 ] && echo "    ⚠ (E) SystemDataPlane mentions in the qemu log = $SDP — see the refusal census below"

echo ""
echo "=== ★ THE STAGE LADDERS, BOTH PROCESSES"
grep -E '^    [✔✘] [AB] ' "$P" 2>/dev/null | sed 's/^/    /'
echo "=== ★ each process own FAIL line, if it named its failure"
grep -hE '^ *FAIL ' "$P" 2>/dev/null | sed 's/^/    /'

echo ""
echo "=== ★★★★★ THE BEACON — GLOBAL FREEZE (BQL) vs PER-PROCESS WAIT"
echo "===     ⊘ this is the whole diagnostic value of the rung; a timeout cannot separate them."
sed -n '/beacon samples captured/,/HOW TO READ THIS/p' "$P" 2>/dev/null | sed 's/^/    /'
echo "    ★ RAW GAP HISTOGRAM (so the comparison across arms is on numbers, not on prose):"
grep -E '^ *GAP ' "$P" 2>/dev/null | sed 's/^/      /'
echo "    max gap    = [$(grep -oE 'max inter-sample gap = [0-9.]+ s' "$P" 2>/dev/null | tail -1)]"
echo "    gaps > 1 s = [$(grep -oE 'gaps > 1.0 s *= [0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "    span       = [$(grep -oE 'beacon span *= [0-9.]+ s' "$P" 2>/dev/null | tail -1)]"

echo ""
echo "=== ★★ THE LIVE DIAGNOSTIC — what the stalled process was waiting on"
sed -n '/ONE PROCESS TERMINATED AND THE OTHER DID NOT/,/beacon state AT THIS MOMENT/p' "$P" 2>/dev/null | head -80 | sed 's/^/    /'
echo "    DIAG blocks = [$(grep -c 'DIAG for cup3' "$P" 2>/dev/null)]  (0 ⇒ never fired: both finished, or both hung)"
echo ""
echo "=== ★★ DIRECT PER-PROCESS LIVENESS (⊘ never inferred from an empty file)"
grep -hE 'CUP3[AB]_WRAPPER=|CUP3[AB]_OUT_BYTES=|LIVE_CUP3_PIDS=' "$P" 2>/dev/null | sed 's/^/    /'

echo ""
echo "=== ★★ GUEST dmesg — soft-lockup / RCU stall / hung task (STRONG evidence of a BQL stall)"
grep -iE 'soft lockup|softlockup|rcu.*stall|hung task|blocked for more than|watchdog:' "$P" 2>/dev/null | head -20 | sed 's/^/    /'
echo "    matches = [$(grep -icE 'soft lockup|softlockup|rcu.*stall|hung task|blocked for more than' "$P" 2>/dev/null)]"

echo ""
echo "=== (E) REGRESSION CHECK — the single-process control is Xid=0, host_rows 18295/18309"
if [ -e "$D" ]; then
  echo "    Xid count = [$(grep -c Xid "$D")]   (host dmesg DELTA file exists, $(stat -c%s "$D") bytes)"
  grep -E 'Xid' "$D" 2>/dev/null | head -6 | sed 's/^/      /'
else
  echo "    Xid count = [⊘ NO HOST DMESG FILE AT ALL — UNMEASURED. ⊘ NOT zero.]"
fi
echo "    final host_rows readings:"
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | tail -6 | sed 's/^/      /'

echo ""
echo "=== ★★★ THE MULTI-PROCESS CENSUS — did the device SEE two processes?"
echo "===     ⊘ if these read as one, every per-process claim is unattributable."
echo "    distinct proc= = [$(grep -oE 'proc=[0-9]+' "$Q" 2>/dev/null | sort -u | tr '\n' ' ')]"
echo "    distinct pdb=  = [$(grep -oE 'pdb=0x[0-9a-f]+' "$Q" 2>/dev/null | sort -u | wc -l)]"
echo "    isolate census = [$(grep -oE 'isolates: [0-9]+ materialized, [0-9]+ live, [0-9]+ refusing' "$Q" 2>/dev/null | tail -1)]"
echo "      ⊘ the single-process control showed 2 materialized. A second CUDA proc should make 3."
echo "    isolate ids seen = [$(grep -oE 'iso[0-9]+/gpu[0-9]+' "$Q" 2>/dev/null | sort -u | tr '\n' ' ')]"
echo "    GR-BIRTH lines, by isolate:"
grep -oE 'GR-BIRTH iso[0-9]+/gpu[0-9]+ #[0-9]+ engine=[A-Za-z]+' "$Q" 2>/dev/null | sort | uniq -c | sed 's/^/      /'
echo "    CE-SYSPROC-KEPT = [$(grep -c 'CE-SYSPROC-KEPT' "$Q" 2>/dev/null)]  (control: 202)"
echo "    LateMerge       = [$(grep -c 'LateMerge' "$Q" 2>/dev/null)]  (MUST be 0 — two procs merging is the #14 aliasing bug)"

echo ""
echo "=== (E) NAMED REFUSALS — the control set is 2 AllocClassNotPermitted + 1 ReservedClient + 1 UnmappedAllocClass"
echo "    every FwdFault variant:"
grep -oE 'FwdFault::[A-Za-z]+' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -20 | sed 's/^/      /'
echo "    ⊘REFUSED, by name:"
grep -o '⊘REFUSED `[^`]*`' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -20 | sed 's/^/      /'
echo "    other refusal names:"
grep -oE 'AllocClassNotPermitted|ReservedClient|UnmappedAllocClass|SystemDataPlane|WindowExhausted|ClientKindRuleUnknown' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | sed 's/^/      /'

echo ""
echo "=== ★ THE UNSERVICED LEDGER — the single-process cup3 baseline was 2 distinct ids"
grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -30 | sed 's/^/      /'
echo "      distinct ids = [$(grep -ho 'unserviced fn 76 cmd 0x[0-9a-f]*' "$Q" 2>/dev/null | sort -u | wc -l)]"
echo "    ⊘ an id here and NOT in the single-process baseline is a MULTI-PROCESS-specific demand."

echo ""
echo "=== ★ DOORBELLS BY ENGINE — two processes should show MORE work, not the same"
grep -o 'by engine: .*' "$Q" 2>/dev/null | tail -2 | sed 's/^/      /'
echo "      per-doorbell routing tally:"
grep -ho 'engine=[A-Za-z]*' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | sed 's/^/        /'

echo ""
echo "=== ⊘ EVERY RELAXATION THAT WAS ON — a relaxed green is a MAP, not the milestone"
for v in KAYFABE_PT_SWEEP KAYFABE_OPERAND_JOIN KAYFABE_FB_JOIN KAYFABE_VAS_PUBLISH \
         KAYFABE_GR_ROUTE KAYFABE_GUEST_RING KAYFABE_GUEST_PUSHBUF KAYFABE_GUEST_SEMA \
         KAYFABE_GUEST_OPERAND KAYFABE_ISOLATES KAYFABE_CE_EXECUTOR; do
  echo "    $v = [$(grep -oE "$v=[a-z]+" "$OUT" 2>/dev/null | tail -1)]"
done

echo ""
echo "=== ★★ HARNESS SELF-CHECK"
echo "    qemu log bytes  = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)]"
echo "    probe log bytes = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W299 GRADE END TAG=$TAG at $(date -Is) ==="
