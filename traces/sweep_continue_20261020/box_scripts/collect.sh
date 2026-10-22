#!/bin/bash
set -euo pipefail
. /root/sweep2/common.sh
run=${1:?run}; [[ $run =~ ^[a-z0-9_]{1,40}$ ]]
exec >"/root/prov/collect_${run}.log" 2>&1
cd /root/kayfabe
export WINVM_QEMU=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ -f "$VM/disk/${run}.qcow2" ]
launched=0; owned_pid=
cleanup(){
 rc=$?;trap - EXIT;set +e
 if [ "$launched" = 1 ] && [ -f "$VM/run/qemu.pid" ] && [ "$(cat "$VM/run/qemu.pid")" = "$owned_pid" ]; then timeout 300 bash "$W" stop; fi
 echo "COLLECT_EXIT run=$run rc=$rc $(date -u -Is)";exit "$rc"
}
trap cleanup EXIT
while IFS= read -r key;do unset "$key";done < <(compgen -A variable KF3_)
echo "COLLECT_START run=$run $(date -u -Is)"
bash "$W" run --overlay "$run" --no-kf3 --once --max-boots 1 --tag "${run}_collect" --detach
launched=1;owned_pid=$(cat "$VM/run/qemu.pid")
for i in $(seq 1 90);do timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null 2>/dev/null|grep -q KF_SSH_UP && break;sleep 5;done
bash "$W" scp-to /root/sweep2/collect_crash.ps1 C:/kf/collect_crash.ps1
timeout 180 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\collect_crash.ps1' </dev/null >"/root/prov/collect_${run}.txt" 2>&1
grep '^COLLECT_DONE' "/root/prov/collect_${run}.txt"
# All dump bytes stay on this disposable rental. Public-symbol analysis is a separate bounded step.
echo "COLLECT_METADATA_READY $(date -u -Is)"
