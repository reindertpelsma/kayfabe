from pathlib import Path
import json,subprocess,datetime,hashlib,os
root=Path('/root/prov');run='off_c'
assert not (root/'XID_STOP').exists()
assert not subprocess.run(['pgrep','-x','qemu-system-x86'],capture_output=True).stdout
assert 'COLLECT_ALL_DONE' in (root/'collect_on_precision.log').read_text()
assert 'COLLECT_EXIT run=on_precision rc=0' in (root/'collect_on_precision.log').read_text()
p=int((root/'secondary_guard.pid').read_text());os.kill(p,0)
assert not (root/f'arm_{run}.started').exists()
subprocess.run(['python3','/root/sweep2/budget_start.py',run],check=True)
script=r'''#!/bin/bash
set -euo pipefail
exec >/root/prov/manual_off_controller.log 2>&1
root=$BASHPID;guard_pid=
cleanup(){ rc=$?;trap - EXIT;set +e;[ -z "$guard_pid" ] || { kill "$guard_pid";wait "$guard_pid"; };echo "MANUAL_OFF_CONTROLLER_EXIT rc=$rc $(date -u -Is)"; }
trap cleanup EXIT
cd /root/kayfabe
. /root/sweep2/common.sh
[ "$(git rev-parse HEAD)" = "$BASE" ]
[ ! -e /root/prov/XID_STOP ]
export HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07
python3 -E -s /root/sweep2/guard/blackwell/phase_xid_guard.py watch /root/prov/native_receipt.json "$root" /root/prov/phase.json /root/prov/XID_STOP >/root/prov/guard_watch_off.log 2>&1 &
guard_pid=$!
for i in $(seq 1 30);do [ -f /root/prov/native_receipt.json.ready.json ] && break;kill -0 "$guard_pid";sleep 1;done
python3 - "$root" <<'VERIFY'
import json,sys
from pathlib import Path
assert json.loads(Path('/root/prov/native_receipt.json.ready.json').read_text())['owned_pgid']==int(sys.argv[1])
VERIFY
kill -0 "$guard_pid"
bash /root/sweep2/arm.sh off off_c /workspace/bench/kf3-bins/8f3a9bb3/qemu-system-x86_64 "$BASE"
[ ! -e /root/prov/XID_STOP ];kill -0 "$guard_pid"
bash /root/sweep2/collect_bw.sh off_c
echo "MANUAL_OFF_CONTROLLER_DONE $(date -u -Is)"
'''
ready=root/'native_receipt.json.ready.json'
if ready.exists():
 (root/'pre_off_native_ready.json').write_bytes(ready.read_bytes());ready.unlink()
path=root/'manual_off_controller.sh';assert not path.exists();path.write_text(script)
subprocess.run(['bash','-n',str(path)],check=True)
log=(root/'manual_off_launcher.log').open('xb')
c=subprocess.Popen(['bash',str(path)],stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
(root/'manual_off_controller.pid').write_text(str(c.pid)+'\n')
r={'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'pid':c.pid,'script_sha256':hashlib.sha256(script.encode()).hexdigest(),'scope':'next fixed approved OFF arm; original per-arm harness and full-history guards unchanged; expected prior 0x50 exported fceac118b','secondary_guard':p}
(root/'manual_off_launcher.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r))
