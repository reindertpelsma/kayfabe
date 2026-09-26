#!/usr/bin/env bash
# ★★★★★ **THE TWO-WORKLOAD INERTNESS GATE — w313.**
#
#   grade  <cup3_probe_log> <r33_probe_log>      # offline, no GPU, no boot
#   run    <tag> [VAR=VALUE ...]                 # on the bench: BOTH boots, then grade
#
# ## ⊘⊘⊘ WHY THIS FILE EXISTS — a gate that was sound and still shipped a regression
#
# `[measured 2026-08-14, w304 → w309 → w313, vh2, real GA106]` w304 declared **five**
# relaxations inert and deleted 2 850 lines. Its gate was careful — n=2 per arm, one variable
# per boot, the arming read back from the **device's own emissions**, a joint boot, and
# known-positives. It graded every arm on **`^CUP3_VAL=43`** and nothing else.
#
# ⊘ **`43` is cup3: libcuda, a CUDA context, a GR compute launch.** Two of the five —
# `KAYFABE_PT_SWEEP` and `KAYFABE_OPERAND_JOIN=join` — are load-bearing for a workload cup3
# does not resemble: **`R33 arm 1`, a raw CE client with no libcuda in the process**, its own
# `FERMI_VASPACE_A`, its own operands, a copy engine and ~53 ioctls. Each ablation ALONE kills
# it (`8d258daa`, one variable per boot), and the regression bisects to the deletion merge
# `d2c58075`. ⇒ ***inert for cup3* was read as *inert*.**
#
# ⚠ And note the shape of the miss, because a paragraph would not have caught it: w304's
# stated reason for `OPERAND_JOIN` being inert **named its own scope out loud** — *"all 96
# OPERAND-JOIN-TABLE lines read `0 CANDIDATE(S)`"* — a fact about **that boot's workload**,
# read as a fact about the **mechanism**. The defect is not carelessness; it is that one
# workload's silence is indistinguishable from a mechanism's silence *when only one workload
# is ever asked*.
#
# ## ★★★ THE CONSTRUCTION, AND WHY IT IS A CHECK RATHER THAN A POLICY
#
# This tree's banked rule is that policy-shaped rules decay and construction-shaped ones do
# not. So:
#
#  - **There is no single-workload mode.** `grade` takes TWO log paths, positionally, and
#    `run` boots TWO arms. A caller cannot ask for half the gate; there is no flag for it.
#  - **A missing log is `UNMEASURED` (exit 2), never `INERT`.** An absent workload can never
#    read as a passing one — the failure this file exists to prevent.
#  - **`INERT` requires BOTH known-positives to have FIRED.** Not "no failures seen": the
#    positive line itself must be present in each log, so a truncated or empty artefact
#    (which reads as healthier than a missing one) grades as UNMEASURED.
#  - The verdict is a single anchored line, `W313 INERT-GATE VERDICT = …`, so a downstream
#    grader asserts on one literal rather than re-deriving the reading.
#
# ⚠ **What this gate does NOT do, said plainly.** Two workloads are two workloads, not a
# proof. `INERT` here means *"inert for the CE plane and the GR/compute plane as this tree
# exercises them"*. A third plane (display, NVENC, multi-process) is unmeasured and the
# verdict line says so. ⊘ Do not read it as "safe to delete" without saying which planes were
# asked.
set -uo pipefail

# ★ ONE literal, shared by the emitter and by every downstream check, so a rename cannot
#   leave a grader hunting a string nothing prints (w309 boot 1 lost a self-check that way).
VERDICT_BANNER="W313 INERT-GATE VERDICT"

# ⊘ Unanchored on purpose. Every hook in this tree indents the workload's own output when it
#   echoes it, and a `^`-anchored grep on the harness log then prints UNMEASURED beside a
#   verbatim block six lines above that contains the value. The probe log carries the raw
#   line; matching the distinctive substring works on both.
R33_POSITIVE='★     R33 arm 1 COPY'
# ⊘ ANCHORED on purpose, and the opposite reason: `CUP3_VAL=` appears inside this tree's own
#   prose and inside harness echoes, so an unanchored match reads a comment as a measurement.
CUP3_POSITIVE='^CUP3_VAL=43'

