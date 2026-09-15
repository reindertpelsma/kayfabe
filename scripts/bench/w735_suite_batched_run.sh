#!/usr/bin/env bash
# ★★★★★ w735 PLAN C — THE SUITE ACROSS SEVERAL BOOTS, when the device cannot be put back.
#
#   usage: bash scripts/bench/w735_suite_batched_run.sh [tag] [arms-per-boot]
#
# ## When to reach for this, and when NOT to
#
# ⊘ **Only if `w735_suite_run.sh` reports `SUITE_RECOVERIES > SUITE_RECOVERED`.** That says a
# guest-side `modprobe -r nvidia` does NOT put the device back, i.e. the state that wedges it is
# **ours** — the emulated device or the host isolates — and no amount of in-guest recovery can
# reach it. ⚠ If in-guest recovery works, this file is a slower way to learn the same thing and
# should not be run: `[measured w424]` the wedge is permanent *within a QEMU lifetime*, and a
# fresh QEMU is the only lever left.
#
# ## ⊘⊘ THIS IS CONTAINMENT, NOT A FIX, AND IT MUST NOT BE READ AS ONE
#
# Batching gets **30 verdicts**, which is what §7's licence needs. It does not make the wedge go
# away, and the wedge is a real product defect: a guest that opens `/dev/nvidia0` a handful of
# times and then cannot open it again is broken for any real multi-process workload — CUDA opens
# it far more often than this suite does. ⇒ Report the batch count as the measurement it is: the
# number of device opens this build survives.
#
# ## The one property that makes the batches comparable
#
# ★ Each boot runs a DISJOINT subset, in the suite's own declared order, and no arm runs twice.
# The union is exactly the 30. ⊘ A batching scheme that re-ran arms would let an arm's verdict
# depend on which batch it landed in, which is the attribution problem this is trying to fix.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO=${KAYFABE_REPO:-/root/kayfabe}
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:-w735b}
# ⊘ THREE, and the number is MEASURED twice rather than chosen.
#
# `[measured w735, boot w735probe, RMLADDER_OPEN_PROBE=8]` — the open-ordinal probe w424 asked
# for, on this build:
#
#     OPEN 1..4  rc=0     the device opens
#     OPEN 5     rc=124   ⇐ THE WALL. It does not refuse; it HANGS (the 30 s timeout fires)
#     OPEN 6..8  rc=1     unopenable — `openat` now fails fast, WPR2 latched
#
# ⊘ **The wall arm TIMES OUT and the ones after it REFUSE.** Two different signatures for one
# cause, which is why a ledger that collapsed TIMEOUT into FAIL would have hidden the shape.
# ★ `boot_capture.sh` spends one cycle on its own `nvidia-smi` before the hook runs, so the
# probe's open #1 is really cycle #2 ⇒ **five cycles per QEMU lifetime, the sixth hangs** —
# and boot `w735a` agrees independently: nvidia-smi + exactly four passing arms, then the wall.
# ⇒ four arms per boot sits EXACTLY on it; three leaves a margin of one, and an arm that opens
# the device twice would eat that margin without saying so.
# ⚠ Re-measure with `RMLADDER_OPEN_PROBE` on any new build: the wall is a property of the tree.
PER=${2:-3}
cd "$REPO" || { echo "⊘ no repo at $REPO"; exit 2; }

ARMS="--concurrency --timer --engines --doorbell-census --gpu-info-sweep --bus-info-sweep
--gpga-reserve-probe --atomics-probe --pce-mask-probe --dictated-ring --dictated-ring-negative
--late-map-race --blockage-coverage --uvm-invalidate --uvm-mean --alias-two-vas
--alias-unmap-observe --map-propagation --missing-page-fault --map-stress --rpc-mixed-allocs
--cross-client-leak --concurrent-fuzz --defer-liveness --guest-ram-pin --guest-ring-channel
--executor-vas --ce-client --ce-client-guest-ram --bar1-crossing"

set -- $ARMS
total=$#
echo "=== W735 BATCHED SUITE $(date -Is) tag=$TAG — $total arms, $PER per boot ==="
echo "TREE_REV=$(git rev-parse HEAD)"

batch=0; pass=0; fail=0; tmo=0; unm=0; rows="$BENCH/${TAG}_rows.txt"; : > "$rows"
while [ $# -gt 0 ]; do
  batch=$((batch+1))
  sub=""; n=0
  while [ $# -gt 0 ] && [ $n -lt "$PER" ]; do sub="$sub $1"; shift; n=$((n+1)); done
  echo ""
  echo "=== BATCH $batch — fresh QEMU, arms:$sub ==="
  pkill -f '[q]emu-system-x86'
  sleep 3
  # ⊘ The FIRST boot builds; the rest reuse. The tree does not change between batches and
  # `single_store_e6_boot.sh` still refuses a stale rev, so the guard is intact and only the
  # relink is skipped. ⚠ A batched run that rebuilt 10 times would spend a third of itself
  # relinking QEMU.
  [ $batch -gt 1 ] && export W735_SKIP_BUILD=1
  RMLADDER_ARMS="$sub" PREFIX="${TAG}$batch" bash "$SRC_DIR/w735_suite_run.sh" "${TAG}$batch" \
    > "$BENCH/${TAG}_batch${batch}.log" 2>&1
  S="$BENCH/run_${TAG}${batch}_suite.out"
  grep -aE '^\s*--[a-z]' "$S" 2>/dev/null | cut -c1-160 | tee -a "$rows"
  led=$(grep -a 'SUITE_ARMS=' "$S" 2>/dev/null | tail -1)
  echo "    $led"
  # ⊘ Summed from each batch's OWN ledger line, never recomputed from the rows: the rows are
  # formatted for a human and a parser over them would be a second source of truth.
  g() { printf '%s' "$led" | grep -ao "$1=[0-9]*" | tail -1 | cut -d= -f2; }
  pass=$((pass + $(g SUITE_PASS 2>/dev/null || echo 0)))
  fail=$((fail + $(g SUITE_FAIL 2>/dev/null || echo 0)))
  tmo=$((tmo + $(g SUITE_TIMEOUT 2>/dev/null || echo 0)))
  unm=$((unm + $(g SUITE_UNMEASURED 2>/dev/null || echo 0)))
done

pkill -f '[q]emu-system-x86'
echo ""
echo "=== ★★★★★ W735 BATCHED LEDGER — $batch boots ==="
echo "W735B_ARMS=$total W735B_PASS=$pass W735B_FAIL=$fail W735B_TIMEOUT=$tmo W735B_UNMEASURED=$unm"
acc=$((pass+fail+tmo+unm))
echo "W735B_ACCOUNTED=$acc  (⊘ anything but $total means a batch produced no ledger at all)"
echo "W735B_BOOTS=$batch W735B_ARMS_PER_BOOT=$PER"
echo "--- every row, all $batch boots ---"
cat "$rows"
echo "=== W735 BATCHED END $(date -Is) ==="
