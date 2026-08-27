#!/usr/bin/env bash
# m563_phaseA_run_host.sh — ON HOST. Verify Phase A (M5.60): the user-CE copy
# DEST becomes a REAL host vidmem object (not a fake fb_page) when we re-walk
# the compute VAS at LAUNCH_DMA. Boots the open-580 guest with NVKVM_M2CEFWD=1,
# runs cup6 (64MB HtoD x3), and samples host nvidia-smi memory.used THROUGHOUT
# to capture the peak. SUCCESS = peak jumps well above the ~19MB pre-Phase-A
# baseline (toward ~64MB) AND cup6 stays byte-exact (rc=0), no Xid.
set -u
PK=/tmp/m563_peak.csv; : > "$PK"
# Background peak sampler (read-only nvidia-smi is safe alongside guest work).
( for i in $(seq 1 800); do
    nvidia-smi --query-gpu=memory.used,utilization.gpu --format=csv,noheader,nounits 2>/dev/null >> "$PK"
    sleep 0.5
  done ) &
SAMP=$!
trap 'kill $SAMP 2>/dev/null' EXIT

export NVKVM_M2CEFWD=1
export CUP6_MB=${CUP6_MB:-64}
bash /workspace/nvkvm/scripts/mode2_diag/m562_placement_probe_host.sh 2>&1

kill $SAMP 2>/dev/null
echo ""
echo "============ M563 PHASE-A VERIFY ============"
echo "--- host memory.used peak during run (MiB, util%) ---"
sort -t, -k1 -n "$PK" | tail -1
echo "  (baseline pre-Phase-A was ~19 MiB peak; expect a clear jump if dst real-backed)"
echo "--- M5.60 re-walk fired? (QEMU log) ---"
grep -c "M5.60 user-CE dst un-backed" /tmp/m0_qemu.log 2>/dev/null | sed 's/^/  M5.60 events: /'
grep "M5.60 user-CE dst un-backed" /tmp/m0_qemu.log 2>/dev/null | head -4
echo "--- gpga_obj backings after re-walk (M7 R2) ---"
grep -c "M7 R2 gpga_obj:" /tmp/m0_qemu.log 2>/dev/null | sed 's/^/  gpga_obj count: /'
echo "--- DPLANE dest verdict tally (post-Phase-A) ---"
grep -oE "verdict=[a-z]+" /tmp/m0_qemu.log 2>/dev/null | sort | uniq -c
echo "--- any Xid / fault (must be NONE) ---"
grep -iE "Xid|FAULT_PTE|fault" /tmp/m0_qemu.log 2>/dev/null | tail -5
echo "============ END ============"
