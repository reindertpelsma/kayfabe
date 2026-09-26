#!/usr/bin/env bash
# ★★★★★ SINGLE-STORE §w724g — IS THE BAR1/BAR2 TRAP PATH REACHABLE?
#
#   usage: PREFIX=<tag> FB_TRAP=<serve|refuse> [SCRATCHPAD=<off|on|require>] [CUDA=<off|on>] \
#          [START_MB=<n>] bash single_store_e36_boot.sh
#
# ## ⊘ WHAT THIS BOOT IS FOR, AND WHAT IT IS NOT
#
# It is **not** the backing switch. `page_backing` still returns memfd leaves; BAR1/BAR2 are
# still served out of the aperture store. This boot answers the ONE question §w724g says must
# be answered before any of that can be deleted:
#
#   > `TRAP_FILLS=0` says traps do not fire. It does not say the trap path is UNREACHABLE.
#   > Make the trap path refuse by name, boot, and see whether the refusal ever fires.
#
# ## ★★ PRE-REGISTERED OUTCOMES — written before any boot, so none reads as the good one
#
#   Q1 THE CLIENT, graded FIRST. `W392D_GUEST_OUTCOME=(P)` with `THREADS 8 of 8`.
#      ⊘ On the `refuse` arm a (P) means something stronger than usual: the guest completed
#      its whole workload with the backstop REMOVED.
#      ⚠ And a NON-(P) on the refuse arm is not automatically a kayfabe defect — it may be
#      exactly the finding (publication was incomplete and the trap path was carrying it).
#      Read Q2 before concluding anything from Q1.
#
#   Q2 THE NUMBER. `FB_TRAP_REFUSALS=` from the END OF RUN census.
#        arm=serve    ⇒ 0 BY CONSTRUCTION. It is the control; it measures nothing.
#        arm=refuse, 0 ⇒ ★★★ the trap path was NEVER REACHED. "Zero in practice" becomes
#                        "unreachable", and increment 7's deletion is licensed.
#        arm=refuse, N ⇒ ⊘⊘ publication is INCOMPLETE, N times. The demand-fill mirror is
#                        load-bearing and must NOT be deleted. **This is a result, not a
#                        failure** — it is the number the whole arm exists to produce.
#
#   Q3 THE CENSUS EXISTS ON BOTH ARMS. `FB-TRAP AT END OF RUN` must appear either way.
#      ⊘⊘ A MISSING LINE IS AN UNMEASURED BOOT, NOT A ZERO.
#
#   Q4 The standing constraint rows, so the arms are comparable: `BAR-MIRROR`, doorbells,
#      `HOST_DMESG_XID`, `TRAPWITNESS`.
#
#   (E) no `FB-TRAP AT` line at all ⇒ UNMEASURED. Say where it stopped. Not a failure value.
#
# ## Traps encoded inline
#
# - ★★ `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
# - ★ A binary predating this change prints no `FB-TRAP` line, which is indistinguishable
#   from "the arm was serve" if you only grep for `refuse`. Checked BY CONTENT, and REFUSES.
# - ★ `grep -c` on its own line, never piped into `grep -q`: a pipe closing early returns 141
#   under `pipefail` and MANUFACTURES a failure when the string IS there (measured w418).
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
tag=${PREFIX:-e1a}
SCRATCHPAD=${SCRATCHPAD:-off}
CUDA=${CUDA:-off}
FB_TRAP=${FB_TRAP:-serve}

# ⊘ The variable under test is exported HERE and nowhere else, so the boot's configuration
# and the boot's grade come from one statement.
export KAYFABE_SCRATCHPAD="$SCRATCHPAD"
export KAYFABE_SCRATCHPAD_CUDA="$CUDA"
export KAYFABE_FB_TRAP="$FB_TRAP"
[ -n "${START_MB:-}" ] && export KAYFABE_SCRATCHPAD_START_MB="$START_MB"

# The rest is the standing "under the constraints" configuration (w586_boot.sh), unchanged —
# an evidence run and its control must differ in exactly one variable.
export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_CE_EXECUTOR=host \
       NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} BOOT_TIMEOUT=${BOOT_TIMEOUT:-180}
export POST_CAPTURE_HOOK="${POST_CAPTURE_HOOK:-$SRC_DIR/w392d_mean_hook.sh}"

echo "=== SINGLE-STORE E36 BOOT $(date -Is) tag=$tag ==="
echo "KAYFABE_FB_TRAP=$KAYFABE_FB_TRAP KAYFABE_SCRATCHPAD=$KAYFABE_SCRATCHPAD KAYFABE_SCRATCHPAD_CUDA=$KAYFABE_SCRATCHPAD_CUDA KAYFABE_SCRATCHPAD_START_MB=${KAYFABE_SCRATCHPAD_START_MB:-<unset, default 12288>}"
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
n_ft=$(strings "$Q_BIN" 2>/dev/null | grep -c 'FB-TRAP AT END OF RUN')
echo "E36-CONTENT: fb_trap_census=$n_ft (0 ⇒ this binary predates §w724g's arm — STOP)"
if [ "$n_ft" -eq 0 ]; then
  echo "⊘ REFUSING TO GRADE: the binary has no FB-TRAP census. Rebuild, then re-run."
  exit 4
