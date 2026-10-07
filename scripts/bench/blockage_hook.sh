#!/usr/bin/env bash
# ★★★★★ T1 — THE BLOCKAGE-COVERAGE PROBE, run INSIDE the Mode-2 guest by the RAW CLIENT.
#
# `docs/design/REQUIREMENTS_TARGET.md` R1, falsifier **C1**:
#   "any mapping used by a channel with no prior blockage point — count must be 0, with a
#    known-positive denominator"
#
# ⊘⊘ READ THIS BEFORE GRADING ANYTHING: **THE VERDICT IS NOT IN THE GUEST'S OUTPUT.**
# The guest program (`kayfabe-rm-ladder --blockage-coverage`) drives two populations and
# brackets them with `BLOCKAGE_PROBE_BEGIN` / `_END`. The number C1 is graded on is the
# DEVICE's, and it is on the HOST's log as
#     kayfabe: BLOCKAGE-COVERAGE <verdict> armed=[…] publications=N by=[…] uses=N uses_by=[…]
#              ⇒ USES_UNCOVERED=N use_misses=N uncovered_rows=N uncovered_vas=[…] | CHANNEL-KIND …
# This script prints BOTH halves and grades on the host half, and it refuses to grade at all
# when it cannot find the host log — an absent measurement is not a passing one.
#
# ## ★★★ THE TWO PHASES AND WHY BOTH ARE REQUIRED
#   phase A  map, THEN ring        -> the covered population
#   phase B  ring, THEN map        -> R1.3's residual, and THIS COUNTER'S KNOWN-POSITIVE
# `a_census_zero_needs_a_known_positive`: `USES_UNCOVERED=0` on a boot that never contained
# an uncoverable mapping is evidence of NOTHING. Phase B is the only thing in the workload
# that can move the counter.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none reads as the favourable one
#   (1) armed=[all zero]                => ⊘ NEVER-ARMED. No blockage point was entered at
#                                          all. Every zero on the line is UNMEASURED. This is
#                                          NOT a pass and is the FIRST thing to check.
#   (2) publications=0                  => ⊘ NO-PUBLICATIONS. The predicate quantifies over
#                                          the empty set. Not a pass.
#   (3) A=PASS B=PASS USES_UNCOVERED>0  => ★★★★★ THE MEASURABLE OUTCOME. The counter is
#                                          demonstrably able to move, and the number it
#                                          reports for the rest of the boot means something.
#   (4) A=PASS B=PASS USES_UNCOVERED=0  => ⊘ THE COUNTER IS BLIND. The late mapping reached
#                                          the engine and our census did not see it. This is
#                                          an INSTRUMENT red, NOT a coverage green.
#   (5) A=PASS B=FAIL                   => the late map faulted (R1.3's backstop). The
#                                          exposure is bounded; `USES_UNCOVERED=0` is then
#                                          legitimate and UNINFORMATIVE about (4).
#   (6) A=FAIL                          => ⊘ UNINTERPRETABLE. The submit path is broken.
#   (7) no BLOCKAGE-COVERAGE line       => ⊘ UNMEASURED. The device never printed one — no
#                                          doorbell rang, or this archive is too old.
#   (8) CHANNEL-KIND ⊘VACUOUS           => C3 is UNMEASURED on this run. A raw client
#                                          allocates no guest-KERNEL channel, so
#                                          `emulated-doorbell=0` here is CORRECT and EXPECTED.
#                                          Grade C3 beside a CUDA workload, never here.
#
# ⊘ `dlen=0` reasoning applies to this file: an empty probe log is a state that needs its own
#   check, not an absence of findings.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
KEY=/workspace/bench/guest_key
SCP_OPTS=(-i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
          -o LogLevel=ERROR -o ConnectTimeout=5)
BIN=${BLOCKAGE_BIN:-}
TMO=${BLOCKAGE_TIMEOUT:-300}
# ⚠ The host log is where the verdict lives. `boot_capture.sh` names it after the boot tag;
# a caller that does not pass one gets a refusal, not a guess.
HOSTLOG=${BLOCKAGE_HOSTLOG:-}

echo "=== ★★★★★ T1 BLOCKAGE COVERAGE — RAW CLIENT, NO libcuda (R1 / C1) ==="

if ! $G true >/dev/null 2>&1; then
  echo "BLOCKAGE_OUTCOME=(7) ⊘ UNMEASURED_GUEST_UNREACHABLE"; exit 0
fi

# --- ship the raw client in ------------------------------------------------------------
# ⚠ Located, never assumed: a missing binary must attribute to the BUILD, not to the GPU.
if [ -z "$BIN" ]; then
  for c in "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w297}"/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder \
           "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w297}"/release/kayfabe-rm-ladder \
           "${KAYFABE_REPO:-/root/kayfabe}"/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder; do
    [ -x "$c" ] && BIN="$c" && break
  done
