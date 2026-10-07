#!/usr/bin/env bash
# ★★★ POST_CAPTURE_HOOK — run probe binaries in the booted guest, one process per mode, and record
# the guest's and the HOST's new Xid lines after each (v3-roperm; generalises readmostly_hook.sh).
#
#   PROBES="<host-path>:<mode>,<mode> <host-path>:"   a binary with no modes runs once, no argument
#   usage: KF_DEVICE=kf3 PROBES=... POST_CAPTURE_HOOK=scripts/bench/probe_hook.sh boot_capture.sh <tag>
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/gssh_nv"
SCP="scp -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
echo "PROBE_HOOK_START $(date -Is) tag=${1:-?} rev=$(git -C "$HERE/../.." rev-parse --short=8 HEAD)"
[ -n "${PROBES:-}" ] || { echo "PROBE_HOOK_NOTRUN PROBES is empty"; exit 2; }
hx0=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
for spec in $PROBES; do
    bin=${spec%%:*}; modes=${spec#*:}; name=$(basename "$bin")
    [ -x "$bin" ] || { echo "PROBE_HOOK_NOTRUN no binary $bin"; continue; }
    echo "=== probe $name md5=$(md5sum < "$bin" | cut -d' ' -f1)"
    $SCP "$bin" "ubuntu@192.168.77.2:/tmp/$name" || { echo "PROBE_HOOK_NOTRUN scp $name failed"; continue; }
    for m in $(echo "${modes:-_}" | tr ',' ' '); do
        arg=$m; [ "$m" = _ ] && arg=""
        echo "=== run $name ${arg:-(no args)} $(date -Is)"
        gx0=$($G 'sudo dmesg | grep -c "NVRM: Xid"' 2>/dev/null)
        $G "cd /tmp && timeout -k 5 180 ./$name $arg 2>&1 | tail -25; echo PROBE_RC=\${PIPESTATUS[0]}" 2>&1
        echo "--- guest NVRM Xid lines new in this run:"
        $G "sudo dmesg | grep 'NVRM: Xid' | tail -n +$(( ${gx0:-0} + 1 ))" 2>&1 | tail -5
        echo "--- HOST Xid lines new in this run:"
        dmesg 2>/dev/null | grep 'NVRM: Xid' | tail -n +$(( hx0 + 1 )) | tail -5
        hx0=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
    done
done
echo "PROBE_HOOK_DONE $(date -Is)"
