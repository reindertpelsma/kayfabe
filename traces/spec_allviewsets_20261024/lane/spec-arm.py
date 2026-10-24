#!/usr/bin/env python3
"""One fresh SPECviewperf arm (claude-spec-20261024, generalised from Q4 ce714-energy-arm.py): argv N REV LABEL WORKLOADS ITERATIONS [SECONDS]; SPEC_CONTINUE=1 keeps running past vendor failures/153 until the CLI ends or the host stops the arm."""
import datetime as dt,hashlib,importlib.util,json,os,re,signal,subprocess,sys,time
from pathlib import Path
R=Path('/data/claude-spec-20261024/lane');N=int(sys.argv[1]);REV=sys.argv[2];LABEL=sys.argv[3];WORKLOADS=sys.argv[4].split(',');ITERATIONS=int(sys.argv[5]);LIMIT=int(sys.argv[6]) if len(sys.argv)>6 else 3600;CONT=os.environ.get('SPEC_CONTINUE')=='1';S=R/'arms'/str(N);S.mkdir(parents=True,exist_ok=True);A=R/'windows-20261022'/f'boundary-kayfabe-{N}'
ROOT=rf'C:\Users\Public\claude-spec-{N}';OBS=rf'C:\Users\Public\claude-spec-observe-{N}'
env=dict(os.environ,Q4_AGENT_STATE=str(S/'agent-state'));server=None;control=None
result={'classification':'MEASURED','arm':LABEL,'run':N,'rev':REV,'workloads':WORKLOADS,'iterations':ITERATIONS,'continue':CONT,'resolution':'FullHD','verdict':'FAIL','reason':'HARNESS_NOT_FINISHED'}
def now():return dt.datetime.now().astimezone().isoformat()
def emit(x):print(json.dumps(x),flush=True)
def agent(script,name,limit=45):
 p=subprocess.run(['python3',str(R/'bisect-agent.py'),'run',script,str(limit)],env=env,capture_output=True,text=True,timeout=limit+10)
 (S/name).write_text(p.stdout)
 if p.returncode:raise RuntimeError(f'GUEST_COMMAND_FAILED {name} rc={p.returncode}')
 return p.stdout.lstrip('\ufeff')
def status_script(boot):
 s=(R/'bisect-status-template.ps1').read_text();s=s.replace('C:\\Users\\Public\\codex-Q4-candidate',ROOT).replace('C:\\Users\\Public\\codex-Q4-observe',OBS).replace('2026-10-22T06:14:43.5000000+02:00',boot);return s
