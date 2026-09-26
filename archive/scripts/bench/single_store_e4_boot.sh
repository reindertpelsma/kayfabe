#!/usr/bin/env bash
# ★★★★★ SINGLE-STORE INCREMENT 4 — CUDA INSIDE THE SCRATCHPAD ISOLATE.
#
#   usage: PREFIX=<tag> SCRATCHPAD=<off|on|require> CUDA=<off|on> [START_MB=<n>] \
#          bash single_store_e4_boot.sh
#
# ## ★★ PRE-REGISTERED OUTCOMES — written before any boot, so none reads as the good one
#
#   Q1 THE CLIENT, graded FIRST. `W392D_GUEST_OUTCOME=(P)` with `THREADS 8 of 8`.
#      ⊘ Everything below is a fact about a failed boot if this is not (P). On the CUDA arm it
#      is also the claim that a dynamically-linked, sandboxed-LATE scratchpad isolate did not
#      break the guest — which is the risk §w724d takes.
#
#   Q2 THE CUDA CENSUS EXISTS. `SCRATCHPAD-CUDA AT REALIZE` and `AT END OF RUN`, both arms.
#      ⊘⊘ A MISSING LINE IS AN UNMEASURED BOOT, NOT A FAILED ONE. `CUDA_WALK=DISARMED` is
#      what distinguishes the control arm from a binary that predates the arm, and
#      `CUDA_WALK=ABSENT` distinguishes a binary built without the `cuda-scratchpad` feature.
#
#   Q3 THE GATE. `CUDA_WALK=OK` — CUDA came up, the committed PTX loaded and JITted, the walk
#      kernel launched against a synthetic table image the isolate built itself, and the
#      report VALIDATED and carried the mapping the fixture declared.
#      Five tokens are possible and each means something different:
#        NO_CUDA          `dlopen`/`cuInit` refused — read `why=`, it carries dlerror verbatim
#        LAUNCH_REFUSED   CUDA up, the launch refused
#        REPORT_MALFORMED the report came back and failed property 3
#        WRONG_ANSWER     the report validated and said the wrong thing ⇐ the worst one
#        OK               the gate
#
#   Q4 THE TWO PROBES, §w724d's *"empirical risk"*. `probe_relaunch=` and
#      `probe_failed_launch=` must both begin `PASS`. ⊘ They run AFTER the namespace and the
#      privilege drop; a warm-up covers the normal path and says nothing about error/recovery
#      paths, which is the whole reason these exist.
#      ⚠ `FAIL the deliberately malformed launch SUCCEEDED` is NOT a pass — it means the
#      probe measured nothing.
#
#   Q5 THE ABI REFUSAL FIRED. `abi_refusal_fired=true`. A skewed `abi_version` must be
#      refused BY NAME at launch (§21). A refusal nobody has seen fire is one nobody knows
#      works, so it is exercised on every armed boot.
#
#   Q6 THE COST. `bring_up_ms=` and `jit_ms=` — CUDA bring-up is on the VM-START path now,
#      like the isolate spawn. Report it beside `worst_trap`.
#
#   (E) no `SCRATCHPAD-CUDA AT` line at all ⇒ UNMEASURED. Say where it stopped.
#
# ## Traps encoded inline
#
# - ★★ `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
# - ★ A binary predating this increment prints no `SCRATCHPAD-CUDA` line, which is
#   indistinguishable from "the gate was off" if you only grep for `arm=on`. The binary is
#   checked BY CONTENT and REFUSES.
# - ★ `grep -c` on its own line, never piped into `grep -q`: a pipe closing early returns 141
#   under `pipefail` and MANUFACTURES a failure when the string IS there (measured w418).
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
tag=${PREFIX:-e1a}
SCRATCHPAD=${SCRATCHPAD:-off}
CUDA=${CUDA:-off}

# ⊘ The variable under test is exported HERE and nowhere else, so the boot's configuration
# and the boot's grade come from one statement.
export KAYFABE_SCRATCHPAD="$SCRATCHPAD"
export KAYFABE_SCRATCHPAD_CUDA="$CUDA"
[ -n "${START_MB:-}" ] && export KAYFABE_SCRATCHPAD_START_MB="$START_MB"

# The rest is the standing "under the constraints" configuration (w586_boot.sh), unchanged —
# an evidence run and its control must differ in exactly one variable.
export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_CE_EXECUTOR=host \
       NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} BOOT_TIMEOUT=${BOOT_TIMEOUT:-180}
export POST_CAPTURE_HOOK="${POST_CAPTURE_HOOK:-$SRC_DIR/w392d_mean_hook.sh}"

echo "=== SINGLE-STORE E4 BOOT $(date -Is) tag=$tag ==="
echo "KAYFABE_SCRATCHPAD=$KAYFABE_SCRATCHPAD KAYFABE_SCRATCHPAD_CUDA=$KAYFABE_SCRATCHPAD_CUDA KAYFABE_SCRATCHPAD_START_MB=${KAYFABE_SCRATCHPAD_START_MB:-<unset, default 12288>}"
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

echo "--- ★★★★★ Q2/Q3/Q4/Q5/Q6 THE CUDA CENSUS (both instants; a missing line is UNMEASURED) ---"
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
echo "=== SINGLE-STORE E4 END $(date -Is) tag=$tag ==="
