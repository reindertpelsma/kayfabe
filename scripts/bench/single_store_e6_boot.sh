#!/usr/bin/env bash
# ★★★★★ SINGLE-STORE §6 STEP 1 — THE LIVE WALK SHADOW.
#
#   usage: PREFIX=<tag> SHADOW=<off|on> [START_MB=<n>] bash single_store_e6_boot.sh
#
# The walk kernel is run ALONGSIDE the host walk at every off-vCPU page-table sweep and the
# two answers are compared by kind. **Nothing is published from the kernel** — this boot is
# supposed to change nothing observable, and is graded on the CENSUS, never on parity.
#
# ## ★★ PRE-REGISTERED OUTCOMES — written before any boot, so none reads as the good one
#
#   Q1 THE CLIENT, graded FIRST. `W392D_GUEST_OUTCOME=(P)` with `THREADS 8 of 8`.
#      ⊘ The shadow must cost the guest nothing. If this is not (P) on the armed arm and IS
#      on the control, the shadow is the difference and the census is a fact about a broken
#      boot.
#
#   Q2 THE CENSUS EXISTS. One `WALK-SHADOW` line at END OF RUN, on BOTH arms.
#      ⊘⊘ A MISSING LINE IS AN UNMEASURED BOOT. `WALK-SHADOW ⊘ DISARMED` is what tells the
#      control arm apart from a binary that predates this increment — which is why the binary
#      is checked BY CONTENT below and REFUSES.
#
#   Q3 THE GATE: a census that is **not vacuous** and has **no disagreements**.
#      Read the verdict token, and read it in this order:
#        `⊘⊘ VACUOUS — … never compared`   the shadow never ran. Read `skipped[…]`:
#             on_vcpu=N            every sweep was on a vCPU thread — the shadow declines
#                                  there BY DESIGN (a round trip would block a vCPU)
#             too_many_pages=N     the image budget; read `image[pages_max=…]`
#             unreadable_page=N    the byte source would not serve a page the host just read
#             isolate_refused=N    the isolate said no — its own stderr carries the sentence
#             report_truncated=N   the kernel ran out of run slots
#        `⊘⊘ VACUOUS — … both sides were EMPTY`  compared, and there was nothing to compare
#        `★★★ AGREEMENT`                          ⇐ THE GATE
#        `⊘⊘ N DISAGREEMENTS`                     read `by_kind[…]` and `first[…]`
#
#   Q4 THE SCOPE, which is printed in the line itself and is NOT optional reading:
#      `compared_flags=0xf` (aperture + read-only; volatile/privilege/atomic/KIND are decoded
#      by the KERNEL ONLY), and the kernel walks a RELOCATED COPY holding exactly the pages
#      the host walk visited — so `missing_in_kernel` is fully live and `extra_in_kernel` is
#      live only within those pages. `absent_edges=` is the measured size of that clipping.
#
#   Q5 THE COST: `image[pages_max= staged_bytes=]` — what the shadow put on the wire.
#
#   (E) no `WALK-SHADOW` line at all ⇒ UNMEASURED. Say where it stopped.
#
# ## Traps encoded inline
#
# - ★★ `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
# - ★ `grep -c` on its own line, never piped into `grep -q`: a pipe closing early returns 141
#   under `pipefail` and MANUFACTURES a failure when the string IS there (measured w418).
# - ★ The shadow needs the scratchpad AND its CUDA arm. Arming only this one gets a SAID
#   refusal at REALIZE, not a silent nothing — grep for it.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
tag=${PREFIX:-e6a}
SHADOW=${SHADOW:-off}

# ⊘ The variable under test is exported HERE and nowhere else. The scratchpad and its CUDA
# arm are NOT the variable: the shadow cannot exist without them, so they are on for both
# arms and the two boots differ in exactly one thing.
export KAYFABE_WALK_SHADOW="$SHADOW"
export KAYFABE_SCRATCHPAD=on
export KAYFABE_SCRATCHPAD_CUDA=on
export KAYFABE_SCRATCHPAD_START_MB="${START_MB:-4096}"

export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_CE_EXECUTOR=host \
       NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} BOOT_TIMEOUT=${BOOT_TIMEOUT:-180}
export POST_CAPTURE_HOOK="${POST_CAPTURE_HOOK:-$SRC_DIR/w392d_mean_hook.sh}"

echo "=== SINGLE-STORE E6 BOOT $(date -Is) tag=$tag ==="
echo "KAYFABE_WALK_SHADOW=$KAYFABE_WALK_SHADOW KAYFABE_SCRATCHPAD=$KAYFABE_SCRATCHPAD KAYFABE_SCRATCHPAD_CUDA=$KAYFABE_SCRATCHPAD_CUDA KAYFABE_SCRATCHPAD_START_MB=$KAYFABE_SCRATCHPAD_START_MB"
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

