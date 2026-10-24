"""Read-only closure audit after the last observation cell; fatal runs use saved text only."""
from pathlib import Path
import datetime as dt
import json
import subprocess
import sys
sys.path.insert(0, '/root/sweep2/guard/blackwell')
import saved_xid32_observation as O
G=O.G
r=Path('/root/prov')
assert not (r/'XID_STOP').exists(), 'fatal run: use refusal exporter, no further GPU work'
q=subprocess.run(['pgrep','-x','qemu-system-x86'],capture_output=True,text=True)
assert q.returncode==1
controller=int((r/'sweep_controller.pid').read_text())
assert not Path('/proc',str(controller),'exe').exists()
assert subprocess.run(['flock','-n','/tmp/kayfabe-fastguest.lock','true']).returncode==0
native=json.loads((r/'native_receipt.json').read_text())['approved_native_xid_lines']
a=json.loads((r/'native_receipt.json.accepted.json').read_text())
authority=json.loads(Path('/root/sweep2/observation_authority.json').read_text())
assert len(native)==5
allowed=a['approved_native_xid_lines']
assert allowed[:5]==native
assert list(G.N.xid_lines(G.N.kernel_snapshot()))==allowed
recorded=[]
for arm,records in a['observations'].items():
    assert arm in authority['arms'] and len(records)<=8
    permit=json.loads((r/'arms'/arm/'observation_permit.json').read_text())
    phase={key:permit[key] for key in ('arm','pid','starttime','exe','pci')}
    lines=[record['line'] for record in records]
    check=O.judge_observation(lines,phase=phase,permit=permit,authority=authority,
                              now=O.aware(a['observed_utc']))
    assert records==check
    recorded.extend(lines)
assert allowed[5:]==recorded
health=O.check_host_health(json.loads((r/'native_receipt.json').read_text())['pci'])
sources={}
for name,path in [('product','/root/kayfabe'),('readiness','/workspace/bw-readiness'),
                  ('native_fixture','/workspace/bw-native-fixture'),
                  ('measured','/workspace/bw-measured'),('observed_b1','/workspace/bw-observed-b1')]:
    if not Path(path).exists():continue
    source=subprocess.check_output(['git','-C',path,'rev-parse','HEAD'],text=True).strip()
    status=subprocess.check_output(['git','-C',path,'status','--porcelain',
                                    '--untracked-files=no'],text=True)
    assert not status
    sources[name]=source
assert sources['measured']=='48b027267a0e05c9ffdb4a049d34f4d719fd1837'
if (r/'observed_b1_binary.json').exists():
    assert sources['observed_b1']=='cb80cbd29b7a05f904acd822f172485fd49938a9'
print(json.dumps({'utc':dt.datetime.now(dt.timezone.utc).isoformat(),
                  'native_xid_count':5,'observed_record_count':len(recorded),
                  'per_arm_counts':{key:len(v) for key,v in a['observations'].items()},
                  'exact_accepted_history':True,'host_health':health,'qemu_count':0,
                  'controller_exited':True,'gpu_lock_released':True,
                  'clean_sources':sources,'no_more_product_arms':True},indent=2))
