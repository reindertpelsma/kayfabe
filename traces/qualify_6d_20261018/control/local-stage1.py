import datetime as dt,json,os,subprocess
from pathlib import Path
R=Path('/workspace/q4-wt');P=Path('/workspace/q4-6d-private');T=Path('/workspace/q4-target');C='70da04ffaebdee3b51e5eca6aa1eb3919e92e80d'
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=R,text=True).strip()==C
assert not subprocess.check_output(['git','diff','--name-only','--','crates','qemu','cuda'],cwd=R,text=True).strip()
env=dict(os.environ,CARGO_TARGET_DIR=str(T),CARGO_BUILD_JOBS='2',KAYFABE_NO_KVM='1');rows=[]
for name,args in [('candidate-tests',['cargo','test','--release','--locked','-j2','-p','kf-*','--no-fail-fast']),('candidate-fmt',['cargo','fmt','--all','--check']),('candidate-clippy',['cargo','clippy','--release','--workspace','--all-targets','--locked','-j2','--message-format=json'])]:
 free=os.statvfs('/').f_bavail*os.statvfs('/').f_frsize
 assert free>=4*1024**3,'DISK_GUARD_BELOW_4G'
 row={'name':name,'revision':C,'start':dt.datetime.now(dt.timezone.utc).isoformat(),'free_bytes':free,'command':args};print(json.dumps({'state':'START',**row}),flush=True)
 with (P/(name+'.log')).open('w') as f:p=subprocess.run(['nice','-n','19',*args],cwd=R,env=env,stdout=f,stderr=subprocess.STDOUT)
 row.update(exit=p.returncode,end=dt.datetime.now(dt.timezone.utc).isoformat());rows.append(row)
 (P/'local-checks.json').write_text(json.dumps({'classification':'MEASURED','evidence':'local-checks.json','candidate':C,'checks':rows},indent=2)+'\n');print(json.dumps({'state':'EXIT',**row}),flush=True)
print('LOCAL_STAGE1_BATCH_FINISHED',flush=True)
