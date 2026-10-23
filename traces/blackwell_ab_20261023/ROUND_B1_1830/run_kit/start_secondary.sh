#!/bin/bash
set -euo pipefail
python3 - <<'PY'
from pathlib import Path
import json,os,signal,subprocess,sys,time,hashlib
sys.path.insert(0,'/root/sweep2/guard/blackwell');import phase_xid_guard as G
b=json.loads(Path('/root/prov/native_receipt.json').read_text())
a=json.loads(Path('/root/prov/native_receipt.json.accepted.json').read_text())['approved_native_xid_lines']
assert a==b['approved_native_xid_lines'] and len(a)==5
p=Path('/root/prov/secondary_receipt.json');G.atomic_json(p,b)
cmd='exec python3 -E -s /root/sweep2/guard/blackwell/saved_xid32_observation.py /root/prov/secondary_receipt.json "$$" /root/prov/phase.json /root/prov/XID_STOP /root/sweep2/observation_authority.json /root/prov/observation_permit.json'
child=subprocess.Popen(['bash','-c',cmd],stdin=subprocess.DEVNULL,stdout=open('/root/prov/secondary_guard.log','w'),stderr=subprocess.STDOUT,start_new_session=True)
Path('/root/prov/secondary_guard.pid').write_text(str(child.pid)+'\n')
for _ in range(150):
 if Path(str(p)+'.ready.json').exists():break
 assert child.poll() is None;time.sleep(.1)
assert Path(str(p)+'.ready.json').exists() and child.poll() is None
G.atomic_json('/root/prov/secondary_guard_receipt.json',{'pid':child.pid,'source':'owner11:05 scoped observation; frozen native/phase guard remains9ff51a6e9','sha256':hashlib.sha256(Path('/root/sweep2/guard/blackwell/saved_xid32_observation.py').read_bytes()).hexdigest(),'state':'READY before bind and all arms'})
probe=subprocess.Popen(['sleep','30'])
try:
 proc=Path('/proc')/str(probe.pid)
 phase={'pid':probe.pid,'starttime':int((proc/'stat').read_text().rpartition(')')[2].split()[19]),'exe':str((proc/'exe').resolve()),'pci':b['pci'],'expected_pci':b['pci']}
 assert G.abort_owned_qemu(phase) and probe.wait(timeout=3)==-signal.SIGKILL
 G.atomic_json('/root/prov/pidfd_runtime_smoke.json',{'status':'PASS','scope':'disposable owned sleep child, not QEMU or GPU'})
finally:
 if probe.poll() is None:probe.terminate();probe.wait(timeout=3)
PY
