#!/usr/bin/env bash
# ★★★ POST_CAPTURE_HOOK — stock UVM READ DUPLICATION in the guest (v3-roperm; the probe and its
# predictions are `V3_UVM_DEMAND_PAGING.md` §6 / E3 on branch v3-uvm-research).
#
# Runs `readmostly_probe <mode>` for every mode, one process each, in the booted guest, and after
# each one records the guest's NVRM/Xid lines AND the HOST's new Xid lines (a read-only host twin
# that a GPU write hits faults on the HOST: that fault is the loud outcome this fix is for).
#
#   gpuwrite / downgrade : a GPU write to a read-only duplicate.
#       before the fix : `CHECK <m> FAIL bad=N`, rc=1, NO CUDA error, no Xid — the SILENT case.
#       after the fix  : `cudaDeviceSynchronize() -> 719` + a host Xid 31 write fault. Never bad=N.
#   reprefetch           : no demand fault needed — must pass with correct values.
#   fault                : needs a replayable fault after the collapse — 719 until fault delivery.
#
# The binary is built on the host (`nvcc -O2 -arch=sm_86 -cudart static`, traces/v3_roperm/
# readmostly_probe.cu) and copied in; set RM_PROBE_BIN to override.
#   usage: KF_DEVICE=kf3 POST_CAPTURE_HOOK=scripts/bench/readmostly_hook.sh boot_capture.sh <tag>
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/gssh_nv"
BIN=${RM_PROBE_BIN:-/workspace/bench/readmostly_probe}
MODES=${RM_PROBE_MODES:-reprefetch gpuwrite downgrade fault reprefetch}
echo "READMOSTLY_HOOK_START $(date -Is) tag=${1:-?} rev=$(git -C "$HERE/../.." rev-parse --short=8 HEAD) bin_md5=$(md5sum < "$BIN" | cut -d' ' -f1)"
[ -x "$BIN" ] || { echo "READMOSTLY_HOOK_NOTRUN no probe binary at $BIN"; exit 2; }
scp -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    "$BIN" ubuntu@192.168.77.2:/tmp/readmostly_probe || { echo "READMOSTLY_HOOK_NOTRUN scp failed"; exit 2; }
hx0=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
for m in $MODES; do
    echo "=== mode $m $(date -Is)"
    gx0=$($G 'sudo dmesg | grep -c "NVRM: Xid"' 2>/dev/null)
    $G "cd /tmp && timeout -k 5 120 ./readmostly_probe $m; echo RM_RC=\$?" 2>&1
    echo "--- guest NVRM Xid lines new in this mode:"
    $G "sudo dmesg | grep 'NVRM: Xid' | tail -n +$(( ${gx0:-0} + 1 ))" 2>&1 | tail -5
    echo "--- HOST Xid lines new in this mode:"
    dmesg 2>/dev/null | grep 'NVRM: Xid' | tail -n +$(( hx0 + 1 )) | tail -5
    hx0=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
done
echo "READMOSTLY_HOOK_DONE $(date -Is)"
