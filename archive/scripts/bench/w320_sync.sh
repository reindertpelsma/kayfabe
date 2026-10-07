#!/usr/bin/env bash
# ★★★★★ w320 — ATTRIBUTE THE `cuCtxSynchronize` FLOOR. One boot per arm.
#
#   usage: scripts/bench/w320_sync.sh <arm>
#     ksweep  — ★ THE DISCRIMINATOR. K launches per ONE sync, K in {1..128}, at the SMALLEST
#               size. Fits total(K) = a*K + b: `a` is the per-LAUNCH term, `b` the per-SYNC
#               term. Answers the batching question and the duration question at once.
#     sizes   — sync vs KERNEL DURATION at K=1, N in {128,512,1024,2048}. ★ Also the
#               KNOWN-POSITIVE for the duration knob: N=2048 MUST move sync, or a flat curve
#               at small N proves nothing about anything.
#     spin    — CU_CTX_SCHED_SPIN. If the wait is a BLOCK, this collapses it.
#     block   — CU_CTX_SCHED_BLOCKING_SYNC. The other end of the same knob.
#     negctrl — ★ BENCH_NOLAUNCH=1 as the FIRST AND ONLY context of its own boot, now
#               covering the SWEEP as well as the per-iteration phase.
#
# ## ⊘ WHY THIS SCRIPT IS THIN, AND WHAT IT DELIBERATELY DOES NOT ADD
#
# The brief says: extend w315's instrument, do not build a new one. So this drives
# `w315_floor.sh base` — the same boot path, the same sampler, the same grading — and varies
# ONLY the workload's environment. `KAYFABE_KFTIME` stays OFF in every arm, and that is a
# measurement decision rather than an omission:
#
#   ⊘⊘ **kftime CANNOT SEE THIS WINDOW.** Every one of its hooks is inside an MMIO trap, and
#      its containment licence is the vmexit — "the guest is stopped for the whole of it"
#      (kftime.rs:53-58). DURING A SYNC THE GUEST IS NOT STOPPED. Arming it here would produce
#      a breakdown of the submit side beside a sync side it has no hook in, and the sync
#      remainder would read as "unaccounted" when it is really "not instrumented". ⇒ the sync
#      instrument is GUEST-SIDE (cup8bench's CLOCK_THREAD_CPUTIME_ID split), where both halves
#      are the same thread's own clocks and the breakdown closes by construction.
#
# ## ★★★ THE GATE IS ON IN EVERY ARM
#
# w318's dirty gate is OFF BY DEFAULT. The 27.55 ms launch this rung exists to explain is the
# GATED number, so every arm here arms it. ⊘ An arm that forgot would be measuring w315's
# world and reporting it as w320's.
set -uo pipefail
ARM="${1:-}"
case "$ARM" in ksweep|sizes|spin|block|negctrl) ;;
  *) echo "usage: $0 ksweep|sizes|spin|block|negctrl" >&2; exit 64 ;;
esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w320}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w320}
export KAYFABE_TAG=${KAYFABE_TAG:-w320$ARM}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/cup8bench_hook.sh"
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

# ---- w318's gate, ON, in every arm (see header) ----------------------------------------
export KAYFABE_DIRTY_GATE_PUBLISH=${KAYFABE_DIRTY_GATE_PUBLISH:-on}
export KAYFABE_DIRTY_GATE_WITNESS=${KAYFABE_DIRTY_GATE_WITNESS:-on}
# ⊘ kftime OFF in every arm — it cannot see the sync window. See the header.
unset KAYFABE_KFTIME KAYFABE_KFTIME_INJECT_US KAYFABE_KFTIME_INJECT_SEG

export KAYFABE_BENCH_VERIFY=${KAYFABE_BENCH_VERIFY:-1}
export KAYFABE_BENCH_ONLY=${KAYFABE_BENCH_ONLY:-measure}
export KAYFABE_BENCH_CTX_FLAGS=${KAYFABE_BENCH_CTX_FLAGS:-0}