try:
 # Actual source tests must be green before any VM is started.
 proof=json.loads((R/f'tests-{REV[:8]}.json').read_text())
 assert proof.get('rev',proof.get('candidate'))==REV and proof['failed']==0 and proof.get('exit',proof.get('cargo_exit'))==0
 (S/'test-proof.json').write_text(json.dumps(proof,indent=2)+'\n')
 serverlog=(S/'agent-server.log').open('w');server=subprocess.Popen(['python3','-u',str(R/'bisect-agent.py')],env=env,stdout=serverlog,stderr=subprocess.STDOUT)
 time.sleep(.2);assert server.poll() is None,'AGENT_PORT_UNAVAILABLE'
 clog=(S/'controller.log').open('w');control=subprocess.Popen(['flock','-w','120','-o','/tmp/kayfabe-fastguest.lock','python3','-u',str(R/'spec_session.py'),str(N),REV],stdout=clog,stderr=subprocess.STDOUT)
 (S/'owned.json').write_text(json.dumps({'runner':os.getpid(),'server':server.pid,'flock_controller':control.pid},indent=2)+'\n')
 end=time.monotonic()+180
 while not (A/'session-owned.json').exists():
  assert control.poll() is None,'CONTROLLER_EXIT_BEFORE_BOOT'
  assert time.monotonic()<end,'BOUNDED_LOCK_OR_LAUNCH_WAIT_EXPIRED'
  time.sleep(1)
 emit({'time':now(),'arm':LABEL,'run':N,'state':'VM_LAUNCHED'})
 end=time.monotonic()+400;ready=False;attempt=0
 while time.monotonic()<end and not (A/'arm-stop.json').exists():
  attempt+=1
  try:
   answer=agent('hostname',f'boot-hostname-{attempt}.txt',15)
   if 'kayfabe-windows' in answer.lower():ready=True;break
  except RuntimeError:pass
  time.sleep(5)
 assert ready,'GUEST_BOOT_NOT_READY'
 health=agent((R/'bisect-boot.ps1').read_text(),'boot.json');h=json.loads(health);boot=h['boot']
 assert h['gpu_ok'],'GUEST_GPU_HEALTH_REFUSED'
 observer=(R/'bisect-observer.ps1').read_text().replace('C:\\Users\\Public\\codex-Q4-observe',OBS)
 agent(observer,'observer-start.txt')
 agent("$p='"+OBS+"\\ready.txt';$end=(Get-Date).AddSeconds(10);while(-not(Test-Path $p) -and (Get-Date) -lt $end){Start-Sleep -Milliseconds 200};if(-not(Test-Path $p)){throw 'OBSERVER_NOT_READY'};'READY'",'observer-ready.txt')
 controls=(R/'bisect-observer-controls.ps1').read_text().replace('C:\\Users\\Public\\codex-Q4-observe',OBS)
 j=json.loads(agent(controls,'observer-controls.json'));items=j.get('value',j) if isinstance(j,dict) else j
 assert [x['actual'] for x in items]==[0,7,9] and [x['expected'] for x in items]==[0,7,9]
 from controls import controls_ok
 records=[]
 for attempt in range(5):
  raw=agent("Get-Content '"+OBS+"\\exits.jsonl' -Raw",f'observer-events-{attempt}.jsonl',15)
  records=[json.loads(x) for x in raw.splitlines() if x.strip()]
  if controls_ok(j,records):break
  time.sleep(1)
 proof={'classification':'MEASURED','passed':controls_ok(j,records),'controls':items,'observed':records}
 (S/'observer-controls-proof.json').write_text(json.dumps(proof,indent=2)+'\n')
 assert proof['passed'],'PROCESS_OBSERVER_CONTROL_REFUSED'
 spec=importlib.util.spec_from_file_location('guest_suite',R/'bisect-guest-suite.py');G=importlib.util.module_from_spec(spec);spec.loader.exec_module(G)
 task=G.render_task(WORKLOADS,ITERATIONS,ROOT);(S/'install.ps1').write_text(task)
 from quiet import load
 load(S/'load-before-spec.json',f'SPEC-{N}-{LABEL}')
 agent(task,'task-start.txt');emit({'time':now(),'arm':LABEL,'run':N,'state':'SPEC_STARTED','workloads':WORKLOADS})
 sample=0;offset=0;maxima={k:0 for k in ['pointer_large_entries','pointer_entry_max_bytes','refused_scan_budget','poisoned']};budget=[];refusal_lines=[];started=time.monotonic();last=None
 while not (A/'arm-stop.json').exists():
  sample+=1;t=now();before=(A/'qemu.log').stat().st_size
  last=None
  for _try in range(4):
   try:
    last=json.loads(agent(status_script(boot),f'status-{sample:04}-{_try}.json'));break
   except (ValueError,RuntimeError) as _e:
    with (S/'status-retries.log').open('a') as _f:_f.write(f'{now()} sample {sample} try {_try}: {type(_e).__name__}: {_e}\n')
    time.sleep(3)
  if last is None:raise RuntimeError('STATUS_UNREADABLE_4_TRIES')
  (S/'status-current.json').write_text(json.dumps(last,indent=2)+'\n')
  with (A/'qemu.log').open('rb') as f:f.seek(0,2);size=f.tell();offset=max(offset,size-16*1024*1024);f.seek(offset);raw=f.read(16*1024*1024);offset=f.tell();chunk=raw.decode('utf-8','replace')
  assert len(raw)<=16*1024*1024,'QEMU_LOG_BYTE_BOUND'
  for line in chunk.splitlines():
   for key in maxima:
    hits=re.findall(key+r'[=:] ?(\d+)',line)
    if hits:maxima[key]=max(maxima[key],max(map(int,hits)))
   if len(refusal_lines)<200 and any(x in line for x in ('REFUSED','POISON','NoCeObject','ForeignClass','Budget','PROGRESS_STRAND','UNLOADING_GUEST_DRIVER')):refusal_lines.append(line[:1500])
   if 'USER_RING_REFUSED' in line and any(x in line for x in ['USER_RING_GP_FORMAT Budget','USER_RING_SCAN_BUDGET']):budget.append(line[:1500])
  events=[x for x in last['system'] if x['id'] in (153,4101)];logs='\n'.join(x['text'] for x in last['run_logs']);completed_names=re.findall(r'\[Workload:\s*([^]]+)\]\s+Completed',logs);completed=all(completed_names.count(w)==ITERATIONS for w in WORKLOADS) and len(completed_names)==len(WORKLOADS)*ITERATIONS;failed_names=re.findall(r'\[Workload:\s*([^]]+)\]\s+Failed',logs);failed=bool(failed_names);subtests=re.findall(r'\[workload:\s*([^]]+?) / subtest:\s*([^]]+)\] Started',logs);sub_done=re.findall(r'\[workload:\s*([^]]+?) / subtest:\s*([^]]+)\] (Completed|Failed)',logs)
  state={'classification':'MEASURED','time':t,'arm':LABEL,'run':N,'sample':sample,'completed':completed,'completed_workloads':completed_names,'failed':failed,'failed_workloads':failed_names,'subtest':subtests[-1] if subtests else None,'subtest_results':sub_done,'refusal_lines':len(refusal_lines),'guest153':sum(x['id']==153 for x in events),'guest4101':sum(x['id']==4101 for x in events),'boot_unchanged':last['currentBoot']==last['initialBoot'],'budget_refusals':len(budget),'counters':maxima,'task':last['task'],'qemu_before':before,'qemu_after':offset}
  with (S/'samples.jsonl').open('a') as f:f.write(json.dumps(state)+'\n')
  (S/'sample-current.json').write_text(json.dumps(state,indent=2)+'\n');emit(state)
  exits=[json.loads(x) for x in last.get('process_exit_log','').splitlines() if x.strip()]
  abnormal=[x for x in exits if 'viewperf' in x.get('name','').lower() and x.get('exit')!=0]
  state['viewperf_nonzero_exits']=abnormal
  ce_refused=any(x in chunk for x in ('NoCeObject','ForeignClass'))
  state['ce_class_refusal']=ce_refused
  (S/'refusal-lines.txt').write_text('\n'.join(refusal_lines)+'\n')
  if not state['boot_unchanged'] or ((events or budget or failed or abnormal or ce_refused) and not CONT):
   result.update(verdict='FAIL',reason='GUEST_EVENT_REBOOT_BUDGET_OR_VENDOR_FAILURE',sample=state,events=events);(S/'first-failure.json').write_text(json.dumps(result,indent=2)+'\n');break
  if CONT and sample>3 and last['task']!=4 and not completed:
   result.update(verdict='FAIL',reason='CLI_ENDED_WITH_FAILURES',sample=state,events=events);break
  if completed and last['task']!=4:
   result.update(verdict='PASS',reason='VENDOR_COMPLETED_NO_GUARD_FAILURE',sample=state,events=[]);break
  if time.monotonic()-started>LIMIT:result.update(reason='BOUNDED_ARM_TIME_EXPIRED',sample=state);break
  time.sleep(20)
 if (A/'arm-stop.json').exists():result.update(verdict='FAIL',reason='HOST_CONTROLLER_STOP',host_stop=json.loads((A/'arm-stop.json').read_text()))
 else:
  export=(R/'bisect-export.ps1').read_text().replace('C:\\Users\\Public\\codex-VF1',ROOT)
  agent(export,'export.json')
  # Preserve final events/boot identity and stop only our guest observer.
  final=json.loads(agent(status_script(boot),'status-final.json'))
  events=[x for x in final['system'] if x['id'] in (153,4101)]
  if events and result['verdict']=='PASS':result.update(verdict='FAIL',reason='GUEST_EVENT',events=events)
  result['final_events']=events
  agent("'' | Set-Content '"+OBS+"\\stop.flag'",'observer-stop.txt',15)
