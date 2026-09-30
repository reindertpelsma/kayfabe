#!/usr/bin/env bash
# ★★ POST_CAPTURE_HOOK — the four managed-memory apps of the V3 app matrix in a kf3 guest
# (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §8 E-A), one process each, with `run_apps.sh`'s own
# verdict rule: PASS = rc 0 AND the app's pass line AND no "CHECK .* FAIL". Built on the host by
# `uvm_apps_build.sh` into /workspace/bench/uvmapps and copied in. After each app: the guest's and
# the HOST's new Xid lines.
#   usage: KF_DEVICE=kf3 KF3_UVM_EFS=1 POST_CAPTURE_HOOK=scripts/bench/uvm_apps_hook.sh boot_capture.sh <tag>
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/gssh_nv"
SRC=${UVM_APPS_DIR:-/workspace/bench/uvmapps}
APPS=${UVM_APPS:-attach_verify UnifiedMemoryStreams conjugateGradientUM UnifiedMemoryPerf}
echo "UVMAPPS_HOOK_START $(date -Is) tag=${1:-?} rev=$(git -C "$HERE/../.." rev-parse --short=8 HEAD) KF3_UVM_EFS=${KF3_UVM_EFS:-unset}"
[ -d "$SRC" ] || { echo "UVMAPPS_HOOK_NOTRUN no $SRC"; exit 2; }
tar -C "$SRC" -czf /tmp/uvmapps.tgz . || { echo "UVMAPPS_HOOK_NOTRUN tar"; exit 2; }
scp -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    /tmp/uvmapps.tgz ubuntu@192.168.77.2:/tmp/uvmapps.tgz || { echo "UVMAPPS_HOOK_NOTRUN scp failed"; exit 2; }
$G 'rm -rf /tmp/uvmapps && mkdir -p /tmp/uvmapps && tar -C /tmp/uvmapps -xzf /tmp/uvmapps.tgz && ls /tmp/uvmapps | tr "\n" " "'; echo
hx0=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
for a in $APPS; do
  case $a in
    UnifiedMemoryStreams) to=120; re='All Done';;
    UnifiedMemoryPerf)    to=300; re='^16384';;
    conjugateGradientUM)  to=120; re='Test Summary:  Error amount = 0';;
    attach_verify)        to=90;  re='RESULT: CORRECT';;
    *) echo "UVMAPP app=$a verdict=NOTRUN note=unknown"; continue;;
  esac
  echo "=== app $a $(date -Is)"
  gx0=$($G 'sudo dmesg | grep -c "NVRM: Xid"' 2>/dev/null)
  t0=$(date +%s)
  out=$($G "cd /tmp/uvmapps && LD_LIBRARY_PATH=/tmp/uvmapps/lib timeout -k 5 $to ./$a; echo UVMAPP_RC=\$?" 2>&1)
  t1=$(date +%s)
  echo "$out" | tail -40
  rc=$(echo "$out" | sed -n 's/^UVMAPP_RC=//p' | tail -1)
  v=FAIL
  if [ "${rc:-x}" = 0 ] && echo "$out" | grep -qE "$re" && ! echo "$out" | grep -qE 'CHECK .* FAIL'; then v=PASS; fi
  [ "${rc:-x}" = 124 ] && v=TIMEOUT
  echo "UVMAPP app=$a verdict=$v rc=${rc:-?} secs=$((t1-t0))"
  echo "--- guest NVRM Xid lines new in this app:"
  $G "sudo dmesg | grep 'NVRM: Xid' | tail -n +$(( ${gx0:-0} + 1 ))" 2>&1 | tail -5
  echo "--- HOST Xid lines new in this app:"
  dmesg 2>/dev/null | grep 'NVRM: Xid' | tail -n +$(( hx0 + 1 )) | tail -5
  hx0=$(dmesg 2>/dev/null | grep -c 'NVRM: Xid')
done
echo "UVMAPPS_HOOK_DONE $(date -Is)"
