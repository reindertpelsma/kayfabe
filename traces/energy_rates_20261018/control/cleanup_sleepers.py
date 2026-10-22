"""Retire only completed Q3 trial timer children, proven by pgid and lane environment."""
import json,os,signal,time,datetime as dt
from pathlib import Path
R=Path('/data/codex-Q3');groups={}
for row in json.loads((R/'series-results.json').read_text()):
 n=row['run'];A=R/'windows-20261022'/f'boundary-kayfabe-{n}';S=R/'energy-bisect'/str(n)
 assert row['restored'] and (S/'session-exit.json').exists()
 j=json.loads((A/'session-owned.json').read_text())
 for pid,start in j['owned_helpers'].items():groups[int(pid)]={'run':n,'start':start}
def info(p):
 try:
  raw=(p/'stat').read_text();parts=raw[raw.rfind(')')+2:].split();cmd=(p/'cmdline').read_bytes().split(b'\0');env=(p/'environ').read_bytes().split(b'\0')
  return {'pid':int(p.name),'state':parts[0],'ppid':int(parts[1]),'pgid':int(parts[2]),'start':int(parts[19]),'cmd':cmd,'lane':b'WIN_DIR=/data/codex-Q3/windows-20261022' in env}
 except (FileNotFoundError,ProcessLookupError,PermissionError):return None
owned=[]
for p in Path('/proc').iterdir():
 if not p.name.isdigit():continue
 j=info(p)
 if not j or j['state']=='Z' or j['pgid'] not in groups or not j['lane']:continue
 if j['cmd'][0] not in [b'sleep',b'/usr/bin/sleep']:continue
 assert j['start']>=groups[j['pgid']]['start'] and j['ppid']==1
 owned.append(j)
for j in owned:
 fresh=info(Path('/proc')/str(j['pid']))
 assert fresh and fresh['start']==j['start'] and fresh['pgid']==j['pgid'] and fresh['lane']
 os.kill(j['pid'],signal.SIGTERM)
end=time.monotonic()+5
while time.monotonic()<end:
 remaining=[j for j in owned if (x:=info(Path('/proc')/str(j['pid']))) and x['state']!='Z' and x['start']==j['start']]
 if not remaining:break
 time.sleep(.1)
assert not remaining,'OWN_TIMER_STILL_ACTIVE'
r={'classification':'MEASURED','evidence':'timer-cleanup.json','time':dt.datetime.now().astimezone().isoformat(),'ownership':'stored session helper process group + exact Q3 WIN_DIR + sleep argv + unchanged starttime + completed/restored trial','terminated':[{'run':groups[j['pgid']]['run'],'pid':j['pid'],'pgid':j['pgid'],'starttime':j['start']} for j in owned],'remaining_active':len(remaining)}
(R/'timer-cleanup.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r))
