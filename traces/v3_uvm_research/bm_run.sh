#!/usr/bin/env bash
# Bare-metal baseline for docs/design/V3_UVM_DEMAND_PAGING.md (no guest, no kayfabe).
# For each workload: the host UVM's replayable-fault counters BEFORE and AFTER, so a PASS
# can be read as "needed demand paging" or "did not". Needs nvidia-uvm loaded with
# uvm_enable_debug_procfs=1 (fault_stats lives under the debug procfs).
# usage: bm_run.sh <hmm:0|1>   (1 = stock, 0 = reload nvidia-uvm with uvm_disable_hmm=1)
set -uo pipefail
HMM=${1:-1}
D=$(cd "$(dirname "$0")" && pwd)
echo "BM_START $(date -Is) hmm=$HMM"
head -1 /proc/driver/nvidia/version
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
echo "uvm_disable_hmm=$(cat /sys/module/nvidia_uvm/parameters/uvm_disable_hmm 2>/dev/null) debug_procfs=$(cat /sys/module/nvidia_uvm/parameters/uvm_enable_debug_procfs 2>/dev/null)"
FS=$(find /proc/driver/nvidia-uvm -name fault_stats 2>/dev/null | head -1)
[ -n "$FS" ] || { echo "NO_FAULT_STATS (need uvm_enable_debug_procfs=1)"; }
snap() { [ -n "$FS" ] && tr -s ' ' < "$FS" | tr '\n' ' '; }
run() {
  local name=$1; shift
  echo "=== $name"
  echo "BEFORE $(snap)"
  timeout 600 "$@" 2>&1 | tail -40
  echo "RC=${PIPESTATUS[0]}"
  echo "AFTER  $(snap)"
}
for m in malloc cpuinit prefetch gpufirst advise pageable hostalloc d2h; do run "um_probe $m" "$D/um_probe" $m; done
for m in reprefetch fault gpuwrite downgrade; do run "readmostly_probe $m" "$D/readmostly_probe" $m; done
command -v clpeak >/dev/null && run clpeak clpeak
echo "BM_EXIT rc=0 $(date -Is)"
