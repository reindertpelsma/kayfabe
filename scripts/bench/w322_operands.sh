#!/usr/bin/env bash
# ★★★★★ w322 — LOCATE THE OPERANDS. One boot per arm.
#
#   usage: scripts/bench/w322_operands.sh <arm>
#     bw       — ★ THE APERTURE SPECTROMETER. A streaming-read kernel over a SWEEP of buffer
#                sizes, in the guest, at the default placement (`cuMemAlloc`). Read against
#                the native VRAM and native sysmem plateaus that w322_native.sh measures on
#                THIS GPU with THIS kernel.
#     bwhost   — the same sweep with the guest's operands in the guest's own
#                `cuMemHostAlloc(DEVICEMAP)`. ⊘ w320 could never ask for this: BENCH_HOSTMEM
#                was not forwarded to the guest. If the two arms read the SAME, the guest's
#                two allocation paths land in the same host backing — which is a fact about
#                where they land, obtained without reading a single line of our own code.
#     bwneg    — ★ THE KNOWN-POSITIVE, its own boot, first and only CUDA context.
#                BENCH_NOLAUNCH=1 ⇒ the bw verifier MUST report bad>0.
#     sizes    — w320's matmul size curve, re-run only if a same-hour control is needed.
#
# ## ★★★★★ THE OTHER HALF OF THE MEASUREMENT IS ON THE HOST, AND IT NEEDS NO CODE
#
# The sweep allocates 1 -> 256 MiB in sequence, one buffer live at a time. So:
#
#   - if those bytes are HOST VRAM, `nvidia-smi memory.used` must STEP by ~256 MiB while the
#     largest row runs, and step back down when it is freed;
#   - if they are HOST SYSMEM (an arena in our own process), the resident set of the isolate /
#     QEMU must grow by that much and `memory.used` must not;
#   - if they are GUEST RAM pinned through, neither moves — guest RAM is a preallocated 2 GiB
#     memfd and a pin adds no new pages on either counter.
#
# ⇒ **Three hypotheses, three DIFFERENT signatures, on counters that owe nothing to our own
#   bookkeeping.** This is why the sampler below is part of the arm rather than a nicety: it
#   is an instrument that cannot be fooled by our own address table being wrong.
# ⚠ It samples at 1 Hz. A row that completes inside a second can be MISSED ENTIRELY, and a
#   missed step reads exactly like a step that never happened. That is why the largest row is
#   also the SLOWEST one, and why the raw samples are kept rather than only their maximum.
#
# ## ⊘ WHAT THIS RUNG CHANGES IN THE DEVICE: NOTHING.
#
# `git diff master -- crates/` is empty for this branch. Every edit is in scripts/bench/, and
# the workload's new behaviour is behind env vars that default to the previous behaviour. ⇒ a
# non-regression this rung cannot cause is not evidence it is asked to produce; the correctness
# ladder here exists to show the MEASUREMENT is sound, not to clear a change.
set -uo pipefail
ARM="${1:-}"
case "$ARM" in bw|bwhost|bwneg|sizes) ;;
  *) echo "usage: $0 bw|bwhost|bwneg|sizes" >&2; exit 64 ;;
esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w322}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w322}
export KAYFABE_TAG=${KAYFABE_TAG:-w322$ARM}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/cup8bench_hook.sh"
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

# w318's gate ON in every arm — the numbers this rung explains are the GATED ones.
export KAYFABE_DIRTY_GATE_PUBLISH=${KAYFABE_DIRTY_GATE_PUBLISH:-on}
export KAYFABE_DIRTY_GATE_WITNESS=${KAYFABE_DIRTY_GATE_WITNESS:-on}
unset KAYFABE_KFTIME KAYFABE_KFTIME_INJECT_US KAYFABE_KFTIME_INJECT_SEG

export KAYFABE_BENCH_VERIFY=${KAYFABE_BENCH_VERIFY:-1}
export KAYFABE_BENCH_ONLY=${KAYFABE_BENCH_ONLY:-measure}
export KAYFABE_BENCH_CTX_FLAGS=0
export KAYFABE_BENCH_BW_TARGET_MIB=${KAYFABE_BENCH_BW_TARGET_MIB:-256}
export KAYFABE_BENCH_BW_ITERS=${KAYFABE_BENCH_BW_ITERS:-7}

