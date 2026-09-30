#!/usr/bin/env bash
# ★★★ POST_CAPTURE_HOOK — managed memory in a kf3 guest (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md`
# §8, E-M2). Runs `um_probe <mode>` for each mode, one process each, in the booted guest; after each
# one records the guest's NVRM/Xid lines and the HOST's new Xid lines. Every mode checks every value
# it computed (`traces/v3_uvm_research/um_probe.cu`): `CHECK <mode> ok bad=0` is the only pass.
#
#   malloc    : the control — no managed memory, no fault (must pass with the plane on AND off).
#   gpufirst  : cudaMallocManaged, first touched BY THE GPU — needs replayable-fault delivery.
#   cpuinit   : CPU writes first, then the GPU reads/writes — needs delivery (GPU faults on each page).
#   prefetch  : cudaMemPrefetchAsync before the kernel — no demand fault needed.
#   advise    : SetAccessedBy — mapping without migration.
#
# The binary is built on the host (`nvcc -O2 -arch=sm_86 -cudart static`) and copied in; set
# UM_PROBE_BIN to override, UM_PROBE_MODES to choose modes.
#   usage: KF_DEVICE=kf3 KF3_UVM_EFS=1 POST_CAPTURE_HOOK=scripts/bench/uvm_guest_hook.sh boot_capture.sh <tag>
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/gssh_nv"
BIN=${UM_PROBE_BIN:-/workspace/bench/um_probe}
MODES=${UM_PROBE_MODES:-attrs malloc gpufirst cpuinit prefetch advise malloc}
echo "UVMG_HOOK_START $(date -Is) tag=${1:-?} rev=$(git -C "$HERE/../.." rev-parse --short=8 HEAD) bin_md5=$(md5sum < "$BIN" | cut -d' ' -f1) KF3_UVM_EFS=${KF3_UVM_EFS:-unset}"
[ -x "$BIN" ] || { echo "UVMG_HOOK_NOTRUN no probe binary at $BIN"; exit 2; }
scp -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    "$BIN" ubuntu@192.168.77.2:/tmp/um_probe || { echo "UVMG_HOOK_NOTRUN scp failed"; exit 2; }
echo "--- host nvidia-uvm EFS parameters:"
for p in uvm_efs_enable uvm_efs_timeout_ms uvm_efs_max_records; do
    printf '%s=%s ' "$p" "$(cat /sys/module/nvidia_uvm/parameters/$p 2>/dev/null || echo ABSENT)"
done; echo
hx0=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
for m in $MODES; do
    echo "=== mode $m $(date -Is)"
    gx0=$($G 'sudo dmesg | grep -c "NVRM: Xid"' 2>/dev/null)
    $G "cd /tmp && timeout -k 5 ${UM_PROBE_TIMEOUT:-120} ./um_probe $m; echo UM_RC=\$?" 2>&1
    echo "--- guest NVRM/Xid/uvm lines new in this mode:"
    $G "sudo dmesg | grep -E 'NVRM: Xid|nvidia-uvm|UVM' | tail -n +$(( ${gx0:-0} + 1 ))" 2>&1 | tail -8
    echo "--- guest UVM fault counters:"
    $G "for f in /proc/driver/nvidia-uvm/gpus/*/fault_stats /proc/driver/nvidia-uvm/gpus/*/replayable_faults; do [ -r \$f ] && { echo \"# \$f\"; sudo cat \$f | head -30; }; done" 2>&1 | head -40
    echo "--- HOST Xid lines new in this mode:"
    dmesg 2>/dev/null | grep 'NVRM: Xid' | tail -n +$(( hx0 + 1 )) | tail -5
    hx0=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
done
echo "UVMG_HOOK_DONE $(date -Is)"
