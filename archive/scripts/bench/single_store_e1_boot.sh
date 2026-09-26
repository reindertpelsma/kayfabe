#!/usr/bin/env bash
# ★★★★★ SINGLE-STORE INCREMENT 1 — THE BOOT THAT MEASURES THE VM-LIFETIME SCRATCHPAD.
#
#   usage: PREFIX=<tag> SCRATCHPAD=<off|on|require> [START_MB=<n>] bash single_store_e1_boot.sh
#
# ## ★★ PRE-REGISTERED OUTCOMES — written before any boot, so none reads as the good one
#
#   Q1 THE CLIENT, graded FIRST. `W392D_GUEST_OUTCOME=(P)` with `THREADS 8 of 8`.
#      ⊘ Everything below is a fact about a failed boot if this is not (P). In particular:
#      on the `off` arm this is the claim that the increment changed nothing on real
#      hardware, which no unit test can make.
#
#   Q2 THE CENSUS EXISTS, ON BOTH ARMS. `SCRATCHPAD AT REALIZE` and `SCRATCHPAD AT END OF
#      RUN` must both appear.
#      ⊘⊘ **A MISSING LINE IS NOT A FAILED RESERVATION — it is an UNMEASURED BOOT**, and the
#      two must not be reported as one. `arm=off` boots print `reservation=DISARMED`, which
#      is what distinguishes the control arm from a binary that predates the arm.
#
#   Q3 THE RESERVATION, on an armed arm. `reservation=HELD RESERVED_MB=<n>` with n > 0.
#      Five outcome tokens are possible and each means something different about the host:
#        NO_WORKER          the isolate never came up — nothing was asked of RM at all
#        PROBE_REFUSED      the probe verb refused (a plane with no RM connection says this)
#        NOTHING_RESERVABLE the probe RAN and answered zero — a real measurement
#        RESERVE_REFUSED    the probe named a size and reserving it then failed
#        HELD               the object is held for the life of the VM
#
#   Q4 THE ADVERTISED SIZE FOLLOWS THE RESERVATION. On a HELD boot, `SCRATCHPAD FB-SIZE
#      derived_from_reservation=<n>` must appear and n must equal `RESERVED_MB`.
#      ⊘ `gpga_is_one_reserved_object.md`: advertise what was reserved, never assert ahead
#      of it.
#
#   Q5 THE SPAWN IS OFF THE GUEST'S PATH. `spawn_ms=` in the REALIZE census is the quantity
#      w470 measured as a 1.62 s vCPU stall inside an MMIO exit. Here the guest is not
#      running, so whatever it costs, the guest does not pay it. Read it BESIDE the boot's
#      own `TRAPWITNESS`/`SLOW-SITES` worst trap, which is where the saving would show.
#
#   (E) no `SCRATCHPAD AT` line at all ⇒ UNMEASURED. Say where it stopped. Not a failure value.
#
# ## Traps encoded inline
#
# - ★★ `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
# - ★ A binary that predates this change prints no `SCRATCHPAD` line at all, which is
#   indistinguishable from "the gate was off" if you only grep for `arm=on`. So the binary is
#   checked BY CONTENT for the string before anything is graded, and it REFUSES.
# - ★ `grep -c` on its own line, never piped into `grep -q`: a pipe closing early returns 141
#   under `pipefail` and MANUFACTURES a failure when the string is present (measured w418).
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
tag=${PREFIX:-e1a}
SCRATCHPAD=${SCRATCHPAD:-off}

# ⊘ The variable under test is exported HERE and nowhere else, so the boot's configuration
# and the boot's grade come from one statement.
export KAYFABE_SCRATCHPAD="$SCRATCHPAD"
[ -n "${START_MB:-}" ] && export KAYFABE_SCRATCHPAD_START_MB="$START_MB"

# The rest is the standing "under the constraints" configuration (w586_boot.sh), unchanged —
# an evidence run and its control must differ in exactly one variable.
export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_CE_EXECUTOR=host \
       NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} BOOT_TIMEOUT=${BOOT_TIMEOUT:-180}
export POST_CAPTURE_HOOK="${POST_CAPTURE_HOOK:-$SRC_DIR/w392d_mean_hook.sh}"

echo "=== SINGLE-STORE E1 BOOT $(date -Is) tag=$tag ==="
echo "KAYFABE_SCRATCHPAD=$KAYFABE_SCRATCHPAD KAYFABE_SCRATCHPAD_START_MB=${KAYFABE_SCRATCHPAD_START_MB:-<unset, default 12288>}"
echo "POST_CAPTURE_HOOK=$POST_CAPTURE_HOOK"

