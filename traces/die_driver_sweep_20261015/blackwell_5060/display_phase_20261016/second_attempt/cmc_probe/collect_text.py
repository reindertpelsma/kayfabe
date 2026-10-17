import json,re,subprocess
from pathlib import Path
root=Path('/workspace/codex/blackwell-hw/traces/die_driver_sweep_20261015/blackwell_5060/display_phase_20261016/second_attempt/cmc_probe')
program=r'''
import hashlib,json,pathlib
names=['gb206phase_cmc_controller.log','gb206phase_cmc_windows.log','gb206phase_cmc_gates.log','gb206phase_cmc_cpu.log','gb206phase_cmc_build.log','gb206phase_cmc_host_xids.log']
for phase in ['cmc-off','cmc-on']:
 for suffix in ['export.json','posted-status.json','flags.txt','process-receipt.json','census.log','stop.log']:names.append('gb206phase-'+phase+'-'+suffix)
rows=[]
secret=pathlib.Path('/root/win/secrets/pw').read_text().strip()
for name in names:
 p=pathlib.Path('/root/prov')/name
 if not p.is_file():continue
 raw=p.read_bytes();assert len(raw)<=1024*1024
 text=raw.decode(errors='replace')
 if secret:text=text.replace(secret,'PASSWORD-REDACTED')
 rows.append({'name':name,'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest(),'text':text})
for phase in ['cmc-off','cmc-on']:
 p=pathlib.Path('/root/prov')/('gb206phase-'+phase+'-qemu.log')
 lines=[l for l in p.read_text(errors='replace').splitlines() if 'rpc-trace' in l and '20800a89' in l or 'PROBE KF3_CMC_LOG_BUFFER_PROBE=' in l]
 rows.append({'name':phase+'-screened-probe-metadata.txt','text':'\n'.join(lines)+'\n','note':'Only probe startup and policy a89 metadata; no input or address bytes'})
print(json.dumps(rows))
'''
r=subprocess.run(['ssh','-F','/workspace/codex/ssh_config','-o','ForwardAgent=no','codex-gb206phase2-56286918','python3 -'],input=program,text=True,capture_output=True,timeout=90);r.check_returncode()
rows=json.loads(r.stdout)
def scrub(s):
 s=re.sub(r'GPU-[A-Za-z0-9-]+','GPU-REDACTED',s)
 s=re.sub(r'(?i)\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b','UUID-REDACTED',s)
 s=re.sub(r'(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b','MAC-REDACTED',s)
 return s
for row in rows:(root/row['name']).write_text(scrub(row.pop('text')))
(root/'manifest.json').write_text(json.dumps(rows,indent=2)+'\n')
for phase in ['cmc-off','cmc-on']:
 x=json.loads((root/('gb206phase-'+phase+'-export.json')).read_text())
 (root/(phase+'-api-full.tsv')).write_text(x['prefix_tsv'])
 print(phase,x['summary']['policy_metadata_commands'],x['summary']['critical']['0x20800a89'])
 print('\n'.join(x['prefix_tsv'].splitlines()[273:282]),x['prefix_tsv'].splitlines()[-1])
print('COLLECTED',len(rows),'screened artifacts')
