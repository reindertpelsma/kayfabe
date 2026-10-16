import json,re,subprocess
from pathlib import Path
root=Path('/workspace/codex/blackwell-hw/traces/die_driver_sweep_20261015/blackwell_5060/display_phase_20261016/second_attempt/linux595_apps');root.mkdir(exist_ok=True)
program=r'''
import hashlib,json,pathlib
files=list(pathlib.Path('/root/gb206phase-cmc595-validation').glob('*.log'))
files+=list(pathlib.Path('/root/gb206phase-cmc595-validation').glob('*.json'))
files+=list(pathlib.Path('/root/prov').glob('gb206phase_cmc_*gfx*.log'))
files+=list(pathlib.Path('/root/prov').glob('gb206phase_cmc_*headless*.log'))
files+=[pathlib.Path('/root/prov')/n for n in ['gb206phase_cmc_fat595.log','gb206phase_cmc_fat_stage.log']]
files+=[pathlib.Path('/workspace/bench')/n for n in ['cl_codex_gb206phase_cmc_native595_host.out','guest-595.91.07.qcow2.STAGED','gfx_lane.receipt']]
files+=list(pathlib.Path('/workspace/bench').glob('cl_codex_gb206phase_cmc_native595_host_*.log'))
rows=[]
for p in files:
 if not p.is_file():continue
 raw=p.read_bytes();name=p.name
 if p.parent.name=='gb206phase-cmc595-validation':name='guest-'+name
 row={'name':name,'path':str(p),'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest()}
 if len(raw)<=1024*1024:row['text']=raw.decode(errors='replace')
 else:row['text']=raw[:16384].decode(errors='replace')+'\n[oversize raw omitted; erased at retirement]\n'+raw[-32768:].decode(errors='replace');row['excerpt_only']=True
 rows.append(row)
print(json.dumps(rows))
'''
r=subprocess.run(['ssh','-F','/workspace/codex/ssh_config','-o','ForwardAgent=no','codex-gb206phase2-56286918','python3 -'],input=program,text=True,capture_output=True,timeout=90);r.check_returncode();rows=json.loads(r.stdout)
def scrub(s):
 s=re.sub(r'GPU-[A-Za-z0-9-]+','GPU-REDACTED',s)
 s=re.sub(r'(?i)\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b','UUID-REDACTED',s)
 s=re.sub(r'(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b','MAC-REDACTED',s)
 return re.sub(r'\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|192\.168\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b','PRIVATE-IP-REDACTED',s)
for row in rows:
 name=row['name']+('.excerpt.txt' if row.get('excerpt_only') else '')
 (root/name).write_text(scrub(row.pop('text')))
(root/'manifest.json').write_text(json.dumps(rows,indent=2)+'\n')
print('COLLECTED_HEADLESS595',len(rows),'text artifacts')