case "$ARM" in
  # ★ N=128 is ~4 MFLOP/launch. At K=128 that is still far below one kernel at N=512, so the
  #   slope measures OUR per-launch cost rather than the GPU's arithmetic. ⊘ Choosing a bigger
  #   size here would confound the slope with real work and make the fit unreadable — which is
  #   exactly why the duration question gets its own arm instead of being read off this one.
  ksweep)  export KAYFABE_BENCH_SIZES=${KAYFABE_BENCH_SIZES:-128}
           export KAYFABE_BENCH_ITERS=${KAYFABE_BENCH_ITERS:-12}
           export KAYFABE_BENCH_BATCH=${KAYFABE_BENCH_BATCH:-1}
           export KAYFABE_BENCH_BATCH_SWEEP=${KAYFABE_BENCH_BATCH_SWEEP:-1,2,4,8,16,32,64,128}
           export KAYFABE_BENCH_BATCH_REPS=${KAYFABE_BENCH_BATCH_REPS:-5} ;;
  # ⊘ SMALL FIRST. cup8bench prints each size's summary when THAT size ends, so a deadline hit
  #   during N=2048 still leaves the three smaller sizes fully measured. At N=2048 one sync is
  #   ~950 ms (w318 r2), so the sweep is DISARMED here — 8 values x 5 reps x 128 launches would
  #   not fit in any boot, and an arm that times out measures nothing at all.
  sizes)   export KAYFABE_BENCH_SIZES=${KAYFABE_BENCH_SIZES:-128,512,1024,2048}
           export KAYFABE_BENCH_ITERS=${KAYFABE_BENCH_ITERS:-12}
           export KAYFABE_BENCH_BATCH=${KAYFABE_BENCH_BATCH:-1}
           export KAYFABE_BENCH_BATCH_SWEEP=""
           export KAYFABE_BENCH_TIMEOUT=${KAYFABE_BENCH_TIMEOUT:-1400} ;;
  spin)    export KAYFABE_BENCH_SIZES=${KAYFABE_BENCH_SIZES:-128,512}
           export KAYFABE_BENCH_ITERS=${KAYFABE_BENCH_ITERS:-12}
           export KAYFABE_BENCH_BATCH=1
           export KAYFABE_BENCH_BATCH_SWEEP=${KAYFABE_BENCH_BATCH_SWEEP:-1,8,64}
           export KAYFABE_BENCH_BATCH_REPS=${KAYFABE_BENCH_BATCH_REPS:-5}
           export KAYFABE_BENCH_CTX_FLAGS=1 ;;   # CU_CTX_SCHED_SPIN
  block)   export KAYFABE_BENCH_SIZES=${KAYFABE_BENCH_SIZES:-128,512}
           export KAYFABE_BENCH_ITERS=${KAYFABE_BENCH_ITERS:-12}
           export KAYFABE_BENCH_BATCH=1
           export KAYFABE_BENCH_BATCH_SWEEP=${KAYFABE_BENCH_BATCH_SWEEP:-1,8,64}
           export KAYFABE_BENCH_BATCH_REPS=${KAYFABE_BENCH_BATCH_REPS:-5}
           export KAYFABE_BENCH_CTX_FLAGS=4 ;;   # CU_CTX_SCHED_BLOCKING_SYNC
  negctrl) export KAYFABE_BENCH_ONLY=negctrl
           export KAYFABE_BENCH_SIZES=256
           export KAYFABE_BENCH_ITERS=3 ;;
esac

