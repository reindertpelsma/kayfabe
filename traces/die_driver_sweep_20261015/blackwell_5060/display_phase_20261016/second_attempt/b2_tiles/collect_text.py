import hashlib,json,re,subprocess
from pathlib import Path
root=Path('/workspace/codex/blackwell-hw/traces/die_driver_sweep_20261015/blackwell_5060/display_phase_20261016/second_attempt/b2_tiles')
root.mkdir(parents=True,exist_ok=True)
program=r'''
import hashlib,json,pathlib,subprocess
paths=list(pathlib.Path('/root/prov').glob('gb206phase_broker*.log'))
paths += [pathlib.Path('/root/prov')/n for n in ['gb206phase_candidate.log','gb206phase_candidate_gates.log','gb206phase_candidate_build.log','gb206phase-tiles-export.json','gb206phase-tiles-flags.txt','gb206phase-tiles-census.log','gb206phase-tiles-stop.log','gb206phase_windows.log']]
paths += [pathlib.Path('/root/win/codex-gb206phase/process-receipt-106606.json')]
for phase in ['baseline','tiles']:
 for name in ['actual-process-receipt.json','broker.log','cmdline.txt','marker.txt','qemu.log','serial.log']:
  paths.append(pathlib.Path('/root/codex-broker-gb206phase')/('validation-'+phase)/name)
rows=[]
secret=pathlib.Path('/root/win/secrets/pw').read_text().strip()
for p in paths:
 if not p.is_file():continue
 data=p.read_bytes()
 assert not (p.name=='qemu.log' and '/root/win/' in str(p))
 row={'path':str(p),'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
 if len(data)<=1024*1024:text=data.decode(errors='replace')
 else:text=data[:16384].decode(errors='replace')+'\n[RAW OVER 1MiB: excerpt only; erased at retirement]\n'+data[-32768:].decode(errors='replace');row['excerpt_only']=True
 row['text']=text.replace(secret,'PASSWORD-REDACTED') if secret else text
 rows.append(row)
xids=subprocess.check_output(['dmesg'],text=True)
xids='\n'.join(l for l in xids.splitlines() if 'NVRM: Xid' in l)+'\n'
rows.append({'path':'host-xid-after-b2.txt','text':xids,'bytes':len(xids),'sha256':hashlib.sha256(xids.encode()).hexdigest()})
print(json.dumps(rows))
'''
r=subprocess.run(['ssh','-F','/workspace/codex/ssh_config','-o','ForwardAgent=no','codex-gb206phase2-56286918','python3 -'],input=program,text=True,capture_output=True,timeout=90)
r.check_returncode();rows=json.loads(r.stdout)
def scrub(s):
 s=re.sub(r'GPU-[A-Za-z0-9-]+','GPU-REDACTED',s)
 s=re.sub(r'(?i)\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b','UUID-REDACTED',s)
 s=re.sub(r'(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b','MAC-REDACTED',s)
 return s
for row in rows:
 path=Path(row['path']);name=path.name
 if path.parent.name.startswith('validation-'):name=path.parent.name+'-'+name
 if row.get('excerpt_only'):name+='.excerpt.txt'
 (root/name).write_text(scrub(row.pop('text')))
 row['export']=name
(root/'manifest.json').write_text(json.dumps(rows,indent=2)+'\n')
print('COLLECTED',len(rows),'text artifacts; oversize',sum(bool(r.get('excerpt_only')) for r in rows))
