import datetime as dt,json,os,subprocess
from pathlib import Path
R=Path('/data/codex-Q3');exclude={os.getpid()};p=os.getppid()
while p>1:
 exclude.add(p)
 try:p=int((Path('/proc')/str(p)/'stat').read_text().rsplit(')',1)[1].split()[1])
 except FileNotFoundError:break
active=[]
for q in Path('/proc').iterdir():
 if not q.name.isdigit() or int(q.name) in exclude:continue
 try:
  stat=(q/'stat').read_text().rsplit(')',1)[1].split();cmd=(q/'cmdline').read_bytes().replace(b'\0',b' ').decode('utf-8','replace')
 except (FileNotFoundError,ProcessLookupError):continue
 if stat[0]!='Z' and '/data/codex-Q3/' in cmd:active.append({'pid':int(q.name),'command':cmd[:500]})
lock=subprocess.run(['flock','-w','5','-o','/tmp/kayfabe-fastguest.lock','true'],timeout=7).returncode==0
series=json.loads((R/'series-final.json').read_text());timers=[json.loads((R/name).read_text()) for name in ['timer-cleanup-through4307.json','timer-cleanup.json']]
rows=json.loads((R/'series-results.json').read_text());assert len(rows)==11
r={'classification':'MEASURED','evidence':'final-host-cleanup.json','time':dt.datetime.now().astimezone().isoformat(),'own_active':active,'gpu_lock_reacquired':lock,'all_11_completed':series['all_11_completed'],'completed_restorations':sum(x['restored'] for x in rows),'timer_children_terminated':sum(len(t['terminated']) for t in timers),'remaining_active_timer_children':sum(t['remaining_active'] for t in timers),'q3_cargo_target_created':False,'large_artifacts_retained':'lane-owned CoWs/logs under /data/codex-Q3/windows-20261022/ and /data/codex-Q3/energy-bisect/'}
(R/'final-host-cleanup.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r));assert not active and lock and r['all_11_completed'] and r['completed_restorations']==11 and r['remaining_active_timer_children']==0