echo "=== ★★★★★ W320 arm=$ARM tag=$KAYFABE_TAG $(date -Is)"
echo "    sizes=[${KAYFABE_BENCH_SIZES:-}] iters=[${KAYFABE_BENCH_ITERS:-}]"
echo "    sweep=[${KAYFABE_BENCH_BATCH_SWEEP:-<none>}] reps=[${KAYFABE_BENCH_BATCH_REPS:-}]"
echo "    ctx_flags=[${KAYFABE_BENCH_CTX_FLAGS}] gate=[$KAYFABE_DIRTY_GATE_PUBLISH/$KAYFABE_DIRTY_GATE_WITNESS]"
echo "    repo=[$REPO] HEAD=[$(cd "$REPO" && git rev-parse --short HEAD 2>/dev/null)]"

"$REPO/scripts/bench/w315_floor.sh" base
BRC=$?

P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log

echo ""
echo "================================================================================"
echo "=== ★★★★★ W320 arm=$ARM inner_rc=$BRC  $(date -Is)"
echo "================================================================================"

echo ""
echo "=== ★★★ PRECONDITION — the thread-state clock. Without it the split is meaningless."
grep -h '^GUEST_TSTATE_AVAILABLE=' "$P" 2>/dev/null | sed 's/^/    /' \
  || echo "    ⊘ GUEST_TSTATE_AVAILABLE ABSENT — UNMEASURED, not 'yes'."
grep -h '^GUEST_BENCH_W320=' "$P" 2>/dev/null | sed 's/^/    /' \
  || echo "    ⊘ GUEST_BENCH_W320 ABSENT — the workload is not w320's. Every number below is w318's."

echo ""
echo "=== ★★★★★ THE SYNC BREAKDOWN — one thread, two of its own clocks, and it CLOSES"
echo "    offcpu = wall - cpu, EXACTLY. There is no third term and no residue to distribute."
grep -h '^GUEST_BSUM ' "$P" 2>/dev/null | sed 's/^/    /' \
  || echo "    ⊘ NO BSUM LINE — no size completed. UNMEASURED."

echo ""
echo "=== ★★★★★ THE BATCH SWEEP — the per-LAUNCH term vs the per-SYNC term"
S=$(grep -c '^GUEST_BSWEEPSUM ' "$P" 2>/dev/null)
if [ "${S:-0}" -gt 0 ]; then
  grep -h '^GUEST_BSWEEPSUM ' "$P" 2>/dev/null | sed 's/^/    /'
else
  echo "    ⊘ NO SWEEP ROWS. For arm=[$ARM] with sweep=[${KAYFABE_BENCH_BATCH_SWEEP:-<none>}]"
  echo "      that is EXPECTED IFF the sweep was disarmed; otherwise it is UNMEASURED."
fi

echo ""
echo "=== ★ THE PER-ITERATION LINES (the paired values; the identity holds HERE, per line)"
grep -h 'ITER N=' "$P" 2>/dev/null | tail -14 | sed 's/^/    /'

echo ""
echo "=== ★★ CORRECTNESS — never optional, and inverted in the negctrl arm"
for k in GUEST_BENCH_TOTAL_BAD GUEST_BENCH_VERDICT GUEST_SIZES_DONE GUEST_XID_COUNT \
         GUEST_NEGCTRL_TOTAL_BAD GUEST_NEGCTRL_SWEEP_ROWS GUEST_NEGCTRL_SWEEP_ROWS_WITH_BAD_0; do
  L=$(grep -h "^$k=" "$P" 2>/dev/null | tail -1)
  echo "    ${L:-⊘ $k ABSENT — UNMEASURED, not 0}"
done

echo ""
echo "=== ★ THE HOST SIDE, correlational ONLY (no containment licence during a sync)"
echo "    host_rows      = [$(grep -aoE 'host_rows=[0-9]+' "$Q" 2>/dev/null | tail -1)]"
echo "    Xid lines      = [$(grep -ac 'Xid' "$Q" 2>/dev/null)]"
echo "    await_sem hits = [$(grep -ac 'await_semaphore\|AWAIT-SEM' "$Q" 2>/dev/null)]"
echo "    ⊘ These are printed because they are cheap, NOT because anything above rests on them."
echo "=== W320 arm=$ARM DONE rc=$BRC ==="
exit $BRC