usage() {
  cat >&2 <<'USAGE'
usage:
  relaxation_inert_gate.sh grade <cup3_probe_log> <r33_probe_log>
  relaxation_inert_gate.sh run   <tag> [VAR=VALUE ...]

There is deliberately no way to grade one workload. See the header.
USAGE
  exit 64
}

# ★★★ THE TERMINATORS — the workload's OWN exit line, written after it finished.
#
# ⊘⊘ **Without these the gate cannot tell "the workload ran and did not fire" from "the
#    artefact is short", and it MUST.** The first is `NOT-INERT` (a finding about the
#    relaxation); the second is `UNMEASURED` (a finding about the harness). ⚠ A truncated
#    artefact reads as PRESENT and looks healthier than an empty one — it has a plausible
#    name, a fresh timestamp and real content — so non-emptiness cannot be the test. The
#    terminator can only be present if the workload reached its own end.
R33_TERMINATOR='^R33_RC=[0-9]+'
CUP3_TERMINATOR='^CUP3_RC=[0-9]+'

# Grade one log for one known-positive.
#   $1 = path, $2 = workload selector (cup3|r33)
# echoes a status word: FIRED | RAN-BUT-ABSENT | UNTERMINATED | NOLOG
grade_one() {
  local path=$1 kind=$2 hits term
  if [ ! -e "$path" ]; then echo NOLOG; return; fi
  case "$kind" in
    cup3) hits=$(grep -acE "$CUP3_POSITIVE" "$path" 2>/dev/null)
          term=$(grep -acE "$CUP3_TERMINATOR" "$path" 2>/dev/null) ;;
    r33)  hits=$(grep -acF "$R33_POSITIVE" "$path" 2>/dev/null)
          term=$(grep -acE "$R33_TERMINATOR" "$path" 2>/dev/null) ;;
    *) echo NOLOG; return ;;
  esac
  if [ "${hits:-0}" -ge 1 ]; then echo FIRED; return; fi
  [ "${term:-0}" -ge 1 ] && echo RAN-BUT-ABSENT || echo UNTERMINATED
}