Q_BIN="$BENCH/qemu-build/qemu-system-x86_64"
BIN_REV=$(strings "$Q_BIN" 2>/dev/null | grep -ao 'kayfabe-rev:[0-9a-f]\{40\}' | head -1 | cut -d: -f2)
TREE_REV=$(git -C "${KAYFABE_REPO:-/root/kayfabe}" rev-parse HEAD 2>/dev/null)
echo "BINARY_REV=${BIN_REV:-UNKNOWN} TREE_REV=${TREE_REV:-UNKNOWN}"
if [ -n "$BIN_REV" ] && [ -n "$TREE_REV" ] && [ "$BIN_REV" != "$TREE_REV" ]; then
  echo "⊘⊘⊘ STALE BINARY — refusing to grade a run that does not contain the tree."
  [ "${KAYFABE_ALLOW_STALE_BINARY:-0}" = "1" ] || exit 3
  echo "    ⊘ KAYFABE_ALLOW_STALE_BINARY=1 — proceeding, as asked."
fi

# ★★★ CONTENT CHECK, not a stamp. A binary that predates this increment prints no SCRATCHPAD
# line at all, and "no line" would otherwise read as "the arm was off".
n_sp=$(strings "$Q_BIN" 2>/dev/null | grep -c 'SCRATCHPAD AT')
echo "E1-CONTENT: scratchpad_census=$n_sp (0 ⇒ this binary predates increment 1 — STOP)"
if [ "$n_sp" -eq 0 ]; then
  echo "⊘ REFUSING TO GRADE: the binary has no SCRATCHPAD census. Rebuild, then re-run."
  exit 4
fi

if pgrep -x qemu-system-x86 >/dev/null 2>&1; then echo "⊘ a QEMU is running; refusing"; exit 3; fi

bash "$SRC_DIR/boot_capture.sh" "$tag" > "$BENCH/run_${tag}_driver.log" 2>&1
echo "boot_capture rc=$?"

Q="$BENCH/run_${tag}_qemu.log"; D="$BENCH/run_${tag}_probe.log"

echo "--- Q1 THE CLIENT (graded first; everything below is uninterpretable without it) ---"
echo "[client] $(grep -a 'W392D_GUEST_OUTCOME=' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-90)"
echo "[client] $(grep -a 'THREADS ' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "[client] $(grep -a 'MEAN_FALSIFIER' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"

echo "--- Q2/Q3/Q5 THE SCRATCHPAD CENSUS (both instants; a missing line is UNMEASURED) ---"
n_cens=$(grep -aoc 'SCRATCHPAD AT' "$Q" 2>/dev/null)
echo "E1-CENSUS-LINES=${n_cens:-0}  (expected 2: REALIZE and END OF RUN)"
grep -ao 'SCRATCHPAD AT [^⇒]*⇒[^⊘]\{0,140\}' "$Q" 2>/dev/null | head -4
echo "--- Q4 THE ADVERTISED SIZE ---"
grep -ao 'SCRATCHPAD FB-SIZE[^⇒]*⇒[^⊘]\{0,120\}' "$Q" 2>/dev/null | head -2
echo "--- the reservation, as one greppable row ---"
echo "E1-RESERVATION=$(grep -ao 'reservation=[A-Z_]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E1-RESERVED_MB=$(grep -ao 'RESERVED_MB=[0-9]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"

echo "--- Q5 WHAT THE GUEST PAID: the worst trap, beside the spawn we moved off its path ---"
grep -ao 'TRAPWITNESS[^|]*' "$Q" 2>/dev/null | tail -1
grep -ao 'SLOW-SITES[^⊘]*' "$Q" 2>/dev/null | tail -1

echo "--- the standing constraint rows, so an armed boot can be compared to the control ---"
grep -ao 'BAR-MIRROR [A-Z ]*AT END OF RUN[^—]\{0,200\}' "$Q" 2>/dev/null | tail -1
grep -ao "doorbells: [0-9]* arrived, [0-9]* served, [0-9]* REFUSED[^;]*" "$Q" 2>/dev/null | tail -1
echo "HOST_DMESG_XID=$(grep -ac 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null)"

echo "--- the guest's OWN first failure (NOT the last line: a re-boot attempt masks it) ---"
grep -a 'NVRM' "$BENCH/run_${tag}_dmesg.log" 2>/dev/null | head -8 | cut -c1-170
echo "=== SINGLE-STORE E1 END $(date -Is) tag=$tag ==="
