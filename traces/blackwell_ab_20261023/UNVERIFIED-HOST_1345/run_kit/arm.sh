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
observer_pid=
init_observer_pid=
jpid=
held_seconds=0
owned_pid=
cleanup(){
 rc=$?; trap - EXIT; set +e
 if [ -n "$init_observer_pid" ]; then
  kill -TERM "$init_observer_pid" 2>/dev/null || true
  wait "$init_observer_pid" 2>/dev/null || true
 fi
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
 if [ -n "$observer_pid" ]; then
  for i in $(seq 1 20); do kill -0 "$observer_pid" 2>/dev/null || break; sleep 1; done
  if kill -0 "$observer_pid" 2>/dev/null; then kill -TERM "$observer_pid"; fi
  wait "$observer_pid" || rc=18
 fi
 [ -z "$jpid" ] || kill -TERM "$jpid" 2>/dev/null
if [ "$launched" = 1 ]; then
 python3 -I /root/sweep2/keyline_times.py "$QLOG" "/root/prov/arms/$run/rpc_timing.jsonl" "/root/prov/arms/$run/keylines.jsonl" PROBE-DUMP "PRECOMP source=T3" "chan token" "BORN " "GSP REFUSED fn76/0x906f0102" GUEST_PANICKED RC_TRIGGERED "kf3: GSP" UNLOADING || echo KEYLINES_FAILED
 if [ -e /root/prov/XID_STOP ]; then rc=7; fi
 python3 - <<'PHASE_END'
import json
from pathlib import Path
p=Path('/root/prov/phase.json');t=p.with_suffix('.tmp');t.write_text('{}\n');t.replace(p)
p=Path('/root/prov/observation_permit.json');t=p.with_suffix('.tmp');t.write_text('{}\n');t.replace(p)
PHASE_END

fi
sha256sum -c /root/prov/bwr_parent_sha256.txt || rc=16
 echo "ARM_EXIT run=$run rc=$rc $(date -u -Is)"
 exit "$rc"
}
trap cleanup EXIT
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = "$BASE" ]
[ ! -e /root/prov/XID_STOP ] || { echo XID_STOP_PRESENT; exit 33; }
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ ! -e "/root/prov/arm_${run}.started" ]
grep -q '^BIND_STOPPED_READY ' /root/prov/bwr_bind.log
sha256sum -c /root/prov/bwr_parent_sha256.txt
python3 -E -s /root/sweep2/guard/blackwell/arm_history.py snapshot /root/prov/native_receipt.json /root/prov/native_receipt.json.accepted.json "/root/prov/arm_${run}_accepted_start.json"
python3 - "$profile" "$run" "$source" "$binary" <<'PY'
import pathlib,json,datetime,sys,hashlib
p=pathlib.Path('/root/sweep2/lease.json');r=json.loads(p.read_text());now=datetime.datetime.now(datetime.timezone.utc);created=datetime.datetime.fromisoformat(r['created_utc']);end=created+datetime.timedelta(minutes=r['auto_destroy_minutes']-25);assert now<end,'latest-start guard: leave25min for observation/export/teardown'
counts=pathlib.Path('/root/prov/arm_counts.json');j=json.loads(counts.read_text()) if counts.exists() else {'on':0,'off':0};assert j[sys.argv[1]]<9,'max9 per profile per lease (owner-approved bisect)';j[sys.argv[1]]+=1;counts.write_text(json.dumps(j,indent=2)+'\n')
out={'utc':now.isoformat(),'own_id':r['id'],'profile':sys.argv[1],'run':sys.argv[2],'source':sys.argv[3],'binary':sys.argv[4],'binary_sha256':hashlib.sha256(pathlib.Path(sys.argv[4]).read_bytes()).hexdigest(),'harness_sha256':hashlib.sha256(pathlib.Path('/root/sweep2/arm.sh').read_bytes()).hexdigest(),'rpc_observer_sha256':hashlib.sha256(pathlib.Path('/root/sweep2/rpc_timing.py').read_bytes()).hexdigest(),'cell_attempt':j[sys.argv[1]],'latest_start_utc':end.isoformat()};pathlib.Path('/root/prov/arm_'+sys.argv[2]+'_start_receipt.json').write_text(json.dumps(out,indent=2)+'\n')
PY
date -u -Is >"/root/prov/arm_${run}.started"
parent=$VM/disk/bw580_bind.qcow2; child=$VM/disk/$run.qcow2
[ -f "$parent" ] && [ ! -e "$child" ]
qemu-img create -q -f qcow2 -F qcow2 -b "$parent" "$child"
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/sweep2/p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1 KF3_DIAG_PROGRESS=1000
[ ! -f "/root/prov/arm_env_${run}.sh" ] || . "/root/prov/arm_env_${run}.sh"
export WINVM_QEMU=$binary
extra=(); [ "$profile" = on ] || extra=(--kf3-extra display=off,gop=off,gop-efi=)
mkdir -p "/root/prov/arms/$run"
journalctl -k -f -o short-iso-precise --since now >"/root/prov/arms/$run/host_kernel.log" 2>&1 &
jpid=$!
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
python3 - "$owned_pid" "$run" "$binary" <<'OWN_PHASE'
import json,sys,os
from pathlib import Path
pid=int(sys.argv[1]);proc=Path('/proc')/str(pid)
rec={'arm':sys.argv[2],'pid':pid,'exe':sys.argv[3],'starttime':int((proc/'stat').read_text().rpartition(')')[2].split()[19]),'pci':Path('/root/prov/pci.txt').read_text().strip(),'rpc_path':'/root/prov/arms/'+sys.argv[2]+'/rpc_timing.jsonl'}
p=Path('/root/prov/phase.json');t=p.with_suffix('.tmp');t.write_text(json.dumps(rec)+'\n');t.replace(p)
OWN_PHASE
mkdir -p "/root/prov/arms/$run"
python3 -I /root/sweep2/rpc_timing.py "$QLOG" "/root/prov/arms/$run/rpc_timing.jsonl" "$owned_pid" >"/root/prov/arms/$run/rpc_timing_controller.log" 2>&1 &
observer_pid=$!
readiness=/root/prov/readiness.json
ready_tool=/root/sweep2/guard/blackwell/readiness.py
python3 -E -s "$ready_tool" boot /root/prov/phase.json "$readiness" /root/sweep2/lease.json "$W"
handshake_seconds=$(python3 -E -s "$ready_tool" value "$readiness" handshake)
export WINVM_SSH_CONNECT_TIMEOUT=$(python3 -E -s "$ready_tool" value "$readiness" banner_query)
python3 /root/sweep2/bind_receipt.py "arm_${run}" "$binary" "$profile"
for f in verify_stage580.ps1 enable580_once.ps1 diag_min.ps1 init_state.ps1; do bash "$W" scp-to "/root/sweep2/probes/$f" "C:/kf/$f"; done
python3 -E -s "$ready_tool" run "$readiness" verify_stage -- bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\verify_stage580.ps1' </dev/null >"/root/prov/arm_${run}_pre_enable.txt" 2>&1
grep '^STAGE580_READY=1 ' "/root/prov/arm_${run}_pre_enable.txt"
if grep -aq 'kf-rm: rpc-trace' "$QLOG"; then echo PRECONDITION_REFUSED_GSP_WHILE_DISABLED; exit 7; fi
init_ready="/root/prov/arms/$run/init_observer.ready.json"
python3 -E -s /root/sweep2/guard/blackwell/init_observer.py --calibration "$readiness" /root/prov/phase.json "$run" "$init_ready" "/root/prov/arms/$run/early_code0.txt" -- bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\init_state.ps1' </dev/null >"/root/prov/arms/$run/init_observer.log" 2>&1 &
init_observer_pid=$!
for i in $(seq 1 "$handshake_seconds"); do
 [ -f "$init_ready" ] && break
 kill -0 "$init_observer_pid" || { echo INIT_OBSERVER_FAILED_BEFORE_ENABLE; exit 22; }
 sleep 1