cmd_grade() {
  local cup3=${1:-} r33=${2:-}
  [ -n "$cup3" ] && [ -n "$r33" ] || usage
  local c r cs rs
  c=$(grade_one "$cup3" cup3)
  r=$(grade_one "$r33" r33)
  cs=$(stat -c%s "$cup3" 2>/dev/null || echo MISSING)
  rs=$(stat -c%s "$r33" 2>/dev/null || echo MISSING)
  echo "=== ★★★★★ TWO-WORKLOAD INERTNESS GATE (w313) — BOTH PLANES OR NO VERDICT"
  echo "    workload 1  GR/COMPUTE  cup3 (libcuda, a real GR launch)   log=[$cup3] ${cs} bytes"
  echo "                known-positive '${CUP3_POSITIVE}'  => [$c]"
  echo "    workload 2  RAW CE      R33 arm 1 (no libcuda, own VAS)    log=[$r33] ${rs} bytes"
  echo "                known-positive '${R33_POSITIVE}'  => [$r]"
  echo "    ⊘ FIRED = the known-positive is present · RAN-BUT-ABSENT = the workload reached its"
  echo "      own terminator and did NOT fire (a finding about the RELAXATION) · UNTERMINATED ="
  echo "      the log stops early, empty or truncated (a finding about the HARNESS) · NOLOG."
  # ★★★ ORDER MATTERS AND IT IS DELIBERATE: UNMEASURED OUTRANKS NOT-INERT.
  #   If ANY plane is unmeasured the run cannot say anything about the relaxation, even if the
  #   other plane visibly broke — because the arm that broke might be the untrustworthy one.
  #   ⊘ Failing toward "we do not know" is the safe direction for a gate whose whole job is to
  #     stop a deletion.
  if [ "$c" = NOLOG ] || [ "$r" = NOLOG ] || [ "$c" = UNTERMINATED ] || [ "$r" = UNTERMINATED ]; then
    echo "$VERDICT_BANNER = UNMEASURED"
    [ "$c" = NOLOG ] && echo "    ⊘ the GR/COMPUTE log does not exist. ⊘ NOT 'cup3 failed'."
    [ "$r" = NOLOG ] && echo "    ⊘ the RAW CE log does not exist. ⊘ NOT 'the CE client failed'."
    [ "$c" = UNTERMINATED ] && echo "    ⊘ the GR/COMPUTE log has no ^CUP3_RC= terminator: empty or truncated."
    [ "$r" = UNTERMINATED ] && echo "    ⊘ the RAW CE log has no ^R33_RC= terminator: empty or truncated."
    echo "      ⊘ UNMEASURED IS NOT A PASS, and it is not 'inert'. It is the state w304's"
    echo "        evidence was actually in for the raw-CE plane: that boot never ran."
    return 2
  fi
  if [ "$c" = FIRED ] && [ "$r" = FIRED ]; then
    echo "$VERDICT_BANNER = INERT-ON-BOTH-PLANES"
    echo "    ⚠ SCOPED: inert for the GR/compute plane and the raw-CE plane AS THIS TREE"
    echo "      EXERCISES THEM. Display, NVENC and multi-process are UNMEASURED, not inert."
    return 0
  fi
  echo "$VERDICT_BANNER = NOT-INERT"
  [ "$c" = RAN-BUT-ABSENT ] && echo "    ⇒ the GR/COMPUTE plane broke: cup3 ran and did not return 43."
  [ "$r" = RAN-BUT-ABSENT ] && echo "    ⇒ the RAW CE plane broke: R33 arm 1 ran and its four-fact bar did not fire."
  echo "    ⊘ This is exactly w304's blind spot when only ONE of these two is asked: the"
  echo "      other plane passed, and a one-workload gate reported that as inert."
  return 1
}

cmd_run() {
  local tag=${1:-}; shift || true
  [ -n "$tag" ] || usage
  local SELFDIR REPO
  SELFDIR=$(cd "$(dirname "$0")" && pwd)
  REPO=${KAYFABE_REPO:-$(cd "$SELFDIR/../.." && pwd)}
  export KAYFABE_REPO="$REPO"
  # ⊘ The ablation is applied to BOTH arms identically, from ONE list, so the two boots cannot
  #   silently run different experiments. `env` rather than `export` so the assignment is
  #   visible in this file's own log beside each invocation.
  echo "=== w313 INERT GATE — ablation = [$*]  tag=$tag  repo=$REPO $(date -Is)"
  echo "--- arm 1/2: GR/COMPUTE (cup3)"
  ( KAYFABE_TAG="${tag}cup3" env "$@" "$REPO/scripts/bench/w297_cup3.sh" )
  # ⊘ NO BACKTICKS inside a double-quoted echo — bash runs them as command substitution, and
  #   the harness would silently report the output of a program called `fresh`.
  echo "--- arm 2/2: RAW CE (R33 arm 1, via the w309 crit1 harness, arm=fresh)"
  ( KAYFABE_TAG="${tag}r33" env "$@" "$REPO/scripts/bench/w309_crit1.sh" fresh )
  echo "--- grading BOTH:"
  cmd_grade "/workspace/bench/run_${tag}cup3_probe.log" "/workspace/bench/run_${tag}r33_probe.log"
}

case "${1:-}" in
  grade) shift; cmd_grade "$@" ;;
  run)   shift; cmd_run   "$@" ;;
  *) usage ;;
esac
