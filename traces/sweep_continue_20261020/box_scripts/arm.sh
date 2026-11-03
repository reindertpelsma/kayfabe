#!/bin/bash
# One owned frozen-parent arm. All observations are exported before cleanup.
set -euo pipefail
. /root/sweep2/common.sh
profile=${1:?on or off}; run=${2:?unique run}; binary=${3:?private binary}; source=${4:?source hash}
[[ $profile = on || $profile = off ]]
[[ $run =~ ^[a-z0-9_]{1,40}$ && $source =~ ^[0-9a-f]{40}$ ]]
[ -x "$binary" ]
exec >"/root/prov/arm_${run}.log" 2>&1
QLOG=$VM/logs/${run}_b1_qemu.log
launched=0
owned_pid=
cleanup(){
 rc=$?; trap - EXIT; set +e
 echo "PRE_CLEANUP_START rc=$rc $(date -u -Is)"
 if [ "$launched" = 1 ] && [ -f "$QLOG" ]; then
   python3 -I /root/sweep2/export.py "$QLOG" "/root/prov/arms/$run"
   python3 -I /root/sweep2/export_state.py "$run"
 fi
 [ "$launched" = 1 ] || { echo "ARM_GUARD_EXIT rc=$rc (no VM started)"; exit "$rc"; }
 date -u -Is >"/root/prov/arm_${run}_cleanup_started.txt"
 if [ -n "$owned_pid" ] && [ ! -e "/proc/$owned_pid/exe" ]; then
 echo OWN_VM_ALREADY_EXITED
 elif [ -f "$VM/run/qemu.pid" ] && [ "$(cat "$VM/run/qemu.pid")" = "$owned_pid" ]; then
 timeout 300 bash "$W" stop
 else echo OWN_VM_PID_CHANGED_STOP; rc=17; fi
 if [ -n "$owned_pid" ]; then
   for i in $(seq 1 60); do [ ! -e "/proc/$owned_pid/exe" ] && break; sleep 1; done
   [ ! -e "/proc/$owned_pid/exe" ] || { echo OWN_VM_STILL_ALIVE_STOP; rc=15; }
 fi
 sha256sum -c /root/prov/bwr_parent_sha256.txt || rc=16
 echo "ARM_EXIT run=$run rc=$rc $(date -u -Is)"
 exit "$rc"
}
trap cleanup EXIT
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = "$BASE" ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ ! -e "/root/prov/arm_${run}.started" ]
grep -q '^BIND_STOPPED_READY ' /root/prov/bwr_bind.log
sha256sum -c /root/prov/bwr_parent_sha256.txt
python3 - "$profile" "$run" "$source" "$binary" <<'PY'
import pathlib,json,datetime,sys,hashlib
p=pathlib.Path('/root/sweep2/lease.json');r=json.loads(p.read_text());now=datetime.datetime.now(datetime.timezone.utc);created=datetime.datetime.fromisoformat(r['created_utc']);end=created+datetime.timedelta(minutes=r['auto_destroy_minutes']-25);assert now<end,'latest-start guard: leave25min for observation/export/teardown'
counts=pathlib.Path('/root/prov/arm_counts.json');j=json.loads(counts.read_text()) if counts.exists() else {'on':0,'off':0};assert j[sys.argv[1]]<2,'max2 per redcell/lease';j[sys.argv[1]]+=1;counts.write_text(json.dumps(j,indent=2)+'\n')
out={'utc':now.isoformat(),'own_id':r['id'],'profile':sys.argv[1],'run':sys.argv[2],'source':sys.argv[3],'binary':sys.argv[4],'binary_sha256':hashlib.sha256(pathlib.Path(sys.argv[4]).read_bytes()).hexdigest(),'cell_attempt':j[sys.argv[1]],'latest_start_utc':end.isoformat()};pathlib.Path('/root/prov/arm_'+sys.argv[2]+'_start_receipt.json').write_text(json.dumps(out,indent=2)+'\n')
PY
date -u -Is >"/root/prov/arm_${run}.started"
parent=$VM/disk/bw580_bind.qcow2; child=$VM/disk/$run.qcow2
[ -f "$parent" ] && [ ! -e "$child" ]
qemu-img create -q -f qcow2 -F qcow2 -b "$parent" "$child"
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/sweep2/p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1 KF3_COMPLETION_PROBE=1000 KF3_PT_STALL_SNAPSHOT=1 KF3_TCENSUS=1
export WINVM_QEMU=$binary
extra=(); [ "$profile" = on ] || extra=(--kf3-extra display=off,gop=off,gop-efi=)
echo "ARM_START run=$run source=$source profile=$profile $(date -u -Is)"
bash "$W" run --overlay "$run" --arm B --guest-driver 580.65.06 --once --max-boots 1 --tag "$run" "${extra[@]}" --detach
launched=1
owned_pid=$(cat "$VM/run/qemu.pid")
for i in $(seq 1 90); do timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null 2>/dev/null | grep -q KF_SSH_UP && break; sleep 5; done
timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null | grep KF_SSH_UP
python3 /root/sweep2/bind_receipt.py "arm_${run}" "$binary" "$profile"
for f in verify_stage580.ps1 enable580_once.ps1 diag_min.ps1; do bash "$W" scp-to "/root/sweep2/probes/$f" "C:/kf/$f"; done
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\verify_stage580.ps1' </dev/null >"/root/prov/arm_${run}_pre_enable.txt" 2>&1
grep '^STAGE580_READY=1 ' "/root/prov/arm_${run}_pre_enable.txt"
if grep -aq 'kf-rm: rpc-trace' "$QLOG"; then echo PRECONDITION_REFUSED_GSP_WHILE_DISABLED; exit 7; fi
timeout 600 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\enable580_once.ps1' </dev/null >"/root/prov/arm_${run}_enable.txt" 2>&1
cat "/root/prov/arm_${run}_enable.txt"
# Read actual PnP/CIM/System state now, before the known40second crash window.
# This observation delay is not a product/driver timeout or TDR policy change.
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\diag_min.ps1' </dev/null >"/root/prov/arm_${run}_diag.txt" 2>&1
cat "/root/prov/arm_${run}_diag.txt"
if grep -q '"code":0' "/root/prov/arm_${run}_diag.txt"; then
 start_seconds=$SECONDS
 while [ $((SECONDS-start_seconds)) -lt 421 ]; do
  if grep -q GUEST_PANICKED "$VM/logs/${run}_run.log" 2>/dev/null; then echo ARM_GUEST_PANICKED; break; fi
  if [ -f "$VM/run/qemu.pid" ] && [ ! -e "/proc/$(cat "$VM/run/qemu.pid")/exe" ]; then echo ARM_VM_EXITED_DURING_HOLD; break; fi
  sleep 5
 done
 echo "ARM_HOLD_OBSERVED seconds=$((SECONDS-start_seconds)) $(date -u -Is)"
fi
if ! grep -q GUEST_PANICKED "$VM/logs/${run}_run.log" 2>/dev/null && pgrep -x qemu-system-x86 >/dev/null; then
 timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\diag_min.ps1' </dev/null >"/root/prov/arm_${run}_diag_after.txt" 2>&1 || true
 bash "$W" shot "$run" || true
fi
echo "ARM_OBSERVATION_DONE $(date -u -Is)"
