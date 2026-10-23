#!/bin/bash
# One owned frozen-parent arm. All observations are exported before cleanup.
set -euo pipefail
. /root/s2/common.sh
profile=${1:?on or off}; run=${2:?unique run}; binary=${3:?private binary}; source=${4:?source hash}
[[ $profile = on || $profile = off ]]
[[ $run =~ ^[a-z0-9_]{1,40}$ && $source =~ ^[0-9a-f]{40}$ ]]
[ -x "$binary" ]
exec >"/root/prov/arm_${run}.log" 2>&1
QLOG=$VM/logs/${run}_b1_qemu.log
launched=0
observer_pid=
held_seconds=0
owned_pid=
cleanup(){
 rc=$?; trap - EXIT; set +e
 echo "PRE_CLEANUP_START rc=$rc $(date -u -Is)"
 if [ "$launched" = 1 ] && [ -f "$QLOG" ]; then
   timeout 90 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\collect_crash.ps1' </dev/null >"/root/prov/arm_${run}_live_events.txt" 2>&1
   python3 -I /root/s2/export.py "$QLOG" "/root/prov/arms/$run"
   python3 -I /root/s2/export_state.py "$run"
 fi
 [ "$launched" = 1 ] || { echo "ARM_GUARD_EXIT rc=$rc (no VM started)"; exit "$rc"; }
 date -u -Is >"/root/prov/arm_${run}_cleanup_started.txt"
 if [ -n "$owned_pid" ] && [ ! -e "/proc/$owned_pid/exe" ]; then
 echo OWN_VM_ALREADY_EXITED
 elif [ -f "$VM/run/qemu.pid" ] && [ "$(cat "$VM/run/qemu.pid")" = "$owned_pid" ]; then
 timeout 300 bash "$W" stop
 # Guest SSH plus powerdown can exhaust the wrapper before win_vm.sh reaches
 # its QMP quit. Always reach that documented fallback for this same owned VM.
 if [ -e "/proc/$owned_pid/exe" ] && [ -f "$VM/run/qemu.pid" ] && [ "$(cat "$VM/run/qemu.pid")" = "$owned_pid" ]; then
   echo OWN_VM_FALLBACK_QMP_QUIT
   timeout 20 python3 scripts/bench/windows/qmp.py "$VM/run/qmp.sock" cmd quit
 fi
 else echo OWN_VM_PID_CHANGED_STOP; rc=17; fi
 if [ -n "$owned_pid" ]; then
   for i in $(seq 1 60); do [ ! -e "/proc/$owned_pid/exe" ] && break; sleep 1; done
   [ ! -e "/proc/$owned_pid/exe" ] || { echo OWN_VM_STILL_ALIVE_STOP; rc=15; }
 fi
 if [ -n "$observer_pid" ]; then
  for i in $(seq 1 20); do kill -0 "$observer_pid" 2>/dev/null || break; sleep 1; done
  if kill -0 "$observer_pid" 2>/dev/null; then kill -TERM "$observer_pid"; fi
  wait "$observer_pid" || rc=18
 fi
 sha256sum -c /root/prov/bwr_parent_sha256.txt || rc=16
 echo "ARM_EXIT run=$run rc=$rc $(date -u -Is)"
 exit "$rc"
}
trap cleanup EXIT
cd /root/kayfabe
. scripts/bench/windows/readiness.sh
readiness_stage "/root/prov/arm_${run}_readiness_stage.json" 600 3600
[ "$(git rev-parse HEAD)" = "$BASE" ]
python3 /root/kayfabe/scripts/bench/diesweep/desktop/live_qemu.py --assert-none
[ ! -e "/root/prov/arm_${run}.started" ]
grep -q '^BIND_STOPPED_READY ' /root/prov/bwr_bind.log
sha256sum -c /root/prov/bwr_parent_sha256.txt
python3 - "$profile" "$run" "$source" "$binary" <<'PY'
import pathlib,json,datetime,sys,hashlib
p=pathlib.Path('/root/s2/lease.json');r=json.loads(p.read_text());now=datetime.datetime.now(datetime.timezone.utc);created=datetime.datetime.fromisoformat(r['created_utc']);end=created+datetime.timedelta(minutes=r['auto_destroy_minutes']-25);assert now<end,'latest-start guard: leave25min for observation/export/teardown'
counts=pathlib.Path('/root/prov/arm_counts.json');j=json.loads(counts.read_text()) if counts.exists() else {'on':0,'off':0};j[sys.argv[1]]+=1;counts.write_text(json.dumps(j,indent=2)+'\n')
out={'utc':now.isoformat(),'own_id':r['id'],'profile':sys.argv[1],'run':sys.argv[2],'source':sys.argv[3],'binary':sys.argv[4],'binary_sha256':hashlib.sha256(pathlib.Path(sys.argv[4]).read_bytes()).hexdigest(),'harness_sha256':hashlib.sha256(pathlib.Path('/root/s2/arm.sh').read_bytes()).hexdigest(),'rpc_observer_sha256':hashlib.sha256(pathlib.Path('/root/s2/rpc_timing.py').read_bytes()).hexdigest(),'cell_attempt':j[sys.argv[1]],'latest_start_utc':end.isoformat()};pathlib.Path('/root/prov/arm_'+sys.argv[2]+'_start_receipt.json').write_text(json.dumps(out,indent=2)+'\n')
PY
date -u -Is >"/root/prov/arm_${run}.started"
parent=$VM/disk/bw580_bind.qcow2; child=$VM/disk/$run.qcow2
[ -f "$parent" ] && [ ! -e "$child" ]
qemu-img create -q -f qcow2 -F qcow2 -b "$parent" "$child"
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/s2/p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1 KF3_DIAG_PROGRESS=1000
export WINVM_QEMU=$binary
extra=(); [ "$profile" = on ] || extra=(--kf3-extra display=off,gop=off,gop-efi=)
if [ "${S2_OBSERVER:-0}" = 1 ]; then
 export WINVM_TRACE=1 WINVM_GSP_OBSERVER=1 WINVM_GSP_OBSERVER_SECONDS=1800
 extra=(--kf3-extra doorbell-ioeventfd=off)
