#!/usr/bin/env bash
# ★★★★★ w322 — THE NATIVE APERTURE ENDPOINTS, and the pessimality check on w320's control.
#
#   usage (on vh2):  scripts/bench/w322_native.sh
#   writes:          /workspace/w322/native_<mode>.log  and  /workspace/w322/native_summary.log
#
# ## Why this runs FIRST, before any boot
#
# The brief's cheapest question is *"was w320's host-memory control pessimal?"*, and if it was,
# the whole `guest ÷ hostmem` comparison it licensed is soft. That question needs no guest, no
# QEMU and no emulated GPU — only this GPU and libcuda — so it is answered before a single
# boot is spent.
#
# ## What each arm is FOR. ⊘ None of them is "our guest"; they are the RULER.
#
#   vram          the fast endpoint. Whatever `bw` reports here IS this GPU's VRAM read
#                 bandwidth for THIS kernel, measured rather than quoted from a datasheet.
#   hostalloc     w320's control, unchanged — `cuMemHostAlloc(DEVICEMAP)`, cacheable pages.
#   hostalloc_wc  + WRITECOMBINED. Expected WORSE for a read-only kernel; it is here as a
#                 DIRECTIONAL known-positive for the instrument (if WC does not come out
#                 slower than plain hostalloc, the sweep is not resolving placement at all).
#   hostreg       2 MiB-aligned anonymous memory + MADV_HUGEPAGE + cuMemHostRegister. The
#                 PAGE-SIZE arm.
#   managed_cpu   cuMemAllocManaged pinned to the CPU by advice. Sysmem reached through UVM's
#                 mappings rather than cuMemHostAlloc's.
#
# ★★★ THE READING. If `hostalloc`, `hostreg` and `managed_cpu` all land within a small factor
# of each other, then "pinned sysmem over PCIe" is a NARROW band and w320's control was
# representative. If one of them is much faster, the control was PESSIMAL and w320's
# "our guest beats sysmem, so it must be something else" inference loses its force.
# ⊘ Either way this arm cannot say where OUR buffers are. It calibrates the ruler.
set -uo pipefail
WORK=${W322_DIR:-/workspace/w322}
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
MODES=${W322_MODES:-vram hostalloc hostalloc_wc hostreg managed_cpu}
BW=${W322_BW:-1,4,16,64,256}
BWTGT=${W322_BW_TARGET_MIB:-256}
BWIT=${W322_BW_ITERS:-7}
SUM="$WORK/native_summary.log"

mkdir -p "$WORK"
: >"$SUM"
{
echo "=== ★★★★★ W322 NATIVE START $(date -Is) pid=$$ ==="
echo "modes=[$MODES] bw=[$BW] target_mib=$BWTGT iters=$BWIT"
echo "src md5 = $(md5sum < "$SRC_DIR/cup8bench.c" | cut -d' ' -f1)"

# ★★ The QEMU guard is inside w311_native.sh (it uses `pgrep -x qemu-system-x86`, the 15-char
#    comm-truncation trap). It is checked HERE TOO because this script runs a LOOP: a QEMU
#    that comes up between arms would contend with the later ones only, producing one slow
#    mode that reads as a placement effect.
if pgrep -x qemu-system-x86 >/dev/null 2>&1; then
  echo "★★★ REFUSING: a QEMU is running. Every number would be CONTENDED."
  echo "=== W322 NATIVE EXIT rc=64 ==="; exit 64
fi
nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv
} | tee -a "$SUM"

RC_ALL=0
for M in $MODES; do
  L="$WORK/native_$M.log"
  echo "" | tee -a "$SUM"
  echo "=== ★ ARM alloc=$M -> $L  $(date -Is)" | tee -a "$SUM"
  # ⊘ BENCH_BW_ONLY=1: the matmul phase is SKIPPED in these arms. It is not the measurement
  #   here and at `managed_cpu` / N=2048 it would cost minutes for a number w320 already has.
  W311_NATIVE_LOG="$L" W311_NATIVE_DIR="$WORK/build" \
  BENCH_ALLOC="$M" BENCH_BW="$BW" BENCH_BW_TARGET_MIB="$BWTGT" BENCH_BW_ITERS="$BWIT" \
  BENCH_BW_ONLY=1 BENCH_SIZES=256 BENCH_ITERS=3 BENCH_BATCH=2 \
    "$SRC_DIR/w311_native.sh"
  R=$?
  [ "$R" -eq 0 ] || RC_ALL=$R
  {
    echo "    arm_rc=$R"
    # ⊘ Print the ROW lines whatever they say — including UNMEASURED ones. A grep that
    #   selected only successful rows would make a refused mode look like an absent mode.
    grep -hE '^BWROW |^BENCH_BW_MODULE_RC=|^BENCH_BW_FUNC_RC=|^BW_ALLOC_FAIL |^BW_ADVISE |^BENCH_BW_ROWS' "$L" 2>/dev/null \
      | sed 's/^/    /' || echo "    ⊘ NO BW LINES AT ALL — UNMEASURED, not zero."
  } | tee -a "$SUM"
done

{
echo ""
echo "=== ★★★★★ THE RULER — read_GBps by mode and buffer size ==="
echo "    (⊘ small sizes are the L2 plateau and are the SAME whatever the backing is;"
echo "     the LARGE end is the aperture. Compare across modes at the largest size.)"
printf "    %-14s" "mib"; for M in $MODES; do printf "%14s" "$M"; done; echo ""
for S in ${BW//,/ }; do
  printf "    %-14s" "$S"
  for M in $MODES; do
    V=$(grep -h "^BWROW mib=$S " "$WORK/native_$M.log" 2>/dev/null \
        | sed -n 's/.* read_GBps=\([0-9.]*\) .*/\1/p' | tail -1)
    printf "%14s" "${V:-UNMEAS}"
  done
  echo ""
done
echo ""
echo "=== ★ correctness of every row (bad MUST be 0 in every measured row) ==="
grep -hE '^BWROW ' "$WORK"/native_*.log 2>/dev/null | sed -n 's/.*mib=\([0-9]*\) alloc=\([a-z_]*\).* bad=\([0-9]*\)$/    mib=\1 alloc=\2 bad=\3/p'
echo "=== W322 NATIVE DONE rc=$RC_ALL $(date -Is) ==="
} | tee -a "$SUM"
exit "$RC_ALL"