except Exception as exc:
 result.update(verdict='FAIL',reason='HARNESS_OR_GUARD_EXCEPTION',exception=type(exc).__name__+': '+str(exc));emit(result)
finally:
 result['ended']=now();(S/'result-before-restore.json').write_text(json.dumps(result,indent=2)+'\n')
 if control is not None:
  if (A/'command.json').exists() and control.poll() is None:(A/'finish.flag').write_text(result['reason']+'\n')
  try:result['controller_exit']=control.wait(timeout=360)
  except subprocess.TimeoutExpired:result['controller_exit']='BOUNDED_RESTORATION_WAIT_EXPIRED'
  if (A/'arm-stop.json').exists():
   result['host_stop']=json.loads((A/'arm-stop.json').read_text())
   if result['host_stop']['new_host_xids']:result.update(verdict='FAIL',reason='NEW_HOST_XID')
 if server is not None:
  if server.poll() is None:server.terminate()
  result['agent_exit']=server.wait(timeout=15)
 if (S/'restoration.json').exists():result['restoration']=json.loads((S/'restoration.json').read_text())
 result['restored']=bool(result.get('controller_exit')==0 and result.get('restoration',{}).get('cuda',{}).get('after')==0)
 (S/'result.json').write_text(json.dumps(result,indent=2)+'\n');emit(result)
 if not result['restored']:sys.exit(2)
