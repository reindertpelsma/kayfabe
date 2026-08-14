#!/usr/bin/env bash
# ★★★★★ **THE INERTNESS GATE'S OWN SELF-TEST — offline, no GPU, no boot, no QEMU.**
#
# ## ⊘ WHY A SELF-TEST AND NOT "the gate is obviously right"
#
# `a_census_zero_needs_a_known_positive`, and the mirror of it: a gate nobody has watched FAIL
# is a wish. w304's gate was sound and still shipped a regression; this one is grader code
# with three outcomes, and outcomes that have never been produced are outcomes that do not
# exist. Every case below is driven from a **recorded artefact committed in this tree**, so
# the check runs anywhere, in seconds, with no hardware.
#
# ## ★★★★★ CASE 1 IS THE POINT: THE NEW GATE REFUSES w304'S OWN EVIDENCE
#
# w304's arms are committed at `traces/w304_confirm/`. They are cup3 probe logs and there is
# **no raw-CE log among them, because that boot was never run.** The gate must therefore say
# `UNMEASURED` — not `INERT` — over exactly the evidence that was read as proof of inertness.
# ⇒ The regression w313 is fixing is caught, by construction, from the artefacts of the rung
# that caused it.
set -uo pipefail
SELFDIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SELFDIR/../.." && pwd)
GATE="$SELFDIR/relaxation_inert_gate.sh"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

W304=$REPO/traces/w304_confirm
W313=$REPO/traces/w313_restore

fails=0
run_case() {
  local name=$1 want=$2 cup3=$3 r33=$4 got line
  line=$("$GATE" grade "$cup3" "$r33" 2>&1); got=$?
  local verdict
  verdict=$(printf '%s\n' "$line" | grep -oE 'W313 INERT-GATE VERDICT = [A-Z-]+' | tail -1)
  if [ "$got" -eq "$want" ]; then
    printf '  ✔ %-46s exit=%s  %s\n' "$name" "$got" "${verdict:-⊘ NO VERDICT LINE}"
  else
    printf '  ✘ %-46s exit=%s (want %s)  %s\n' "$name" "$got" "$want" "${verdict:-⊘ NO VERDICT LINE}"
    fails=$((fails + 1))
  fi
  # ⊘ The verdict LINE must exist on every path. A grader that exits the right number while
  #   printing nothing is unreadable by a human, which is who acts on it.
  if [ -z "$verdict" ]; then
    printf '     ⊘ the verdict banner was not emitted — the grader is broken, not the case\n'
    fails=$((fails + 1))
  fi
}

echo "=== ★★★★★ w313 INERTNESS-GATE SELF-TEST (offline; 0 pass · 1 NOT-INERT · 2 UNMEASURED)"
echo ""
echo "--- CASE 1 — ★★★★★ w304'S OWN EVIDENCE, GRADED BY THE NEW GATE."
echo "    Its five arms are cup3 logs and there is NO raw-CE log, because that boot never ran."
run_case "w304 ptsweep arm: cup3 only, no raw CE" 2 \
  "$W304/run_w304ptsweep_probe.log" "$W304/run_w304_THERE_IS_NO_R33_LOG.log"
run_case "w304 opjoin arm: cup3 only, no raw CE" 2 \
  "$W304/run_w304opjoin_probe.log" "$W304/run_w304_THERE_IS_NO_R33_LOG.log"
echo "    ⇒ ★ Both UNMEASURED. The gate cannot be satisfied by the evidence that shipped the"
echo "      regression, and the reason it gives is the true one: that boot never happened."
echo ""
echo "--- CASE 2 — the RESTORED tree: both planes fire ⇒ INERT-ON-BOTH-PLANES"
run_case "w313 restore: cup3 43 + R33 arm 1 PASS" 0 \
  "$W313/run_w313restorecup3_probe.log" "$W313/run_w313restore1_probe.log"
echo ""
echo "--- CASE 3 — MASTER at 0ff3e1e2: cup3 is green and the raw CE plane is BROKEN."
echo "    ★ This is the known-positive for the FAILING direction, and it is the case w304's"
echo "      one-workload gate reported as a pass."
run_case "master: cup3 43 + R33 arm 1 FAIL" 1 \
  "$W313/run_w313restorecup3_probe.log" "$W313/run_w313master1_probe.log"
echo ""
echo "--- CASE 4 — the artefact traps, both directions."
: >"$TMP/empty.log"
printf 'CUP3_VAL=43\n' >"$TMP/cup3ok.log"
# ⚠ A TRUNCATED artefact reads as PRESENT and looks healthier than an empty one: it has a
#   plausible name, a fresh timestamp and real content. Only the known-positive's absence
#   distinguishes it, which is why the verdict turns on that line and never on non-emptiness.
head -c 4096 "$W313/run_w313restore1_probe.log" >"$TMP/truncated.log"
run_case "empty raw-CE log (0 bytes)" 2 "$TMP/cup3ok.log" "$TMP/empty.log"
run_case "TRUNCATED raw-CE log (4096 bytes, plausible)" 2 "$TMP/cup3ok.log" "$TMP/truncated.log"
run_case "no cup3 log at all" 2 "$TMP/does_not_exist.log" "$W313/run_w313restore1_probe.log"
echo ""
echo "--- CASE 5 — ⊘ THE ONE-WORKLOAD REQUEST IS NOT EXPRESSIBLE."
echo "    The construction claim of this gate is that a caller CANNOT ask for half of it."
onearg=$("$GATE" grade "$W313/run_w313restorecup3_probe.log" 2>&1); orc=$?
if [ "$orc" -eq 64 ]; then
  printf '  ✔ %-46s exit=64 (usage)\n' "grade with ONE log is refused"
else
  printf '  ✘ %-46s exit=%s (want 64)\n' "grade with ONE log is refused" "$orc"
  fails=$((fails + 1))
fi
echo ""
echo "=== ★ SELF-TEST RESULT: failures = [$fails]  (MUST be 0)"
echo "=== ⊘ 8 assertions were made (7 graded cases + the one-workload refusal). If that number"
echo "===   is not what you expect, the harness changed — a shrinking selftest is the shape"
echo "===   this tree has paid for before, and a count is the cheapest thing that can see it."
exit $([ "$fails" -eq 0 ] && echo 0 || echo 1)
