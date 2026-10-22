import datetime as dt,json,os,subprocess,sys,time
from pathlib import Path
R=Path('/data/codex-Q4');REV='baa2977527c4509145e8033143bd664f768175a6'
order=[(4400,'FULL2','full-arm.py'),(4401,'E1','energy-arm.py'),(4402,'E2','energy-arm.py'),(4403,'E3','energy-arm.py'),(4404,'E4','energy-arm.py'),(4405,'SMOKE','smoke-arm.py')]
rows=[];stopped=None
(R/'series-owned.json').write_text(json.dumps({'pid':os.getpid(),'started':dt.datetime.now(dt.timezone.utc).isoformat(),'order':order},indent=2)+'\n')
try:
 for n,label,script in order:
  assert time.time()<dt.datetime.fromisoformat('2026-10-22T17:10:00+00:00').timestamp(),'NO_TIME_FOR_ANOTHER_ARM'
  print(json.dumps({'state':'ARM_START','run':n,'label':label,'time':dt.datetime.now(dt.timezone.utc).isoformat()}),flush=True)
  with (R/f'arm-{n}.log').open('w') as f:
   p=subprocess.Popen(['python3','-u',str(R/script),str(n),REV,label],stdout=f,stderr=subprocess.STDOUT)
   (R/'series-child.json').write_text(json.dumps({'pid':p.pid,'run':n,'label':label},indent=2)+'\n');rc=p.wait()
  result=json.loads((R/'energy-bisect'/str(n)/'result.json').read_text())
  row={'classification':'MEASURED','run':n,'label':label,'exit':rc,'verdict':result['verdict'],'reason':result['reason'],'restored':result.get('restored',False),'host_stop':result.get('host_stop')}
  rows.append(row);(R/'series-results.json').write_text(json.dumps(rows,indent=2)+'\n');print(json.dumps(row),flush=True)
  if rc or not row['restored'] or result.get('host_stop',{}).get('new_host_xids'):
   stopped='SAFETY_OR_RESTORATION_STOP';break
  if result['reason']=='HARNESS_OR_GUARD_EXCEPTION':
   stopped='HARNESS_REQUIRES_REPAIR';break
except Exception as e:
 stopped=type(e).__name__+': '+str(e)
finally:
 final={'classification':'MEASURED','evidence':'series-final.json','completed':rows,'stop':stopped,'ended':dt.datetime.now(dt.timezone.utc).isoformat()};(R/'series-final.json').write_text(json.dumps(final,indent=2)+'\n');print(json.dumps(final),flush=True)
