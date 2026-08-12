#!/usr/bin/env bash
# ★★★ inject_matrix_ctxcreate.sh — extend the §14.27 fault-injection ladder PAST cuInit.
#
# ## Why this exists
#
# `traces/real_ga106/cuinit_fault_injection_matrix.txt` measured exactly one entry point.
# Its own header says so: `Driver: cuinit_probe with NVPROBE_STOP=1 (cuInit only).`
# ⇒ "`0x20810108` is NOT load-bearing" is a statement about **cuInit**, and about nothing
# else. Our wall is in **cuCtxCreate**, which that matrix never ran.
#
# `cuinit_probe.c` has always had cuCtxCreate as stage 4; only the driver script stopped
# early. So this needs no new instrument — it needs the existing one not to be truncated.
# (Same shape as the recorder bug this rung's other half fixes.)
#
# ## ⊘ TWO BUGS IN `inject_matrix_1428.sh` THAT THIS SCRIPT DOES NOT INHERIT
#
#  1. It exports `NVTRACE_FILE`, but the interposer reads **`NVTRACE_OUT`**
#     (`cuda_ioctl_trace.c:219`). Its traces went to stderr and survived only via `2>&1`.
#  2. It greps `^cuInit\(0\)` and `head -1`, discarding the later stages it actually ran.
#
# ## ★★★ THE GATE THAT MAKES A GREEN ROW MEAN ANYTHING
#
# The injector can only rewrite the status of an ioctl that **actually happens**. If libcuda
# never issues the targeted id, no `INJECT` line is printed, the run is simply a baseline,
# and "cuCtxCreate -> 0" is VACUOUS — it measures nothing about that id.
# ⇒ every row below records `INJECT_FIRED=<n>`, and a row with `INJECT_FIRED=0` is reported
# as **UNMEASURED**, never as "not load-bearing". `[[a-census-zero-needs-a-known-positive]]`
#
# ## ⊘ STRUCTURAL LIMITS — both stated, neither fixable here
#
#  - The injector can only turn `NV_OK` into an error. It CANNOT produce a served-but-wrong
#    answer, which is the failure mode our port is actually suspected of.
#  - It runs on hardware that works. A negative here means "this refusal alone does not break
#    a HEALTHY stack" — it does not clear the id in combination with our other divergences.
#
#   usage: bash inject_matrix_ctxcreate.sh [outfile]
set -u
OUT=${1:-/workspace/bench/ctxcreate_fault_injection_matrix.txt}
SRC=${RPCTRACE_SRC:-/workspace/kayfabe_w274/scripts/rpctrace}
WORK=${WORK_DIR:-/workspace/bench/inject_ctx}

mkdir -p "$WORK" || exit 90
cd "$WORK" || exit 90

echo "STARTED $(date -Is)" > "$WORK/.marker"

{
  echo "# cuCtxCreate fault-injection matrix — real GA106, real libcuda, no QEMU"
  echo "# date: $(date -Is)"
  echo "# host: $(hostname)  driver: $(sed -n 's/.*Module for x86_64 *\([0-9.]*\).*/\1/p' /proc/driver/nvidia/version 2>/dev/null | head -1)"
  echo "# gpu:  $(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)"
  echo "# driver program: cuinit_probe, FULL ladder (NVPROBE_STOP unset => stages 1..5)"
  echo "#   stage1 cuInit  stage2 cuDeviceGetCount  stage3 cuDeviceGet  stage4 cuCtxCreate  stage5 cuCtxDestroy"
  echo "# ⊘ a row with INJECT_FIRED=0 is UNMEASURED for that id, not evidence the id is inert."
  echo
} > "$OUT"

echo "== build ==" | tee -a "$OUT"
cc -shared -fPIC -O2 -o "$WORK/cuda_ioctl_trace.so" "$SRC/cuda_ioctl_trace.c" -ldl 2>&1 | tail -5
[ -f "$WORK/cuda_ioctl_trace.so" ] || { echo "FATAL: interposer did not build" | tee -a "$OUT"; exit 91; }
cc -O2 -o "$WORK/cuinit_probe" "$SRC/cuinit_probe.c" -ldl 2>&1 | tail -5
[ -x "$WORK/cuinit_probe" ] || { echo "FATAL: probe did not build" | tee -a "$OUT"; exit 92; }
# ⚠ ENOSPC is a false green; check it from this same invocation.
echo "   disk: $(df -h /workspace | tail -1)" | tee -a "$OUT"
echo "   built ok" | tee -a "$OUT"
echo | tee -a "$OUT"

# run <label> <NVFAULT_CTRL value or "-">
run_row() {
    label=$1; ctrl=$2
    log="$WORK/${label}.log"
    rm -f "$log"
    if [ "$ctrl" = "-" ]; then
        env -u NVFAULT_CTRL -u NVFAULT_ALLOC -u NVFAULT_STATUS \
            NVTRACE_OUT="$WORK/${label}.trace" \
            LD_PRELOAD="$WORK/cuda_ioctl_trace.so" \
            timeout 300 "$WORK/cuinit_probe" > "$log" 2>&1
    else
        env NVFAULT_CTRL="$ctrl" NVFAULT_STATUS=0x56 \
            NVTRACE_OUT="$WORK/${label}.trace" \
            LD_PRELOAD="$WORK/cuda_ioctl_trace.so" \
            timeout 300 "$WORK/cuinit_probe" > "$log" 2>&1
    fi
    rc=$?
    fired=$(grep -c 'INJECT CTRL' "$log" "$WORK/${label}.trace" 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
    init=$(grep -aoE '^cuInit\(0\) -> -?[0-9]+' "$log" | head -1 | grep -oE -- '-?[0-9]+$')
    ctx=$(grep -aoE '^cuCtxCreate\(0, [0-9]+\) -> -?[0-9]+' "$log" | head -1 | grep -oE -- '-?[0-9]+$')
    {
      echo "== ROW $label"
      echo "   NVFAULT_CTRL   = $ctrl   NVFAULT_STATUS = 0x56"
      echo "   INJECT_FIRED   = ${fired:-0}   $( [ "${fired:-0}" -eq 0 ] && [ "$ctrl" != "-" ] && echo '⊘ UNMEASURED — the id was never issued, so this row says NOTHING about it')"
      echo "   cuInit(0)      = ${init:-<no line>}"
      echo "   cuCtxCreate    = ${ctx:-<no line — did not reach stage 4>}"
      echo "   probe rc       = $rc"
      echo "   INJECT lines:"
      grep -ah 'INJECT CTRL' "$log" "$WORK/${label}.trace" 2>/dev/null | sort -u | sed 's/^/     /' | head -10
      echo "   stage markers:"
      grep -aE '^(stage|cuInit|cuDevice|cuCtx)' "$log" | sed 's/^/     /' | head -20
      echo
    } >> "$OUT"
    echo "   $label: fired=${fired:-0} cuInit=${init:-?} cuCtxCreate=${ctx:-?} rc=$rc"
}

echo "== rows ==" | tee -a "$OUT"
run_row baseline            -
run_row f_ctrl_20810108     0x20810108
run_row f_ctrl_2080200a     0x2080200a
run_row f_ctrl_20800a9a     0x20800a9a
run_row f_ctrl_all_three    0x20810108,0x2080200a,0x20800a9a

echo "EXIT_STATUS=0 $(date -Is)" >> "$WORK/.marker"
echo "=== MATRIX DONE -> $OUT ==="