# ★★★★★ BENCH_BW_REPS=1 IN EVERY BW ARM, AND THE FIRST RUN IS WHY.
#
# The repeat loop lives INSIDE the thread, so its reuse working set is
# `resident_threads * (NF/NT) * 4` — not the buffer. On a GA106 only ~46 080 of the 262 144
# threads are resident, so at 16 MiB that is ~2.95 MiB and it FITS IN L2. The first native
# sweep measured the consequence unmistakably: `vram` read 1930 GB/s at 16 MiB against
# 325 GB/s at 64 MiB, and `hostalloc` read 107 GB/s at 16 MiB — **8.5x PCIe gen3 x16's
# theoretical ceiling**, which no real sysmem read can do.
# ⇒ every row with R>1 was reporting L2, not the aperture. With R=1 each byte is fetched from
#   the backing store EXACTLY ONCE, so bytes/time is the aperture at any size, and the whole
#   sweep becomes readable instead of only the one row where `mib == target`.
case "$ARM" in
  # ⊘ SMALL FIRST: the program prints each row when THAT row ends, so a refusal at the largest
  #   size still leaves every smaller row fully measured. ⚠ The first run's 64 MiB row died in
  #   the FILL (rc=719) and took the context with it, losing 256 MiB too; the fill is chunked
  #   now, and 32 MiB is included so there is a size between the two.
  bw)      export KAYFABE_BENCH_BW=${KAYFABE_BENCH_BW:-4,16,32,64,128}
           export KAYFABE_BENCH_BW_REPS=1
           export KAYFABE_BENCH_BW_ONLY=1
           export KAYFABE_BENCH_SIZES=256
           export KAYFABE_BENCH_ITERS=3
           export KAYFABE_BENCH_TIMEOUT=${KAYFABE_BENCH_TIMEOUT:-1400} ;;
  bwhost)  export KAYFABE_BENCH_BW=${KAYFABE_BENCH_BW:-4,16,32,64,128}
           export KAYFABE_BENCH_BW_REPS=1
           export KAYFABE_BENCH_BW_ONLY=1
           export KAYFABE_BENCH_ALLOC=hostalloc
           export KAYFABE_BENCH_SIZES=256
           export KAYFABE_BENCH_ITERS=3
           export KAYFABE_BENCH_TIMEOUT=${KAYFABE_BENCH_TIMEOUT:-1400} ;;
  bwneg)   export KAYFABE_BENCH_ONLY=measure
           export KAYFABE_BENCH_NOLAUNCH=1
           export KAYFABE_BENCH_BW=${KAYFABE_BENCH_BW:-4,16}
           export KAYFABE_BENCH_BW_REPS=1
           export KAYFABE_BENCH_BW_ONLY=1
           export KAYFABE_BENCH_SIZES=256
           export KAYFABE_BENCH_ITERS=3
           export KAYFABE_BENCH_TIMEOUT=${KAYFABE_BENCH_TIMEOUT:-400} ;;
  sizes)   export KAYFABE_BENCH_SIZES=${KAYFABE_BENCH_SIZES:-128,512,1024,2048}
           export KAYFABE_BENCH_ITERS=12
           export KAYFABE_BENCH_BATCH=1
           export KAYFABE_BENCH_TIMEOUT=${KAYFABE_BENCH_TIMEOUT:-1400} ;;
esac

FBLOG=/workspace/bench/run_${KAYFABE_TAG}_fb.log
RSSLOG=/workspace/bench/run_${KAYFABE_TAG}_rss.log

echo "=== ★★★★★ W322 arm=$ARM tag=$KAYFABE_TAG $(date -Is)"
echo "    bw=[${KAYFABE_BENCH_BW:-<none>}] reps=[${KAYFABE_BENCH_BW_REPS:-target-based}] alloc=[${KAYFABE_BENCH_ALLOC:-<default:vram>}]"
echo "    repo=[$REPO] HEAD=[$(cd "$REPO" && git rev-parse --short HEAD 2>/dev/null)]"

