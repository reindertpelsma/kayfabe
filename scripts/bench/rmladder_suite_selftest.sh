#!/usr/bin/env bash
# ★★★★★ DOES THE CONTAINMENT ACTUALLY CONTAIN? — offline, no GPU, no guest.
#
# `rmladder_suite.sh` claims three things it had no way to demonstrate: that an arm which cannot
# open the device is reported as UNMEASURED rather than as a defect, that recovery + retry turns
# such an arm back into a real verdict, and that `RMLADDER_RECOVER=none` reproduces the old
# cascade. ⊘ **Every one of those is a claim about a code path that only fires when something is
# already broken**, which is this tree's most-recorded way to ship a diagnostic that never ran.
#
# So the arms are FAKE and the wedge is SCRIPTED: a stub binary that refuses to "open the
# device" until a marker file is removed, and a recovery command that removes it. That makes the
# containment's known-positive independent of any GPU.
#
#   usage: bash rmladder_suite_selftest.sh     ⇒ prints SELFTEST_RC=0 when all four cases hold
set -uo pipefail
SUITE="$(cd "$(dirname "$0")" && pwd)/rmladder_suite.sh"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
WEDGE="$T/wedged"

cat > "$T/stub" <<'STUB'
#!/usr/bin/env bash
# $1.. = --gpu N --<arm>
arm=""; for a in "$@"; do case "$a" in --*) arm="$a";; esac; done
case "$arm" in
  --gpu) ;;
esac
arm=$(printf '%s\n' "$@" | grep -E '^--(pass|wedge|hardwedge|hang|fail)$' | head -1)
# ★ Every arm is a "device open": once the wedge marker exists, NOTHING opens, exactly as the
# real wall behaves (`[measured w424]` opens #1-#3 pass, #4 fails, and it is permanent).
if [ -e "$KF_WEDGE" ]; then
  echo "RM bring-up failed at R1 openat(nvidia0): Input/output error (os error 5)"
  exit 1
fi
case "$arm" in
  --pass)      echo "done"; exit 0;;
  # ⊘ THE WEDGING ARM PASSES. That is what makes this fixture faithful and what makes the
  # cascade hard: `[measured w424]` the arm that leaves the device unusable opens it FINE —
  # the Nth `RmInitAdapter` after it is the one that dies. An arm that failed at its own
  # `openat` would be trivially attributable and is not the case anyone got wrong.
  --wedge)     touch "$KF_WEDGE"; echo "done"; exit 0;;
  --hang)      sleep 600;;
  --fail)      echo "FAIL R17 CE COPY = dst did not move"; exit 1;;
esac
echo unreachable; exit 9
STUB
chmod +x "$T/stub"

export KF_WEDGE="$WEDGE"
ARMS="--pass --wedge --pass --fail"
fails=0
chk() { # name expected actual
  if [ "$2" = "$3" ]; then printf '  ok   %-42s %s\n' "$1" "$3"
  else printf '  ⊘ FAIL %-42s want=%s got=%s\n' "$1" "$2" "$3"; fails=$((fails+1)); fi
}

echo "=== CASE 1 — recovery WORKS: the wedge is cleared, the arms after it get real verdicts ==="
rm -f "$WEDGE"
out=$(RMLADDER_SUITE_LOGDIR="$T/l1" RMLADDER_ARMS="$ARMS" RMLADDER_RECOVER_CMD="rm -f $WEDGE" \
      bash "$SUITE" "$T/stub" 0 5 2>&1)
printf '%s\n' "$out" | sed 's/^/    /'
led=$(printf '%s\n' "$out" | grep -o 'SUITE_PASS=[0-9]* SUITE_FAIL=[0-9]* SUITE_TIMEOUT=[0-9]* SUITE_UNMEASURED=[0-9]*')
# ★ The point of the case: `--fail` is the LAST arm, two arms past the wedge, and it comes back
# with its OWN verdict. Before containment it was collateral.
chk "every arm got its own verdict"  "SUITE_PASS=3 SUITE_FAIL=1 SUITE_TIMEOUT=0 SUITE_UNMEASURED=0" "$led"
chk "recovery attempted and succeeded" "SUITE_RECOVERIES=1 SUITE_RECOVERED=1" \
    "$(printf '%s\n' "$out" | grep -o 'SUITE_RECOVERIES=[0-9]* SUITE_RECOVERED=[0-9]*')"

echo "=== CASE 2 — THE CONTROL: recovery OFF reproduces the cascade this file exists to kill ==="
rm -f "$WEDGE"
out=$(RMLADDER_SUITE_LOGDIR="$T/l2" RMLADDER_ARMS="$ARMS" RMLADDER_RECOVER=none \
      bash "$SUITE" "$T/stub" 0 5 2>&1)
printf '%s\n' "$out" | sed 's/^/    /'
chk "the arms after the wedge are UNMEASURED" "SUITE_PASS=2 SUITE_FAIL=0 SUITE_TIMEOUT=0 SUITE_UNMEASURED=2" \
    "$(printf '%s\n' "$out" | grep -o 'SUITE_PASS=[0-9]* SUITE_FAIL=[0-9]* SUITE_TIMEOUT=[0-9]* SUITE_UNMEASURED=[0-9]*')"
chk "no recovery was counted"        "SUITE_RECOVERIES=0 SUITE_RECOVERED=0" \
    "$(printf '%s\n' "$out" | grep -o 'SUITE_RECOVERIES=[0-9]* SUITE_RECOVERED=[0-9]*')"
chk "and it does NOT report a green" "SUITE_RC=1" "$(printf '%s\n' "$out" | grep -o 'SUITE_RC=[0-9]*')"

echo "=== CASE 3 — recovery ATTEMPTED AND USELESS: UNMEASURED, and the two counters differ ==="
rm -f "$WEDGE"
out=$(RMLADDER_SUITE_LOGDIR="$T/l3" RMLADDER_ARMS="$ARMS" RMLADDER_RECOVER_CMD="true" \
      bash "$SUITE" "$T/stub" 0 5 2>&1)
printf '%s\n' "$out" | sed 's/^/    /'
chk "attempts > recovered is visible" "SUITE_RECOVERIES=2 SUITE_RECOVERED=0" \
    "$(printf '%s\n' "$out" | grep -o 'SUITE_RECOVERIES=[0-9]* SUITE_RECOVERED=[0-9]*')"

echo "=== CASE 4 — a HANG is TIMEOUT, is not UNMEASURED, and does not stop the suite ==="
rm -f "$WEDGE"
out=$(RMLADDER_SUITE_LOGDIR="$T/l4" RMLADDER_ARMS="--pass --hang --pass" RMLADDER_RECOVER_CMD="true" \
      bash "$SUITE" "$T/stub" 0 3 2>&1)
printf '%s\n' "$out" | sed 's/^/    /'
chk "timeout is its own outcome"   "SUITE_PASS=2 SUITE_FAIL=0 SUITE_TIMEOUT=1 SUITE_UNMEASURED=0" \
    "$(printf '%s\n' "$out" | grep -o 'SUITE_PASS=[0-9]* SUITE_FAIL=[0-9]* SUITE_TIMEOUT=[0-9]* SUITE_UNMEASURED=[0-9]*')"
chk "every arm got a row"          "3" "$(printf '%s\n' "$out" | grep -c '^--')"

echo ""
if [ $fails -eq 0 ]; then echo "SELFTEST_RC=0 — the containment has a known-positive"; exit 0; fi
echo "SELFTEST_RC=1 — $fails check(s) failed"; exit 1