fi
if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
  echo "BLOCKAGE_BIN_FOUND=no"
  echo "BLOCKAGE_OUTCOME=(7) ⊘ UNMEASURED_NO_BINARY — rmladder was not built for the guest"
  exit 0
fi
echo "BLOCKAGE_BIN_FOUND=yes ($BIN, $(stat -c %s "$BIN" 2>/dev/null) bytes)"
# ⚠ The bracket trick is NOT enough on a line that later names the binary — put nothing
#   after the transfer on this line. (`nvkvm-pv`, 2026-08-17.)
if ! scp "${SCP_OPTS[@]}" "$BIN" ubuntu@192.168.77.2:/tmp/rmladder >/dev/null 2>&1; then
  echo "BLOCKAGE_OUTCOME=(7) ⊘ UNMEASURED_SCP_FAILED"; exit 0
fi
$G 'chmod +x /tmp/rmladder' >/dev/null 2>&1

echo "--- guest preconditions ---"
echo "GUEST_NVIDIA_NODES=$($G 'ls /dev/nvidia* 2>&1 | tr "\n" " "' 2>&1 | tr -d '\r')"
echo "GUEST_DRIVER=$($G 'cat /proc/driver/nvidia/version 2>&1 | head -1' 2>&1 | tr -d '\r')"
$G 'sudo dmesg -c >/dev/null 2>&1' >/dev/null 2>&1

# ★ A start marker and an exit-status line, so "the file exists but has no terminator" is a
#   DETECTABLE state. A killed background job and a running one are otherwise identical.
echo "--- rmladder --blockage-coverage (in guest) ---"
OUT=$($G "echo BLOCKAGE_STARTED=\$(date -u +%FT%TZ); timeout $TMO /tmp/rmladder --blockage-coverage 2>&1; echo BLOCKAGE_RC=\$?" 2>&1 | tr -d '\r')
echo "$OUT" | sed 's/^/    /'

echo "--- guest dmesg (a GUEST-visible fault lands here, not in the host log) ---"
$G 'sudo dmesg 2>&1 | grep -iE "xid|nvrm|fault" | tail -20' 2>&1 | tr -d '\r' | sed 's/^/    /'

# --- the guest half's own markers -------------------------------------------------------
# ⊘ Grade on the LINES, never on the exit status: a timeout kills the process and its status
#   says nothing about how far the probe got.
PH_A=$(echo "$OUT" | sed -n 's/^BLOCKAGE_PROBE_A=//p' | tail -1)
PH_B=$(echo "$OUT" | sed -n 's/^BLOCKAGE_PROBE_B=//p' | tail -1)
GUEST_OUTCOME=$(echo "$OUT" | sed -n 's/^BLOCKAGE_PROBE_END outcome=//p' | tail -1)
echo ""
echo "GUEST_PHASE_A=${PH_A:-NONE}"
echo "GUEST_PHASE_B=${PH_B:-NONE}"
echo "GUEST_OUTCOME=${GUEST_OUTCOME:-NONE}"

# --- the DEVICE half, which is the one C1 is graded on ----------------------------------
if [ -z "$HOSTLOG" ]; then
  # ⊘ Deliberately NOT a glob-pick of "the newest log". Choosing one would silently attribute
  #   this run's verdict to another boot's line, which is the exact shape
  #   `no_provenance_looks_cleaner_than_bad_provenance` names.
  echo "BLOCKAGE_HOSTLOG=UNSET"
  echo "    ⊘ BLOCKAGE_HOSTLOG was not passed, so the DEVICE's counters were not read."
  echo "    ⊘ This run is UNMEASURED for C1 whatever the guest printed above. Pass"
  echo "       BLOCKAGE_HOSTLOG=/workspace/bench/run_<tag>.log — a guessed log is worse"
  echo "       than none, because its line looks exactly as authoritative."
  echo "BLOCKAGE_OUTCOME=(7) ⊘ UNMEASURED_NO_HOSTLOG"
  exit 0
fi
if [ ! -s "$HOSTLOG" ]; then
  echo "BLOCKAGE_HOSTLOG=$HOSTLOG (ZERO BYTES or missing)"
  echo "    ⊘ Zero bytes is NOT 'nothing happened'; it is a state that needs its own check."
  echo "BLOCKAGE_OUTCOME=(7) ⊘ UNMEASURED_EMPTY_HOSTLOG"
  exit 0
