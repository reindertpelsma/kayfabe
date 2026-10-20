#!/bin/bash
# usage: collect.sh <run>   boot the arm's own child overlay WITHOUT kf3 and read the crash evidence
set -uo pipefail
. /root/bwr/common.sh
run=$1
exec >"/root/prov/collect_${run}.log" 2>&1
echo "COLLECT_START $run $(date -u -Is)"
cd /root/kayfabe
export WINVM_QEMU=/workspace/bench/kf3-bins/7f22d18d/qemu-system-x86_64
[ -z "$(pgrep -x qemu-system-x86 || true)" ] || { echo QEMU_RUNNING; exit 3; }
[ -f "$VM/disk/${run}.qcow2" ] || { echo NO_CHILD; exit 4; }
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
bash "$W" run --overlay "$run" --no-kf3 --once --max-boots 1 --tag "${run}_collect" --detach
for i in $(seq 1 90); do timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null 2>/dev/null | grep -q KF_SSH_UP && break; sleep 5; done
bash "$W" scp-to /root/bwr/collect_crash.ps1 C:/kf/collect_crash.ps1
timeout 180 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\collect_crash.ps1' </dev/null > "/root/prov/collect_${run}.txt" 2>&1
mkdir -p "/root/prov/dumps/$run"; bash "$W" scp-from 'C:/Windows/Minidump' "/root/prov/dumps/$run/" >/dev/null 2>&1 || true
find /root/prov/dumps/$run -type f -exec sha256sum {} \; > "/root/prov/collect_${run}_dump_hashes.txt"
timeout 200 bash "$W" stop
echo "COLLECT_EXIT $run $(date -u -Is)"