# ---- ★★★ THE HOST-SIDE COUNTERS. Started BEFORE the boot so the baseline is a MEASUREMENT.
{
  echo "# epoch_s  fb_used_MiB  gpu_util  qemu_rss_KiB  compute_apps"
  while true; do
    # ⚠ THE FIRST RUN'S BUG, FIXED HERE: `--format=csv,noheader,nounits` returns "1234, 7"
    #   and `tr -d " "` glued both numbers into ONE field, so the analyser's $2 was the RSS
    #   column and every sample parsed as UNMEASURED — 63 samples, zero read. ★ The data was
    #   never missing; the READER was wrong, and an empty summary looked exactly like a
    #   sampler that never ran. Split on the comma so each number is its own field.
    FB=$(nvidia-smi --query-gpu=memory.used,utilization.gpu --format=csv,noheader,nounits 2>/dev/null | tr -d " " | tr "," " ")
    APPS=$(nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader,nounits 2>/dev/null | tr '\n' ';' | tr -d ' ')
    # ⚠ `pgrep -x qemu-system-x86` and NOT the full name: /proc/PID/comm truncates at 15
    #   chars, so the long form can never match and this line would silently always be empty.
    RSS=$(for p in $(pgrep -x qemu-system-x86 2>/dev/null); do
            awk '/^VmRSS/{print $2}' "/proc/$p/status" 2>/dev/null; done | paste -sd, -)
    echo "$(date +%s) $FB ${RSS:-none} ${APPS:-none}"
    sleep 1
  done
} >"$FBLOG" 2>&1 &
SAMPLER=$!
echo "    fb sampler pid=$SAMPLER -> $FBLOG"
# ⊘ Kill it on EVERY exit path. A sampler left running writes into the NEXT arm's baseline.
trap 'kill '"$SAMPLER"' 2>/dev/null' EXIT

sleep 2   # so at least two pre-boot samples exist: a baseline of one point is not a baseline

"$REPO/scripts/bench/w315_floor.sh" base
BRC=$?
sleep 2
kill "$SAMPLER" 2>/dev/null

P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log

echo ""
echo "================================================================================"
echo "=== ★★★★★ W322 arm=$ARM inner_rc=$BRC  $(date -Is)"
echo "================================================================================"

echo ""
echo "=== ★★★ ARMING — a run that did not receive the knobs measured the DEFAULT and said so"
grep -h '^GUEST_BENCH_W322=\|^BENCH_W322_PARAMS' "$P" 2>/dev/null | sed 's/^/    /' \
  || echo "    ⊘ NO W322 ARMING LINE — this boot ran w320's workload. UNMEASURED."
grep -ah '^ *BENCH_W322 ' "$P" 2>/dev/null | tail -1 | sed 's/^ */    /' \
  || echo "    ⊘ the workload never printed BENCH_W322 — it is not this rung's binary."

echo ""
echo "=== ★★★★★ THE APERTURE ROWS — read_GBps by buffer size"
echo "    ⊘ A row reading UNMEASURED is a row that measured NOTHING. It is not slow, and it"
echo "      is not zero. Do not average over it."
grep -ah 'BWROW ' "$P" 2>/dev/null | sed 's/^ */    /' \
  || echo "    ⊘ NO BWROW LINES — UNMEASURED."
echo "    module_rc/func_rc:"
grep -ah 'BENCH_BW_MODULE_RC=\|BENCH_BW_FUNC_RC=\|BENCH_BW_ROWS' "$P" 2>/dev/null | sed 's/^ */    /'
echo "    allocation refusals (each is a NAMED reason, never a silent fallback):"
grep -ah 'BW_ALLOC_FAIL \|BW_ADVISE ' "$P" 2>/dev/null | sed 's/^ */    /' || echo "    (none)"

