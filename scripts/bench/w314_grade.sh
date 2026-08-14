#!/usr/bin/env bash
# ★★★★★ w314 — GRADE THE w310 PIN RELEASE AGAINST ITS OWN PRE-REGISTERED CRITERIA A–G.
#
#   usage: w314_grade.sh <tag>            # e.g. w314cup3   (reads /workspace/bench/run_<tag>_*)
#   exit:  0 = all criteria pass · 1 = a criterion FAILED · 2 = UNMEASURED
#
# The criteria are `docs/design/guest_ram_pin_release.md` §7, verbatim, run as written. This
# file exists so that the reading is a program rather than a paragraph, and so that the two
# outcomes this tree keeps confusing stay separate words:
#
#   ⊘⊘ **ABSENT ≠ ZERO.** Criterion C's own text says it: the `PIN-RELEASE` line prints only
#      on CHANGE, so a boot whose CUDA process never exits can legitimately never print it.
#      That is `UNMEASURED`, it is not `released=0`, and it is not a pass. Every clause below
#      that can be absent says which of the two it is.
#
#   ⊘ **A TRUNCATED ARTEFACT READS AS PRESENT.** Sizes are printed for every input, and a
#      log with no terminator grades as UNMEASURED rather than as a measured zero.
#
#   ⚠ **`host_rows` IS PRINTED AND NEVER GRADED** — the old criterion graded it as an exact
#      value and failed on a correct result (w298, `^CUP3_VAL=43` with zero HOST-PUBLISHED
#      lines). Criterion B delegates to `regression_check_e.sh`, which encodes that lesson.
set -uo pipefail

TAG=${1:-}
[ -n "$TAG" ] || { echo "usage: $0 <tag>" >&2; exit 64; }
SELFDIR=$(cd "$(dirname "$0")" && pwd)
# ⊘ Overridable ONLY so this grader has a known-positive: `w314_grade_selftest.sh` drives it
#   over crafted fixtures with no GPU and no boot. A criterion nobody has watched fail is a
#   wish. Defaulted to the bench path, so every real caller is unchanged.
B=${W314_BENCH:-/workspace/bench}
Q=$B/run_${TAG}_qemu.log
P=$B/run_${TAG}_probe.log
D=$B/run_${TAG}_hostdmesg.log
G=$B/run_${TAG}_dmesg.log
S=$B/run_${TAG}_serial.log

VERDICT=0
fail() { VERDICT=1; }
unmeas() { [ "$VERDICT" -eq 1 ] || VERDICT=2; }
sz() { stat -c%s "$1" 2>/dev/null || echo MISSING; }

echo "================================================================================"
echo "=== ★★★★★ w314 — CONFIRMING w310's PIN RELEASE. Criteria A–G, as w310 wrote them."
echo "===     tag=$TAG   $(date -Is)"
echo "================================================================================"
echo "    inputs, with sizes — ⊘ a truncated artefact reads as PRESENT and looks healthier"
echo "    than an empty one, so the size is part of the evidence:"
echo "      qemu     = $Q  [$(sz "$Q") bytes]"
echo "      probe    = $P  [$(sz "$P") bytes]"
echo "      hostdmsg = $D  [$(sz "$D") bytes]   (a per-boot DELTA: 0 bytes is the normal green)"
echo "      guestdmg = $G  [$(sz "$G") bytes]"
echo "      serial   = $S  [$(sz "$S") bytes]"
echo ""

# ---------------------------------------------------------------- (A) ^CUP3_VAL=43
echo "=== (A) ^CUP3_VAL=43 — first compute must survive the release"
VAL=$(grep -oE '^CUP3_VAL=[0-9]+' "$P" 2>/dev/null | tail -1)
RC=$(grep -oE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
echo "    ^ANCHORED CUP3_VAL = [${VAL:-⊘ NO LINE BEGINS WITH CUP3_VAL= — UNMEASURED, ⊘ NOT 0}]"
echo "    ^ANCHORED CUP3_RC  = [${RC:-⊘ NO TERMINATOR}]"
echo "    UNANCHORED, for contrast = [$(grep -oh 'CUP3_VAL=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
if [ "$VAL" = "CUP3_VAL=43" ]; then
  echo "    (A) ✔ PASS — 43 is un-forgeable; the host GR engine ran the guest's shader."
elif [ -z "$VAL" ]; then
  echo "    (A) ⊘ UNMEASURED — the value line never printed. ⊘ NOT a failure value, NOT 0."; unmeas
else
  echo "    (A) ✘ FAIL — $VAL. The release is not safe as built (pre-registration D)."; fail
fi
echo ""

# ---------------------------------------------------------------- (B) regression_check_e
echo "=== (B) scripts/bench/regression_check_e.sh — Xid=0, drain ran and pinned (a FLOOR),"
echo "===     MUST-be-0 invariants. ⊘ host_rows printed, NEVER graded."
"$SELFDIR/regression_check_e.sh" "$Q" "$D"
ERC=$?
case "$ERC" in
  0) echo "    (B) exit=0 ✔ PASS" ;;
  2) echo "    (B) exit=2 ⊘ UNMEASURED — ⊘ 2 IS NOT A PASS."; unmeas ;;
  *) echo "    (B) exit=$ERC ✘ REGRESSION"; fail ;;