done
[ -f "$init_ready" ] || { echo INIT_OBSERVER_HANDSHAKE_TIMEOUT; exit 23; }
echo "INIT_OBSERVER_READY $(date -u -Is)"
python3 - "$profile" "$run" <<'OBSERVATION_PERMIT'
import datetime,json,sys
from pathlib import Path
sys.path.insert(0,'/root/sweep2/guard/blackwell')
import windows_observation as O
phase=json.loads(Path('/root/prov/phase.json').read_text())
authority=json.loads(Path('/root/sweep2/observation_authority.json').read_text())
now=datetime.datetime.now(datetime.timezone.utc)
assert phase['arm']==sys.argv[2]
phase['expected_pci']=phase['pci'];O.G.validate_phase_owner(phase)
permit={key:phase[key] for key in ('arm','pid','starttime','exe','pci')}
permit.update(mode=O.MODE,profile=sys.argv[1],lease_id=authority['lease_id'],
              enable_count=1,armed_utc=now.isoformat())
assert O.scope_enabled(phase,permit,authority,now)
O.G.atomic_json('/root/prov/observation_permit.json',permit)
O.G.atomic_json('/root/prov/arms/'+phase['arm']+'/observation_permit.json',permit)
print('WINDOWS_ON_OBSERVATION_ARMED '+json.dumps(permit))
OBSERVATION_PERMIT
timeout 600 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\enable580_once.ps1' </dev/null >"/root/prov/arm_${run}_enable.txt" 2>&1
cat "/root/prov/arm_${run}_enable.txt"
# Read actual PnP/CIM/System state now, before the known40second crash window.
# This observation delay is not a product/driver timeout or TDR policy change.
diag_rc=0
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\diag_min.ps1' </dev/null >"/root/prov/arm_${run}_diag.txt" 2>&1 || diag_rc=$?
echo "OBSERVATION_DIAG_RC $diag_rc"
cat "/root/prov/arm_${run}_diag.txt"
# Future-run guard source must include the Code0 gate; old guards fail closed here.
# Early observer records actual Code0 independently; a late diagnostic never replaces that proof.
if grep -q '"code":0' "/root/prov/arm_${run}_diag.txt" ||
 [ -s "/root/prov/arms/$run/early_code0.txt" ]; then
 start_seconds=$SECONDS
 while [ $((SECONDS-start_seconds)) -lt 600 ]; do
  if grep -q GUEST_PANICKED "$VM/logs/${run}_run.log" 2>/dev/null; then echo ARM_GUEST_PANICKED; break; fi
  if [ ! -e "/proc/$owned_pid/exe" ]; then echo ARM_VM_EXITED_DURING_HOLD; break; fi
  sleep 5
 done
 held_seconds=$((SECONDS-start_seconds))
 echo "ARM_HOLD_OBSERVED seconds=$held_seconds $(date -u -Is)"
fi
if ! grep -q GUEST_PANICKED "$VM/logs/${run}_run.log" 2>/dev/null && pgrep -x qemu-system-x86 >/dev/null; then
 timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\diag_min.ps1' </dev/null >"/root/prov/arm_${run}_diag_after.txt" 2>&1 || true
 bash "$W" shot "$run" || true
fi
echo "OBSERVATION_ONLY apps_unrun held_seconds=$held_seconds"
echo "ARM_OBSERVATION_DONE $(date -u -Is)"