echo ""
echo "=== ★★★★★ THE HOST FRAMEBUFFER COUNTER — the aperture measured OUTSIDE our bookkeeping"
if [ -s "$FBLOG" ]; then
  MIN=$(awk 'NR>1 && $2 ~ /^[0-9]+$/ {print $2}' "$FBLOG" | sort -n | head -1)
  MAX=$(awk 'NR>1 && $2 ~ /^[0-9]+$/ {print $2}' "$FBLOG" | sort -n | tail -1)
  echo "    samples        = $(( $(wc -l <"$FBLOG") - 1 ))"
  echo "    fb_used_min    = ${MIN:-UNMEASURED} MiB"
  echo "    fb_used_max    = ${MAX:-UNMEASURED} MiB"
  echo "    fb_used_delta  = $(( ${MAX:-0} - ${MIN:-0} )) MiB"
  echo "    ★ the sweep's largest buffer is ${KAYFABE_BENCH_BW_TARGET_MIB} MiB. A delta of"
  echo "      that order says the operands are IN THE FRAMEBUFFER; a delta near zero says"
  echo "      they are not — provided the sampler saw the window at all (samples > 0)."
  echo "    distinct fb_used values seen:"
  awk 'NR>1 && $2 ~ /^[0-9]+$/ {print $2}' "$FBLOG" | sort -n | uniq -c | sed 's/^/      /'
  echo "    qemu RSS min/max (KiB): $(awk 'NR>1 && $4 ~ /^[0-9]+$/{print $4}' "$FBLOG" | sort -n | head -1) / $(awk 'NR>1 && $4 ~ /^[0-9]+$/{print $4}' "$FBLOG" | sort -n | tail -1)"
else
  echo "    ⊘ THE SAMPLER PRODUCED NO FILE OR AN EMPTY ONE — UNMEASURED. An empty artifact"
  echo "      is not a measured zero; this is the trap this tree has paid for repeatedly."
fi

echo ""
if [ "$ARM" = bwneg ]; then
  echo "=== ★★★★★ THE ARMING ASSERTION — a known-positive that did not fire is WORSE than none"
  echo "    ⊘ Measured 2026-08-14: this arm exported KAYFABE_BENCH_NOLAUNCH=1, the hook did not"
  echo "      forward it, the workload ran in MEASURE mode and returned"
  echo "      BENCH_VERDICT: PASS (every bw row verified) bad=0 — a GREEN, and the only tell"
  echo "      was BENCH_MODE. So the mode is asserted here rather than assumed."
  if grep -aq 'BENCH_MODE=NOLAUNCH' "$P" 2>/dev/null; then
    NB=$(grep -ah 'BENCH_NOLAUNCH_TOTAL_BAD=' "$P" 2>/dev/null | tail -1 | sed 's/.*=//' | tr -d '
 ')
    echo "    ✔ BENCH_MODE=NOLAUNCH present — the launches really were skipped."
    echo "    BENCH_NOLAUNCH_TOTAL_BAD=${NB:-ABSENT}"
    if [ -n "${NB:-}" ] && [ "${NB:-0}" -gt 0 ] 2>/dev/null; then
      echo "    ★★★★★ KNOWN-POSITIVE FIRED (bad>0 with no launches). Every bad=0 elsewhere is now"
      echo "          ASSERTED rather than inherited."
    else
      echo "    ⊘⊘ KNOWN-POSITIVE DEAD: launches skipped and the verifier still reported 0."
      echo "       Every bad=0 in this rung is VACUOUS."
    fi
  else
    echo "    ⊘⊘⊘ VOID: BENCH_MODE=NOLAUNCH is ABSENT. This arm ran the MEASUREMENT workload"
    echo "        under a negative-control name. It grades NOTHING, and a bad=0 from it must"
    echo "        not be quoted as a control."
  fi
fi

echo ""
echo "=== ★★ CORRECTNESS — never optional, and INVERTED in the bwneg arm"
for k in GUEST_BENCH_TOTAL_BAD GUEST_BENCH_VERDICT GUEST_SIZES_DONE GUEST_XID_COUNT; do
  L=$(grep -h "^$k=" "$P" 2>/dev/null | tail -1)
  echo "    ${L:-⊘ $k ABSENT — UNMEASURED, not 0}"
done

echo ""
echo "=== ★ THE HOST SIDE, correlational ONLY"
echo "    host_rows      = [$(grep -aoE 'host_rows=[0-9]+' "$Q" 2>/dev/null | tail -1)]"
echo "    publish census = [$(grep -aoE 'total=[0-9]+ already_host=[0-9]+ already_pinned=[0-9]+ guest_ram=[0-9]+ not_vidmem=[0-9]+' "$Q" 2>/dev/null | tail -1)]"
echo "    Xid lines      = [$(grep -ac 'Xid' "$Q" 2>/dev/null)]"
echo "=== W322 arm=$ARM DONE rc=$BRC ==="
exit $BRC