esac
echo ""

# ---------------------------------------------------------------- (C) PIN-RELEASE released>0
echo "=== (C) ★ kayfabe: PIN-RELEASE released=N with N > 0 — THE NON-VACUITY."
echo "===     ⊘ ABSENT ≠ ZERO. w310's own text: the line prints only on CHANGE, from"
echo "===     Spine::pin_reclaim_gone, so a boot whose CUDA proc never exits legitimately"
echo "===     never prints it — UNMEASURED for this rung, not a pass and not a zero."
PRN=$(grep -c 'kayfabe: PIN-RELEASE' "$Q" 2>/dev/null || true)
echo "    PIN-RELEASE lines = [$PRN]"
grep -o 'kayfabe: PIN-RELEASE .*' "$Q" 2>/dev/null | tail -4 | fold -w 190 | sed 's/^/      /'
REL=$(grep -o 'kayfabe: PIN-RELEASE released=[0-9]*' "$Q" 2>/dev/null | grep -oE '[0-9]+$' | sort -n | tail -1)
if [ "${PRN:-0}" -eq 0 ]; then
  echo "    (C) ⊘ UNMEASURED — the line is ABSENT. The release did not reach production on"
  echo "        this boot; report it as the finding (pre-registration B), never as released=0."
  unmeas
elif [ "${REL:-0}" -ge 1 ]; then
  echo "    (C) ✔ PASS — max released=$REL  (graded as a FLOOR, never as a number)"
else
  echo "    (C) ✘ FAIL — the line printed and released stayed 0."
  fail
fi
echo ""

# ---------------------------------------------------------------- (D) no soft lockup / RCU stall
echo "=== (D) guest dmesg: non-empty, contains NVRM, and carries NO soft lockup / NO RCU stall."
echo "===     ★ This is the clause-(b) INSTRUMENT for w303's unbounded BQL disposal (§5)."
GB=$(sz "$G")
if [ "$GB" = MISSING ] || [ "${GB:-0}" -eq 0 ] 2>/dev/null; then
  echo "    (D) ⊘ UNMEASURED — guest dmesg is missing or ZERO BYTES. ⊘ 'no stall in an empty"
  echo "        file' is not a measurement; a harness that writes an empty file and exits 0 is"
  echo "        worse than none, because the file's existence reads as capture."
  unmeas
else
  NVRM=$(grep -c NVRM "$G" 2>/dev/null || true)
  STALL=$(grep -ciE 'soft lockup|softlockup|BUG: soft|rcu_sched|rcu_preempt|RCU stall|self-detected stall|hung task|blocked for more than' "$G" 2>/dev/null || true)
  echo "    guest dmesg bytes = $GB   NVRM lines = $NVRM   stall-signature lines = $STALL"
  grep -inE 'soft lockup|softlockup|BUG: soft|rcu_sched|rcu_preempt|RCU stall|self-detected stall|hung task|blocked for more than' "$G" 2>/dev/null | head -8 | sed 's/^/      /'
  if [ "${NVRM:-0}" -lt 1 ]; then
    echo "    (D) ⊘ UNMEASURED — the file exists but carries NO 'NVRM'. The serial log is not"
    echo "        where the driver's output is; this file did not capture the driver."
    unmeas
  elif [ "${STALL:-0}" -eq 0 ]; then
    echo "    (D) ✔ PASS — no soft lockup, no RCU stall, driver output present."
    echo "        ⚠ SCOPED: 'bounded IN PRACTICE on this workload', NOT 'bounded'."
  else
    echo "    (D) ✘ STALL PRESENT — this is the BQL-stall signature and it is the MORE"
    echo "        VALUABLE outcome (pre-registration C): the clause-(b) violation is now"
    echo "        measured rather than argued."
    fail
  fi
fi
echo ""

# ---------------------------------------------------------------- (E) refused_no_host_vas=0
echo "=== (E) refused_no_host_vas=0 on the same line — a non-zero contradicts"
echo "===     commit_pin_guest_ram's own invariant and is a finding in its own right."
if [ "${PRN:-0}" -eq 0 ]; then
  echo "    (E) ⊘ UNMEASURED — no PIN-RELEASE line to read it from."
  unmeas
else
  RNV=$(grep -o 'refused_no_host_vas=[0-9]*' "$Q" 2>/dev/null | grep -oE '[0-9]+$' | sort -n | tail -1)
  echo "    every distinct reading = [$(grep -o 'refused_no_host_vas=[0-9]*' "$Q" 2>/dev/null | sort -u | tr '\n' ' ')]"
  if [ "${RNV:-0}" -eq 0 ]; then
    echo "    (E) ✔ PASS — max refused_no_host_vas=0"
  else
    echo "    (E) ✘ FAIL — max refused_no_host_vas=$RNV"
    fail
  fi