fi
echo ""
echo "--- device counters (host log: $HOSTLOG) ---"
COV_LINES=$(grep -a "BLOCKAGE-COVERAGE" "$HOSTLOG" 2>/dev/null)
COV_N=$(printf '%s' "$COV_LINES" | grep -ac . || true)
echo "BLOCKAGE_LINES=$COV_N"
if [ "${COV_N:-0}" -eq 0 ]; then
  echo "    ⊘ The device printed no BLOCKAGE-COVERAGE line at all."
  echo "    ⊘ Two causes and they are different findings: no doorbell rang (the probe never"
  echo "       reached the device), or this archive predates the census. Check for any"
  echo "       'kayfabe: PT-DECODE' line to tell them apart."
  echo "PT_DECODE_LINES=$(grep -ac 'PT-DECODE' "$HOSTLOG" 2>/dev/null || echo 0)"
  echo "BLOCKAGE_OUTCOME=(7) ⊘ UNMEASURED_NO_DEVICE_LINE"
  exit 0
fi
# ★ The LAST per-doorbell line is the live state; the `AT=teardown` copy is the final one and
#   its CHANNEL-KIND half is expected to be ⊘VACUOUS. Both are printed, and which is which is
#   said, because grading C3 on the teardown copy is a mistake this file exists to prevent.
LIVE=$(printf '%s\n' "$COV_LINES" | grep -a -v 'AT=teardown' | tail -1)
FINAL=$(printf '%s\n' "$COV_LINES" | grep -a 'AT=teardown' | tail -1)
echo "LIVE   : ${LIVE:-<none>}"
echo "TEARDOWN: ${FINAL:-<none>}"

GRADE_LINE="${LIVE:-$FINAL}"
ARMED=$(printf '%s' "$GRADE_LINE" | sed -n 's/.*armed=\[\([^]]*\)\].*/\1/p')
PUBS=$(printf '%s' "$GRADE_LINE" | sed -n 's/.*publications=\([0-9]*\).*/\1/p')
UNCOV=$(printf '%s' "$GRADE_LINE" | sed -n 's/.*USES_UNCOVERED=\([0-9]*\).*/\1/p')
UNCOV_ROWS=$(printf '%s' "$GRADE_LINE" | sed -n 's/.*uncovered_rows=\([0-9]*\).*/\1/p')
echo "DEVICE_ARMED=[${ARMED:-?}]"
echo "DEVICE_PUBLICATIONS=${PUBS:-?}"
echo "DEVICE_USES_UNCOVERED=${UNCOV:-?}"
echo "DEVICE_UNCOVERED_ROWS=${UNCOV_ROWS:-?}"
echo "DEVICE_CHANNEL_KIND=$(printf '%s' "$GRADE_LINE" | sed -n 's/.*\(CHANNEL-KIND [^|]*\)/\1/p')"

echo ""
echo "=== ★★★★★ THE VERDICT, stated once, in the pre-registered vocabulary"
if printf '%s' "$GRADE_LINE" | grep -q 'NEVER-ARMED'; then
  echo "    BLOCKAGE_OUTCOME=(1) ⊘ NEVER-ARMED — no blockage point was entered. Every zero"
  echo "        on that line is UNMEASURED. Check that the guards are installed in THIS build."
elif printf '%s' "$GRADE_LINE" | grep -q 'NO-PUBLICATIONS'; then
  echo "    BLOCKAGE_OUTCOME=(2) ⊘ NO-PUBLICATIONS — the predicate quantifies over the empty"
  echo "        set. Not a pass."
elif [ "${PH_A:-}" != PASS ]; then
  echo "    BLOCKAGE_OUTCOME=(6) ⊘ UNINTERPRETABLE — the positive control A=${PH_A:-NONE}."
  echo "        The submit path is broken and every device number measures that."
elif [ "${PH_B:-}" = PASS ] && [ "${UNCOV:-0}" -gt 0 ]; then
  echo "    BLOCKAGE_OUTCOME=(3) ★★★★★ MEASURABLE — the known-positive fired"
  echo "        (USES_UNCOVERED=$UNCOV over publications=$PUBS). The counter can move, so"
  echo "        the zeros it reports elsewhere in this boot are MEASURED zeros."
elif [ "${PH_B:-}" = PASS ]; then
  echo "    BLOCKAGE_OUTCOME=(4) ⊘ THE COUNTER IS BLIND — phase B's late mapping reached the"
  echo "        engine and USES_UNCOVERED is still ${UNCOV:-0}. ⚠ This is an INSTRUMENT red."
  echo "        It is NOT a coverage green and must never be reported as one."
else
  echo "    BLOCKAGE_OUTCOME=(5) phase B FAULTED — R1.3's backstop case. The exposure is"
  echo "        bounded; USES_UNCOVERED=${UNCOV:-0} is legitimate here and says NOTHING about"
  echo "        whether the counter can see an uncovered use."
fi
echo "    ⊘ C3 IS NOT GRADED HERE. A raw client allocates no guest-KERNEL channel, so"
echo "       'emulated-doorbell=0' and a ⊘VACUOUS CHANNEL-KIND are CORRECT on this run."