fi
n_sp=$(strings "$Q_BIN" 2>/dev/null | grep -c 'SCRATCHPAD AT')
n_cu=$(strings "$Q_BIN" 2>/dev/null | grep -c 'SCRATCHPAD-CUDA AT')
echo "E4-CONTENT: scratchpad_census=$n_sp cuda_census=$n_cu (either 0 ⇒ an OLDER binary — STOP)"
if [ "$n_sp" -eq 0 ] || [ "$n_cu" -eq 0 ]; then
  echo "⊘ REFUSING TO GRADE: the binary predates increment 4. Rebuild, then re-run."
  exit 4
fi
# ★★★ And the IMAGE, by content: a binary built without the `cuda-scratchpad` feature has an
# EMPTY second image and would answer `CUDA_WALK=ABSENT` on an armed boot. ⊘ That is a
# different diagnosis from "CUDA would not load" and must not be discovered from the census.
n_img=$(strings "$Q_BIN" 2>/dev/null | grep -c 'kayfabe-isolate-cuda')
echo "E4-IMAGE: cuda_image_symbols=$n_img (0 ⇒ built WITHOUT --features cuda-scratchpad)"

if pgrep -x qemu-system-x86 >/dev/null 2>&1; then echo "⊘ a QEMU is running; refusing"; exit 3; fi

bash "$SRC_DIR/boot_capture.sh" "$tag" > "$BENCH/run_${tag}_driver.log" 2>&1
echo "boot_capture rc=$?"

Q="$BENCH/run_${tag}_qemu.log"; D="$BENCH/run_${tag}_probe.log"

echo "--- Q1 THE CLIENT (graded first; everything below is uninterpretable without it) ---"
echo "[client] $(grep -a 'W392D_GUEST_OUTCOME=' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-90)"
echo "[client] $(grep -a 'THREADS ' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "[client] $(grep -a 'MEAN_FALSIFIER' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"

echo "--- ★★★★★ Q2/Q3 §w724g: IS THE TRAP PATH REACHABLE? ---"
n_ftc=$(grep -aoc 'FB-TRAP AT END OF RUN' "$Q" 2>/dev/null)
echo "E36-FB-TRAP-CENSUS-LINES=${n_ftc:-0}  (expected 1; 0 ⇒ UNMEASURED, not zero)"
grep -ao 'FB-TRAP [^|]\{0,420\}' "$Q" 2>/dev/null | head -3
echo "E36-FB_TRAP_ARM=$(grep -ao 'FB-TRAP AT END OF RUN: arm=[a-z]*' "$Q" 2>/dev/null | tail -1 | sed 's/.*arm=//')"
echo "E36-FB_TRAP_REFUSALS=$(grep -ao 'FB_TRAP_REFUSALS=[0-9]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "--- ★ where the guest's driver stopped, if it did (read BEFORE blaming the arm) ---"
grep -a 'NVRM' "$BENCH/run_${TAG}_dmesg.log" 2>/dev/null | head -4 | cut -c1-160
echo "--- Q4 the CUDA census, if armed (both instants; a missing line is UNMEASURED) ---"
n_cc=$(grep -aoc 'SCRATCHPAD-CUDA AT' "$Q" 2>/dev/null)
echo "E4-CUDA-CENSUS-LINES=${n_cc:-0}  (expected 2: REALIZE and END OF RUN)"
grep -ao 'SCRATCHPAD-CUDA AT [^|]\{0,900\}' "$Q" 2>/dev/null | head -2
echo "E4-CUDA_WALK=$(grep -ao 'CUDA_WALK=[A-Z_]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E4-ABI_REFUSAL=$(grep -ao 'abi_refusal_fired=[a-z]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E4-JIT_MS=$(grep -ao 'jit_ms=[0-9.]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E4-BRINGUP_MS=$(grep -ao 'bring_up_ms=[0-9.]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "--- ★★ Q4 THE TWO POST-SANDBOX PROBES (both must begin PASS) ---"
grep -ao 'probe_relaunch=\"[^\"]*\"' "$Q" 2>/dev/null | tail -1
grep -ao 'probe_failed_launch=\"[^\"]*\"' "$Q" 2>/dev/null | tail -1
echo "--- ★ the isolate's OWN stderr, in case it died before the parent could ask ---"
grep -a 'kayfabe-isolate: CUDA-WALK' "$Q" 2>/dev/null | tail -4 | cut -c1-400
echo "--- Q2b THE RESERVATION CENSUS (both instants) ---"
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
echo "=== SINGLE-STORE E36 END $(date -Is) tag=$tag ==="