fi
echo ""

# ---------------------------------------------------------------- (F) no new Xid CLASSES
echo "=== (F) NO NEW Xid CLASSES vs the previous green boot. ⊘ CLASSES, not counts —"
echo "===     a_count_cannot_see_a_substitution: a count cannot see a substitution."
echo "===     Baseline: traces/w297_cup3/run_w297cup3_dmesg.log carries ZERO Xid, and w313's"
echo "===     green boots the same. The baseline class set is therefore EMPTY."
for f in "$D" "$G"; do
  [ -e "$f" ] || { echo "      ⊘ $f MISSING — that plane is UNMEASURED"; continue; }
  n=$(grep -c Xid "$f" 2>/dev/null || true)
  echo "      $(basename "$f"): Xid lines = $n"
  [ "${n:-0}" -ge 1 ] && grep -oE 'Xid \(PCI:[^)]*\): *[0-9]+|ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|ACCESS_TYPE_[A-Z_]+|FAULT_(PDE|PTE)[0-9]*' "$f" 2>/dev/null | sort | uniq -c | sed 's/^/          /'
done
# ⊘ `grep -c` prints 0 AND exits 1, so a `|| echo 0` fallback emits a SECOND zero and the
#   arithmetic below becomes a syntax error. Count with a helper that reads the output only.
xidn() { local n; n=$(grep -c Xid "$1" 2>/dev/null); echo "${n:-0}"; }
FX=$(( $(xidn "$D") + $(xidn "$G") ))
if [ ! -e "$D" ]; then
  echo "    (F) ⊘ UNMEASURED — no host dmesg delta."
  unmeas
elif [ "$FX" -eq 0 ]; then
  echo "    (F) ✔ PASS — the observed class set is EMPTY, i.e. equal to the baseline's."
else
  echo "    (F) ✘ FAIL — Xid classes present that the baseline does not carry (baseline = {})."
  fail
fi
echo ""

# ---------------------------------------------------------------- (G) REAP and PIN-RELEASE pair
echo "=== (G) REAP and PIN-RELEASE both present, or both absent — they are driven from the"
echo "===     same edge and the same teardown. One without the other = half-wired."
RPN=$(grep -c 'kayfabe: REAP reaped=' "$Q" 2>/dev/null || true)
echo "    REAP lines = [$RPN]   PIN-RELEASE lines = [$PRN]"
grep -o 'kayfabe: REAP .*' "$Q" 2>/dev/null | tail -3 | fold -w 190 | sed 's/^/      /'
if { [ "${RPN:-0}" -ge 1 ] && [ "${PRN:-0}" -ge 1 ]; } then
  echo "    (G) ✔ PASS — both present."
elif [ "${RPN:-0}" -eq 0 ] && [ "${PRN:-0}" -eq 0 ]; then
  echo "    (G) ✔ PASS by the pairing rule — both absent. ⚠ But that state makes (C)"
  echo "        UNMEASURED, which is the finding; the pairing holding does not rescue it."
else
  echo "    (G) ✘ FAIL — one without the other: the mechanism is half-wired."
  fail
fi
echo ""

# ------------------------------------------------- ★★★ THE DISPOSAL TIMING (w314's own ask)
echo "=== ★★★ THE DISPOSAL TIMING — w303's armed reap put an UNBOUNDED disposal inside"
echo "===     Regs::write, under the QEMU BQL. §5 of the design derives ~3 s by ARITHMETIC"
echo "===     (host_rows 12818 × ~240 us) against scrubberDestruct's 4 000 000 us. This is"
echo "===     the MEASUREMENT. ⊘ It is NOT graded — it is the number the next rung needs."
TN=$(grep -c 'REAP-TIMING' "$Q" 2>/dev/null || true)
echo "    REAP-TIMING lines = [$TN]"
grep -o 'kayfabe: REAP-TIMING max_reap_us=[0-9]* reaped=[0-9]*' "$Q" 2>/dev/null | tail -8 | sed 's/^/      /'
MAXUS=$(grep -o 'max_reap_us=[0-9]*' "$Q" 2>/dev/null | grep -oE '[0-9]+$' | sort -n | tail -1)
if [ "${TN:-0}" -eq 0 ]; then
  echo "    ⊘ UNMEASURED — the instrument never printed. That means reap_retired() was never"
  echo "      called on a path this build reached, NOT that the disposal took 0 us."
else
  echo "    ★ LONGEST BLOCKING DISPOSAL = ${MAXUS} us"
  if [ -n "${MAXUS:-}" ]; then
    awk -v u="$MAXUS" 'BEGIN{printf "      = %.3f ms = %.1f%% of the 4 000 ms scrubberDestruct budget\n", u/1000.0, 100.0*u/4000000.0}'
  fi
fi
echo ""

echo "=== ★★★★★ w314 VERDICT = $VERDICT   (0 ALL PASS · 1 FAILED · 2 UNMEASURED)"
echo "    ⊘ 2 IS NOT A PASS."
exit "$VERDICT"