fi
echo "ARM_START run=$run source=$source profile=$profile $(date -u -Is)"
bash "$W" run --overlay "$run" --arm B --guest-driver 580.65.06 --once --max-boots 1 --tag "$run" "${extra[@]}" --detach
launched=1
# Detached launch is asynchronous. Wait for this harness's PID/log before observing it.
launch_wait=$SECONDS
while [ ! -f "$VM/run/qemu.pid" ] || [ ! -f "$QLOG" ]; do
 [ $((SECONDS-launch_wait)) -lt 90 ] || { echo ARM_LAUNCH_RECEIPT_TIMEOUT_STOP; exit 19; }
 sleep 1
done
owned_pid=$(cat "$VM/run/qemu.pid")
mkdir -p "/root/prov/arms/$run"
python3 -I /root/s2/rpc_timing.py "$QLOG" "/root/prov/arms/$run/rpc_timing.jsonl" "$owned_pid" >"/root/prov/arms/$run/rpc_timing_controller.log" 2>&1 &
observer_pid=$!
[ -e "/proc/$owned_pid/exe" ] || { echo ARM_QEMU_EXITED_BEFORE_ENABLE_STOP; exit 20; }
readiness_wait "arm_${run}_ssh" bash "$W" ssh 'echo KF_SSH_UP'
python3 /root/s2/bind_receipt.py "arm_${run}" "$binary" "$profile"
readiness_step once "arm_${run}_media_dir" 15 120 bash "$W" ssh 'New-Item -ItemType Directory -Force C:\kfmedia | Out-Null' </dev/null
readiness_step once "arm_${run}_app_copy" 180 1200 bash "$W" scp-to /workspace/bench/kfapps.iso C:/kfmedia/kfapps.iso
readiness_step once "arm_${run}_nvrtc_copy" 120 600 bash "$W" scp-to /workspace/bench/cuda_nvrtc.zip C:/kfmedia/cuda_nvrtc.zip
echo "APPDISK_GUEST_COPY_DONE $(date -u -Is)"
for f in verify_stage580.ps1 enable580_once.ps1 diag_min.ps1 collect_crash.ps1; do bash "$W" scp-to "/root/s2/probes/$f" "C:/kf/$f"; done
readiness_step query "arm_${run}_verify_stage580" 30 240 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\verify_stage580.ps1' </dev/null >"/root/prov/arm_${run}_pre_enable.txt" 2>&1
grep '^STAGE580_READY=1 ' "/root/prov/arm_${run}_pre_enable.txt"
if grep -aq 'kf-rm: rpc-trace' "$QLOG"; then echo PRECONDITION_REFUSED_GSP_WHILE_DISABLED; exit 7; fi
readiness_step once "arm_${run}_enable580_once" 150 900 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\enable580_once.ps1' </dev/null >"/root/prov/arm_${run}_enable.txt" 2>&1
cat "/root/prov/arm_${run}_enable.txt"
# Read actual PnP/CIM/System state now, before the known40second crash window.
# This observation delay is not a product/driver timeout or TDR policy change.
readiness_step query "arm_${run}_diag_min" 30 240 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\diag_min.ps1' </dev/null >"/root/prov/arm_${run}_diag.txt" 2>&1
cat "/root/prov/arm_${run}_diag.txt"
export KF_GUEST_USER=kf
if KF_GUEST_PW=$(cat "$VM/secrets/win_password") \
 WINVM_READINESS_STAGE="$READINESS_STAGE" \
 readiness_step once "arm_${run}_signin_once" 150 900 \
 python3 scripts/bench/windows/signin.py "$VM/run/qmp.sock" "$VM/run/qga.sock" \
 >"/root/prov/arm_${run}_signin.txt" 2>&1; then :; else
 timeout 90 bash "$W" ssh '[pscustomobject]@{utc=[DateTime]::UtcNow.ToString("o");user=(Get-CimInstance Win32_ComputerSystem).UserName;logon=@(Get-Process LogonUI -ErrorAction SilentlyContinue | Select-Object Id,SessionId,StartTime);explorer=@(Get-Process explorer -ErrorAction SilentlyContinue | Select-Object Id,SessionId,StartTime)} | ConvertTo-Json -Compress' </dev/null >"/root/prov/arm_${run}_signin_state.txt" 2>&1 || true
 exit 1
fi
grep '^SIGNIN_VERIFIED ' "/root/prov/arm_${run}_signin.txt"
bash "$W" shot "${run}_signed_in" || true
python3 -B scripts/bench/diesweep/desktop/matrix.py --vm-root "$VM" --qlog "$QLOG" --out "/root/prov/matrix_$run" --iso /workspace/bench/kfapps.iso --guest-iso --override-dir /root/s2/apps --manifest scripts/bench/windows/appmatrix/manifest.json --lease /root/s2/lease.json >"/root/prov/arm_${run}_matrix.log" 2>&1
echo "ARM_OBSERVATION_DONE $(date -u -Is)"
