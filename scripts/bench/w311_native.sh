#!/usr/bin/env bash
# ★★★★★ w311 — THE NATIVE REFERENCE ARM. Runs on the BENCH HOST, no QEMU, no guest.
#
#   usage (on vh):  scripts/bench/w311_native.sh
#   writes:         /workspace/bench/w311_native.log
#
# ## ★★★ Why this arm exists and why it must be the SAME PHYSICAL GPU
#
# The deliverable of w311 is a RATIO — guest GFLOP/s ÷ native GFLOP/s. The absolute figure is
# nearly meaningless (the kernel is a naive triple loop; both arms are far off peak), and the
# ratio IS the forwarding overhead. ⊘ A ratio taken against a remembered or published figure
# is not a ratio: it is a comparison against an unmeasured constant. So this runs the SAME
# SOURCE, built by the SAME compiler with the SAME flags, against the SAME physical GA106 the
# guest arm drives.
#
# ⚠ SERIALISE. The GPU is the one the guest VM is driving in the other arm. This script
#   REFUSES to run if a QEMU is up, and it records nvidia-smi before and after so a
#   contended measurement is detectable rather than silently slow.
#
# ## ⊘ cuda.h is NOT installed on this host, deliberately not worked around
#
# cup8bench.c declares the dozen Driver-API entry points it uses, so `gcc -lcuda` is the whole
# toolchain on BOTH sides. Installing a CUDA toolkit here to satisfy an #include would have
# made the two arms differ by header and compiler version — on a rung whose entire output is
# a ratio between them.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=${W311_NATIVE_LOG:-$BENCH/w311_native.log}
WORK=${W311_NATIVE_DIR:-/workspace/w311}
SIZES=${BENCH_SIZES:-512,1024,2048}
ITERS=${BENCH_ITERS:-30}
BATCH=${BENCH_BATCH:-20}
# ★ w320 — the sweep + ctx-flag knobs, forwarded so the NATIVE arm can run the SAME
#   experiment as the guest arm. ⊘ Default empty/0, so a caller that does not set them gets
#   w311's arm unchanged and the two rungs' native logs stay comparable.
SWEEP=${BENCH_BATCH_SWEEP:-}
HOSTMEM=${BENCH_HOSTMEM:-0}
REPS=${BENCH_BATCH_REPS:-5}
CTXF=${BENCH_CTX_FLAGS:-0}

mkdir -p "$WORK"
exec >"$OUT" 2>&1
echo "=== W311 NATIVE START $(date -Is) pid=$$ ==="
echo "SIZES=$SIZES ITERS=$ITERS BATCH=$BATCH"

# ---- phase 0: nothing else may own the GPU ------------------------------------------
# ★★ `pgrep -x qemu-system-x86_64` CAN NEVER MATCH (/proc/PID/comm truncates at 15 chars) so
#    a guard built on the long name passes vacuously. `pgrep -f <literal>` is the opposite
#    failure: it always matches the asker. Use the short -x name.
if pgrep -x qemu-system-x86 >/dev/null 2>&1; then
  echo "★★★ REFUSING: a QEMU is running (pids $(pgrep -x qemu-system-x86 | tr '\n' ' '))."
  echo "    The native arm must not contend with the guest arm for this GPU."
  echo "=== W311 NATIVE EXIT rc=64 ==="
  exit 64
fi
echo "=== GPU STATE BEFORE ==="
nvidia-smi --query-gpu=name,driver_version,memory.used,utilization.gpu --format=csv
nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv
echo "    ⊘ any process listed above shares this GPU and the numbers below are CONTENDED."

# ---- build ---------------------------------------------------------------------------
# ★ Delete first: `[ -x ]` cannot tell fresh from stale. No build => no file => no run.
cp -f "$SRC_DIR/cup8bench.c" "$WORK/cup8bench.c" || { echo "★ no cup8bench.c"; exit 2; }
rm -f "$WORK/cup8bench"
echo "=== BUILD ==="
echo "source md5 = $(md5sum < "$WORK/cup8bench.c" | cut -d' ' -f1)"
( cd "$WORK" && gcc -O2 -o cup8bench cup8bench.c -lcuda -lm ) ; GRC=$?
echo "NATIVE_GCC_RC=$GRC"
[ -x "$WORK/cup8bench" ] || { echo "★ cup8bench did not build"; echo "=== W311 NATIVE EXIT rc=2 ==="; exit 2; }
echo "binary md5 = $(md5sum < "$WORK/cup8bench" | cut -d' ' -f1)"

# ---- ★★★★★ THE KNOWN-POSITIVE, RUN FIRST -------------------------------------------
# A `bad=0` from the measure arm means nothing unless the verifier is shown to be alive on
# THIS build, on THIS box, in THIS session. Launches skipped => bad MUST equal N*N per
# verified iteration.
echo ""
echo "=== ★★★★★ NEGATIVE CONTROL (BENCH_NOLAUNCH=1) — the verifier must FIRE ==="
( cd "$WORK" && BENCH_NOLAUNCH=1 BENCH_SIZES=256 BENCH_ITERS=3 BENCH_BATCH=2 \
  BENCH_BATCH_SWEEP=1,4 BENCH_BATCH_REPS=2 ./cup8bench ) \
  | grep -E '^BENCH_MODE|^ITER |^B256_BAD=|^BENCH_NOLAUNCH_TOTAL_BAD=|^BENCH_VERDICT|^BSWEEP '
NCRC=${PIPESTATUS[0]}
echo "NATIVE_NEGCTRL_RC=$NCRC   (0 = the verifier fired; ⊘ 3 = it is DEAD and every green below is vacuous)"

# ---- the measurement ------------------------------------------------------------------
echo ""
echo "=== ★★★★★ THE NATIVE MEASUREMENT ==="
START=$(date +%s)
( cd "$WORK" && BENCH_SIZES="$SIZES" BENCH_ITERS="$ITERS" BENCH_BATCH="$BATCH" \
  BENCH_BATCH_SWEEP="$SWEEP" BENCH_BATCH_REPS="$REPS" BENCH_CTX_FLAGS="$CTXF" BENCH_HOSTMEM="$HOSTMEM" ./cup8bench )
RC=$?
END=$(date +%s)
echo "NATIVE_RC=$RC"
echo "NATIVE_WALL_S=$(( END - START ))"

echo ""
echo "=== GPU STATE AFTER ==="
nvidia-smi --query-gpu=memory.used,utilization.gpu --format=csv,noheader
echo "=== W311 NATIVE EXIT rc=$RC at $(date -Is) ==="
exit "$RC"
