#!/bin/bash
# B1n DIAGNOSTIC arm lx_reload (owner 2026-10-24): one Linux fat-guest boot under kf3 with the
# B1 binary via the existing boot_capture.sh; its hook unloads and reloads the guest NVIDIA
# driver and runs nvidia-smi again (second guest RM life, the FSP-family path of H2).
# The QEMU is registered in phase.json for the Xid guard with NO observation permit, so any
# Xid or new host kernel error stops it. Bounded to 15 minutes; logs go to the capture.
set -euo pipefail
. /root/sweep2/common.sh
run=lx_reload; binary=${1:?B1 binary}; source=${2:?source}
[[ $source =~ ^[0-9a-f]{40}$ ]]
[ -x "$binary" ]
exec >"/root/prov/arm_${run}.log" 2>&1
tag=bw4_lx_reload
cap_pid=; jpid=; registered=0
cleanup(){
 rc=$?; trap - EXIT; set +e
 if [ -n "$cap_pid" ]; then kill -TERM "$cap_pid" 2>/dev/null; wait "$cap_pid" 2>/dev/null; fi
 [ -z "$jpid" ] || kill -TERM "$jpid" 2>/dev/null
 if [ "$registered" = 1 ]; then
  python3 - <<'PHASE_END'
from pathlib import Path
p=Path('/root/prov/phase.json');t=p.with_suffix('.tmp');t.write_text('{}\n');t.replace(p)
PHASE_END
 fi
 for f in /workspace/bench/run_${tag}_*; do [ -f "$f" ] && cp "$f" "/root/prov/arms/$run/"; done
 if [ -e /root/prov/XID_STOP ]; then rc=7; fi
 echo "ARM_EXIT run=$run rc=$rc $(date -u -Is)"
 exit "$rc"
}
trap cleanup EXIT
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = "$BASE" ]
[ ! -e /root/prov/XID_STOP ] || { echo XID_STOP_PRESENT; exit 33; }
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ ! -e "/root/prov/arm_${run}.started" ]
[ -f /workspace/bench/guest.qcow2 ]
date -u -Is >"/root/prov/arm_${run}.started"
mkdir -p "/root/prov/arms/$run"
cp /workspace/bench/guest.qcow2.STAGED "/root/prov/arms/$run/guest_staged.txt" || true
sha256sum "$binary" >"/root/prov/arms/$run/binary_sha256.txt"
journalctl -k -f -o short-iso-precise --since now >"/root/prov/arms/$run/host_kernel.log" 2>&1 &
jpid=$!
chmod 0755 /root/sweep2/reload_hook.sh
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
export KF3_RPC_TRACE=1
echo "ARM_START run=$run source=$source label=DIAGNOSTIC $(date -u -Is)"
flock -w 60 -o /tmp/kayfabe-fastguest.lock env QEMU_BIN="$binary" \
 POST_CAPTURE_HOOK=/root/sweep2/reload_hook.sh BOOT_TIMEOUT=150 GQ_TIMEOUT=90 \
 NVKVM_RAM_MB=4096 KF_SMP=3 timeout 900 bash scripts/bench/boot_capture.sh "$tag" &
cap_pid=$!
pid=
for i in $(seq 1 180); do
 pid=$(pgrep -x qemu-system-x86 || true)
 [ -n "$pid" ] && break
 kill -0 "$cap_pid" 2>/dev/null || break
 sleep 1
done
[ -n "$pid" ] || { echo LINUX_QEMU_NEVER_STARTED; exit 42; }
[ "$(wc -w <<<"$pid")" = 1 ]
python3 - "$pid" "$binary" <<'OWN_PHASE'
import json,os,sys
from pathlib import Path
sys.path.insert(0,'/root/sweep2/guard/blackwell');import phase_xid_guard as G
pid=int(sys.argv[1]);proc=Path('/proc')/str(pid)
rec={'arm':'lx_reload','pid':pid,'exe':os.path.realpath(sys.argv[2]),
     'starttime':int((proc/'stat').read_text().rpartition(')')[2].split()[19]),
     'pci':Path('/root/prov/pci.txt').read_text().strip(),'rpc_path':None}
rec['expected_pci']=rec['pci'];G.validate_phase_owner(rec)
del rec['expected_pci']
p=Path('/root/prov/phase.json');t=p.with_suffix('.tmp')
t.write_text(json.dumps(rec)+'\n');t.replace(p)
print('LINUX_QEMU_REGISTERED '+json.dumps(rec))
OWN_PHASE
registered=1
cap_rc=0; wait "$cap_pid" || cap_rc=$?; cap_pid=
echo "BOOT_CAPTURE_RC=$cap_rc $(date -u -Is)"
grep -aE 'SMI_RC=|SMI2_RC=|MODPROBE2_RC=|RELOAD_VERDICT|HOOK_RC|NVIDIA_LOADED_AFTER' \
 "/workspace/bench/run_${tag}_probe.log" || echo RELOAD_LINES_ABSENT
[ ! -e /root/prov/XID_STOP ]
echo "ARM_OBSERVATION_DONE $(date -u -Is)"
