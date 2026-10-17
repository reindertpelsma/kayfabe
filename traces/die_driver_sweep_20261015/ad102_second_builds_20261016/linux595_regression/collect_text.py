import json,re,subprocess
from pathlib import Path
root=Path('/workspace/codex/blackwell-hw/traces/die_driver_sweep_20261015/ad102_second_builds_20261016/linux595_regression');root.mkdir(exist_ok=True)
program=r'''
import hashlib,json,pathlib,re,subprocess
files=list(pathlib.Path('/workspace/bench').glob('fast_codex_ada2build_full595_*.log'))
files+=list(pathlib.Path('/root/prov/ada2build-fast-receipts').glob('*.json'))
files+=list(pathlib.Path('/root/prov/ada2build-fast-receipts').glob('*.FAILED'))
files+=[pathlib.Path('/workspace/bench')/n for n in ['codex_ada2build_full595_suite.out','cl_codex_ada2build_native595_host.out']]
files+=[pathlib.Path('/root/prov')/n for n in ['ada2build_raw.log','ada2build_births.log','ada2build_gates.log','ada2build_linux.log','ada2build_native_cuda.log','ada2build_native_caps.txt','ada2build_variants.log','ada2build_caps00c_gates.log','ada2build_lpwr2801_gates.log','ada2build_lpwr2806_gates.log','ada2build_payload_hashes.txt']]
rows=[];zero=nonzero=0;arms=[]
for p in files:
 if not p.is_file():continue
 data=p.read_bytes();row={'name':p.name,'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
 if len(data)<=1024*1024:row['text']=data.decode(errors='replace')
 else:row['text']=data[:16384].decode(errors='replace')+'\n[oversize raw omitted; retained until retirement]\n'+data[-32768:].decode(errors='replace');row['excerpt_only']=True
 rows.append(row)
 if p.name.endswith('_serial.log'):
  lines=[l for l in data.decode(errors='replace').splitlines() if '_kgspIsHeartbeatTimedOut: Heartbeat timed out' in l]
  for l in lines:
   m=re.search(r' heartbeat (\d+) ',l)
   if m and int(m[1])==0:zero+=1
   else:nonzero+=1
  arms.append({'name':p.name,'warnings':len(lines),'samples':lines[:2]})
probe=pathlib.Path('/root/prov/ada2build_die.log').read_text();probe='\n'.join(l for l in probe.splitlines() if l.startswith('PROBE_DIE ') or l.startswith('PROBE_START '))+'\n'
rows.append({'name':'native-die-privilege.txt','text':probe})
receipts=[json.loads(p.read_text()) for p in pathlib.Path('/root/prov/ada2build-fast-receipts').glob('*.json')]
xids=[l for l in subprocess.check_output(['dmesg'],text=True).splitlines() if 'NVRM: Xid' in l];mapped=[]
for line in xids:
 m=re.search(r'pid=(\d+)',line);pid=int(m[1]) if m else None;r=next((r for r in receipts if r['pid']==pid),None)
 arm=next((a for a in ['defer-liveness','missing-page-fault'] if r and '--'+a in ' '.join(r['argv'])),None)
 mapped.append({'line':line,'pid':pid,'raw_arm':arm})
summary={'serial_arms':len(arms),'heartbeat_raw_zero_warnings':zero,'heartbeat_raw_nonzero_warnings':nonzero,'arms':arms,'actual_process_receipts':len(receipts),'host_xids':mapped}
rows.append({'name':'heartbeat-xid-census.json','text':json.dumps(summary,indent=2)+'\n'})
print(json.dumps(rows))
'''
r=subprocess.run(['ssh','-F','/workspace/codex/ssh_config','-o','ForwardAgent=no','codex-ada2build-56302930','python3 -'],input=program,capture_output=True,text=True,timeout=90);r.check_returncode();rows=json.loads(r.stdout)
def scrub(s):
 s=re.sub(r'GPU-[A-Za-z0-9-]+','GPU-REDACTED',s)
 s=re.sub(r'(?i)\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b','UUID-REDACTED',s)
 s=re.sub(r'(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b','MAC-REDACTED',s)
 return re.sub(r'\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|192\.168\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b','PRIVATE-IP-REDACTED',s)
for row in rows:(root/(row['name']+('.excerpt.txt' if row.get('excerpt_only') else ''))).write_text(scrub(row.pop('text')))
(root/'manifest.json').write_text(json.dumps(rows,indent=2)+'\n')
summary=json.loads((root/'heartbeat-xid-census.json').read_text())
print('COLLECTED',len(rows),'text artifacts; HB0',summary['heartbeat_raw_zero_warnings'],'HBnonzero',summary['heartbeat_raw_nonzero_warnings'],'receipts',summary['actual_process_receipts'],'XID_mapping',[(r['pid'],r['raw_arm']) for r in summary['host_xids']])