# ★★★ CONTENT CHECK, not a stamp: a binary that predates this increment prints no
# `WALK-SHADOW` line at all, and "no line" would otherwise read as "the arm was off".
n_ws=$(strings "$Q_BIN" 2>/dev/null | grep -c 'WALK-SHADOW')
n_cu=$(strings "$Q_BIN" 2>/dev/null | grep -c 'SCRATCHPAD-CUDA AT')
echo "E6-CONTENT: walk_shadow_strings=$n_ws cuda_census=$n_cu (either 0 ⇒ an OLDER binary — STOP)"
if [ "$n_ws" -eq 0 ] || [ "$n_cu" -eq 0 ]; then
  echo "⊘ REFUSING TO GRADE: the binary predates §6 step 1's live half. Rebuild, then re-run."
  exit 4
fi
n_img=$(strings "$Q_BIN" 2>/dev/null | grep -c 'kayfabe-isolate-cuda')
echo "E6-IMAGE: cuda_image_symbols=$n_img (0 ⇒ built WITHOUT --features cuda-scratchpad ⇒ the shadow CANNOT run)"

if pgrep -x qemu-system-x86 >/dev/null 2>&1; then echo "⊘ a QEMU is running; refusing"; exit 3; fi

bash "$SRC_DIR/boot_capture.sh" "$tag" > "$BENCH/run_${tag}_driver.log" 2>&1
echo "boot_capture rc=$?"

Q="$BENCH/run_${tag}_qemu.log"; D="$BENCH/run_${tag}_probe.log"

echo "--- Q1 THE CLIENT (graded first; everything below is uninterpretable without it) ---"
echo "[client] $(grep -a 'W392D_GUEST_OUTCOME=' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-90)"
echo "[client] $(grep -a 'THREADS ' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "[client] $(grep -a 'MEAN_FALSIFIER' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"

echo "--- ★★★★★ Q2/Q3/Q4/Q5 THE SHADOW CENSUS (a missing line is UNMEASURED, not clean) ---"
n_wc=$(grep -ac 'WALK-SHADOW' "$Q" 2>/dev/null)
echo "E6-SHADOW-LINES=${n_wc:-0}  (0 ⇒ UNMEASURED)"
grep -ao 'WALK-SHADOW[^|]\{0,1400\}' "$Q" 2>/dev/null | tail -2
echo "E6-COMPARED=$(grep -ao 'WALK-SHADOW compared=[0-9]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E6-DISAGREEMENTS=$(grep -ao 'disagreements=[0-9]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E6-BY-KIND=$(grep -ao 'by_kind\[[^]]*\]' "$Q" 2>/dev/null | tail -1)"
echo "E6-SKIPPED=$(grep -ao 'skipped\[[^]]*\]' "$Q" 2>/dev/null | tail -1)"
echo "E6-IMAGE-STATS=$(grep -ao 'image\[[^]]*\]' "$Q" 2>/dev/null | tail -1)"
echo "E6-VACUOUS=$(grep -aoc 'VACUOUS' "$Q" 2>/dev/null)"
echo "--- ★ the per-refresh refusals, verbatim, if any fired ---"
grep -a 'WALK-SHADOW image refused\|WALK-SHADOW refresh refused\|WALK-SHADOW report' "$Q" 2>/dev/null | head -6 | cut -c1-300
echo "--- ★ the isolate's OWN stderr for the shadow verbs ---"
grep -a 'kayfabe-isolate: ⊘ WALK-SHADOW' "$Q" 2>/dev/null | head -6 | cut -c1-300

echo "--- Q2b THE CUDA CENSUS: the shadow cannot run without it ---"
echo "E4-CUDA_WALK=$(grep -ao 'CUDA_WALK=[A-Z_]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E1-RESERVATION=$(grep -ao 'reservation=[A-Z_]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "E1-RESERVED_MB=$(grep -ao 'RESERVED_MB=[0-9]*' "$Q" 2>/dev/null | tail -1 | cut -d= -f2)"
echo "--- ★ the SAID refusal, if the shadow was armed with no isolate to run in ---"
grep -a 'WALK-SHADOW AT REALIZE' "$Q" 2>/dev/null | head -2 | cut -c1-300

echo "--- the arena counter the previous session's plan turned on (reported, not depended on) ---"
grep -ao 'arena\[[^]]*\]' "$Q" 2>/dev/null | tail -1

echo "--- Q5 WHAT THE GUEST PAID ---"
grep -ao 'TRAPWITNESS[^|]*' "$Q" 2>/dev/null | tail -1
grep -ao 'SLOW-SITES[^⊘]*' "$Q" 2>/dev/null | tail -1
grep -ao 'PT-SWEEP[^|]\{0,200\}' "$Q" 2>/dev/null | tail -1
echo "HOST_DMESG_XID=$(grep -ac 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null)"

echo "--- the guest's OWN first failure (NOT the last line: a re-boot attempt masks it) ---"
grep -a 'NVRM' "$BENCH/run_${tag}_dmesg.log" 2>/dev/null | head -8 | cut -c1-170
echo "=== SINGLE-STORE E6 END $(date -Is) tag=$tag ==="
