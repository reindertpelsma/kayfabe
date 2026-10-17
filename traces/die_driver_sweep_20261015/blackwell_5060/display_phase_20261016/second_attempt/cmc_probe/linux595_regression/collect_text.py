import base64,hashlib,json,re,subprocess
from pathlib import Path

root=Path('/workspace/codex/blackwell-hw/traces/die_driver_sweep_20261015/blackwell_5060/display_phase_20261016/second_attempt/cmc_probe/linux595_regression')
root.mkdir(parents=True,exist_ok=True)
program=r'''
import hashlib,json,pathlib
files=list(pathlib.Path('/workspace/bench').glob('fast_codex_gb206phase_cmc595_*.log'))
files+=list(pathlib.Path('/root/prov/gb206phase-cmc-fast-receipts').glob('*.json'))
files+=list(pathlib.Path('/root/prov/gb206phase-cmc-fast-receipts').glob('*.FAILED'))
files+=[pathlib.Path('/workspace/bench/codex_gb206phase_cmc595_suite.out'),pathlib.Path('/workspace/bench/cl_codex_gb206phase_native595_host.out')]
files+=[pathlib.Path('/root/prov')/n for n in ['gb206phase_cmc_raw.log','gb206phase_cmc_births.log','gb206phase_cmc_heartbeat_census.json','gb206phase_cmc_raw_build.log','gb206phase_cmc_fast_build.log']]
rows=[]
for p in sorted(set(files)):
 if not p.is_file():continue
 data=p.read_bytes();row={'file':p.name,'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
 if len(data)<=1024*1024:row['text']=data.decode('utf-8',errors='replace')
 else:row['excerpt']=data[:16384].decode('utf-8',errors='replace')+'\n[OMITTED: raw text exceeds1MiB; retained on rental]\n'+data[-32768:].decode('utf-8',errors='replace')
 rows.append(row)
print(json.dumps(rows))
'''
r=subprocess.run(['ssh','-F','/workspace/codex/ssh_config','-o','ForwardAgent=no','codex-gb206phase2-56286918','python3 -'],input=program,capture_output=True,text=True,timeout=90)
r.check_returncode();rows=json.loads(r.stdout)
def scrub(s):
    s=re.sub(r'GPU-[A-Za-z0-9-]+','GPU-REDACTED',s)
    s=re.sub(r'(?i)\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b','UUID-REDACTED',s)
    s=re.sub(r'(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b','MAC-REDACTED',s)
    def ip(m):
        return 'IP-REDACTED' if all(int(n)<=255 for n in m.group().split('.')) else m.group()
    return re.sub(r'(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])',ip,s)
manifest=[]
for row in rows:
    name=row['file'];assert Path(name).name==name
    text=row.pop('text',None);excerpt=row.pop('excerpt',None)
    if text is not None:(root/name).write_text(scrub(text))
    else:(root/(name+'.excerpt.txt')).write_text(scrub(excerpt))
    row['export']='screened complete text' if text is not None else 'screened excerpt only; full raw text stays on rental'
    manifest.append(row)
(root/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('COLLECTED_LINUX_TEXT',len(rows),'oversize_excerpt_only',sum('excerpt' in r['export'] for r in manifest))
