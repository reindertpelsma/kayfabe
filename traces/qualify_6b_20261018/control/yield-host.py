import datetime as dt,json,os,subprocess
from pathlib import Path
R=Path('/data/codex-Q4')
last=json.loads((R/'energy-bisect/4404/result.json').read_text())
assert last['restored'] and last['controller_exit']==0 and last['host_stop']['new_host_xids']==0
(R/'gpu-monitor-stop.flag').touch()
work=['series.py','gpu-monitor.py','energy-arm.py','full-arm.py','smoke-arm.py','repeat8-arm.py','windows_session_bisect.py','bisect-agent.py','nvkvm-display-broker','qemu-system-x86_64','native-host.sh','linux-host.sh']
active=[]
for p in Path('/proc').iterdir():
 if not p.name.isdigit():continue
 try:
  args=(p/'cmdline').read_bytes().split(b'\0');cmd=b' '.join(args).decode(errors='replace')
  if '/data/codex-Q4/' in cmd:
   matched=[name for name in work if any(Path(a.decode(errors='replace')).name==name for a in args if a)]
   if matched:active.append({'pid':int(p.name),'programs':matched})
 except (FileNotFoundError,PermissionError,ProcessLookupError):pass
assert not active,('OWN_WORKLOADS_STILL_ACTIVE',active)
assert not (R/'cargo-target').exists()
subprocess.run(['flock','-w','120','-o','/tmp/kayfabe-fastguest.lock','python3',str(R/'final-health.py')],check=True,timeout=300)
health=json.loads((R/'final-health.json').read_text())
assert health['cuda_after']==0 and health['nvidia_bound'] and health['gdm']['stdout']=='active' and health['automatic_login']['configured']
# The restoration subprocess returned: its flock descriptor is closed.
# Test the lock without using the GPU or starting a VM, then return it immediately.
p=subprocess.run(['flock','-n','-o','/tmp/kayfabe-fastguest.lock','true'])
assert p.returncode==0,'LOCK_WAS_NOT_AVAILABLE_FOR_YIELD'
now=dt.datetime.now(dt.timezone.utc).isoformat()
audit={'classification':'MEASURED','evidence':'yield-audit.json','time':now,'own_active_workloads':active,'own_host_target_absent':True,'lock_acquired_and_released':True,'latest_arm':4404,'latest_arm_restored':True,'health':'final-health.json'}
(R/'yield-audit.json').write_text(json.dumps(audit,indent=2)+'\n')
(R/'HOST_YIELDED').write_text(f'''HOST YIELDED {now}
Q4 guest is shut down; no Q4 GPU/VM workloads remain; fastguest lock released.
NVIDIA bound; fresh-process cuInit 0; GDM active; automatic login configured.
E1-E4 fresh Energy series finished. Candidate has a stability failure and formatting/CI blocker.
Pending: separate eight other SPECviewperf workloads twice; Heaven windowed; Minecraft Java ten minutes; Steel Nomad GUI; app matrix if present; broker hover/grab cursor and input; final qualification report/comparison refresh.
Known display limitation: earlier GNOME state reported zero monitors and broker frames were discarded. The GDM restart own-PIDs exception remains unanswered; no display-only restart performed.
WAITING FOR CLAUDE TO EXPLICITLY RETURN THE HOST. NO Q4 GPU OR VM WORK UNTIL THEN.
''')
print(json.dumps(audit))
print('HOST_YIELDED')
