import datetime as dt,hashlib,json,os,subprocess,sys,time
from pathlib import Path
R=Path('/data/codex-Q3')
REVS={'D':'468a35c9d845afc1938683540acc251bdef90e5a','A':'40583bc297264fb57a20d84cc795d05d493197b5','B':'a83394e66e960d8017caa0ef19fa579f1724806b','C':'afe00b3cbc7e345e2953b1bc199d9cce1054cc38'}
HASHES={'D':'ea10ba9b1e12a1d43509fce0f527e3377ceb5806cd0b76b147ec2ba221f07845','A':'084d100bc3bf343d3fc2c311f2601694ab3924313d9999651673b14a4c7e335a','B':'ab9850e1c58580ed20dbf315f73522d22ee9ed12c158c94eee8a94530956c801','C':'9f7c26f69c6e94c37f8757b2928c4300cece10914c843db62dd6bf9f5662d34d'}
ORDER=['D1','A3','B2','C3','D2','B3','A4','D3','B4','C4','D4']
START=dt.datetime.now().astimezone().isoformat()
def write(p,j):p.write_text(json.dumps(j,indent=2)+'\n')
def emit(j):print(json.dumps(j),flush=True)
write(R/'series-owned.json',{'pid':os.getpid(),'started':START,'order':ORDER})
rows=[];stop=None
try:
 for i,label in enumerate(ORDER):
  n=4301+i;rev=REVS[label[0]]
  assert time.time()<dt.datetime.fromisoformat('2026-10-22T15:35:00+02:00').timestamp(),'NO_TIME_FOR_ANOTHER_FULL_ARM'
  actual=hashlib.sha256((R/'kf3-bins'/rev[:8]/'qemu-system-x86_64').read_bytes()).hexdigest()
  assert actual==HASHES[label[0]],'BINARY_HASH_CHANGED'
  started=dt.datetime.now().astimezone().isoformat()
  emit({'state':'ARM_START','order':i+1,'run':n,'arm':label,'time':started})
  write(R/'series-current.json',{'state':'ARM_RUNNING','order':i+1,'run':n,'arm':label,'started':started,'completed':rows})
  with (R/f'energy-{n}.log').open('w') as log:
   proc=subprocess.Popen(['python3','-u',str(R/'energy-arm.py'),str(n),rev,label],stdout=log,stderr=subprocess.STDOUT)
   write(R/'series-child.json',{'pid':proc.pid,'run':n,'arm':label});code=proc.wait()
  p=R/'energy-bisect'/str(n)/'result.json'
  assert p.exists(),'ARM_RESULT_MISSING'
  result=json.loads(p.read_text())
  row={'classification':'MEASURED','order':i+1,'run':n,'arm':label,'rev':rev,'started':started,'ended':dt.datetime.now().astimezone().isoformat(),'exit':code,'verdict':result['verdict'],'reason':result['reason'],'restored':result.get('restored',False)}
  rows.append(row);write(R/'series-results.json',rows);emit(row)
  if code!=0 or not result.get('restored') or result['reason'] in ['HARNESS_OR_GUARD_EXCEPTION','HARNESS_NOT_FINISHED','BOUNDED_ARM_TIME_EXPIRED']:
   stop='SAFETY_OR_HARNESS_STOP';break
  subprocess.run(['python3',str(R/'extract-public.py'),str(n),str(R)],check=True)
except Exception as e:
 stop=type(e).__name__+': '+str(e)
finally:
 final={'classification':'MEASURED','evidence':'series-final.json','started':START,'ended':dt.datetime.now().astimezone().isoformat(),'completed':rows,'stop':stop,'all_11_completed':len(rows)==11 and stop is None}
 write(R/'series-final.json',final);write(R/'series-current.json',final);emit(final)
