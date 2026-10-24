#!/bin/bash
# B1n DIAGNOSTIC guest prep boot (owner 2026-10-24, arm on_tdr60): standard VGA, NO kf3 device,
# NO GPU work and no kayfabe flag. Applies one pinned guest registry script to the arm's own
# child overlay, then shuts the guest down. Bounded; the product boot follows separately.
set -euo pipefail
. /root/sweep2/common.sh
run=${1:?arm name}; prep=${2:?prep script}
[[ $run =~ ^on_[a-z0-9_]{1,37}$ ]]
[ "$prep" = prep_tdr60.ps1 ]
exec >"/root/prov/prep_${run}.log" 2>&1
launched=0; owned_pid=
cleanup(){
 rc=$?; trap - EXIT; set +e
 if [ "$launched" = 1 ] && [ -f "$VM/run/qemu.pid" ] &&
  [ "$(cat "$VM/run/qemu.pid")" = "$owned_pid" ]; then timeout 300 bash "$W" stop; fi
 # The launcher removes its pid file at boot end; the product boot must never read a stale pid.
 for i in $(seq 1 60); do [ ! -f "$VM/run/qemu.pid" ] && break; sleep 1; done
 [ ! -f "$VM/run/qemu.pid" ] || { echo PREP_PID_FILE_STALE; rc=44; }
 echo "PREP_EXIT run=$run rc=$rc $(date -u -Is)"; exit "$rc"
}
trap cleanup EXIT
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = "$BASE" ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ -f "$VM/disk/${run}.qcow2" ]
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
export WINVM_QEMU=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
echo "PREP_START run=$run script=$prep (standard VGA, no kf3 device) $(date -u -Is)"
bash "$W" run --overlay "$run" --no-kf3 --once --max-boots 1 --tag "${run}_prep" --detach
launched=1
start=$SECONDS
while [ ! -f "$VM/run/qemu.pid" ]; do [ $((SECONDS-start)) -lt 90 ]; sleep 1; done
owned_pid=$(cat "$VM/run/qemu.pid")
for i in $(seq 1 90); do
 [ -e "/proc/$owned_pid/exe" ] && [ $((SECONDS-start)) -lt 450 ]
 timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null 2>/dev/null | grep -q KF_SSH_UP && break
 sleep 5
done
bash "$W" scp-to "/root/sweep2/$prep" "C:/kf/$prep"
timeout 120 bash "$W" ssh "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\$prep" \
 </dev/null >"/root/prov/prep_${run}.txt" 2>&1
cat "/root/prov/prep_${run}.txt"
grep -q '^PREP_TDR60_DONE TdrDdiDelay=60 ' "/root/prov/prep_${run}.txt"
timeout 300 bash "$W" stop
for i in $(seq 1 60); do [ ! -e "/proc/$owned_pid/exe" ] && break; sleep 1; done
[ ! -e "/proc/$owned_pid/exe" ]
launched=0
echo "PREP_READY run=$run $(date -u -Is)"
