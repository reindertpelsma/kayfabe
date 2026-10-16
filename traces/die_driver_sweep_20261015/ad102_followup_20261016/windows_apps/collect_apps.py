import hashlib,json,re,subprocess
from pathlib import Path

out=Path('/workspace/codex/blackwell-hw/traces/die_driver_sweep_20261015/ad102_followup_20261016/windows_apps')
out.mkdir(exist_ok=True)
program=r'''
import hashlib,json,pathlib,subprocess
prov=pathlib.Path('/root/prov');run=pathlib.Path('/root/win/codex-adaref2')
names=['adaref2_persistent_apps.log','adaref2_windows_apps_stage.log',
       'adaref2_windows_apps_stage_sync_failure.log','adaref2_windows_apps_null_exitcode.log',
       'adaref2_windows_apps_wave_sync_failure.log','adaref2_apps_exitcode_retry.log',
       'adaref2_windows_apps_wave.log','adaref2_stop_77a72da9.log']
files=[prov/n for n in names]+[run/'process-receipt-110767.json',run/'flags.txt',run/'marker.txt']
rows=[]
for p in files:
 data=p.read_bytes();assert len(data)<=1024*1024
 rows.append({'name':p.name,'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest(),'text':data.decode(errors='replace')})
q=run/'qemu.log';data=q.read_bytes()
xids=[s for s in subprocess.check_output(['dmesg'],text=True).splitlines() if 'NVRM: Xid' in s]
meta={'qemu_bytes':len(data),'qemu_sha256':hashlib.sha256(data).hexdigest(),
      'qemu_export':'raw log stayed on rental; no bodies exported',
      'host_xids_after':len(xids),'host_xids_before':5,'host_xid_delta':len(xids)-5,
      'qpid_exists':(run/'qpid').exists(),'qemu_110767_alive':pathlib.Path('/proc/110767').exists(),
      'controller_110740_alive':pathlib.Path('/proc/110740').exists()}
print(json.dumps({'files':rows,'metadata':meta}))
'''
r=subprocess.run(['ssh','-F','/workspace/codex/ssh_config','-o','ForwardAgent=no','codex-adaref2-56267214','python3 -'],input=program,capture_output=True,text=True,timeout=60)
r.check_returncode();result=json.loads(r.stdout)
def scrub(s):
 s=re.sub(r'GPU-[A-Za-z0-9-]+','GPU-REDACTED',s)
 s=re.sub(r'(?i)\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b','UUID-REDACTED',s)
 s=re.sub(r'(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b','MAC-REDACTED',s)
 def ip(m):return 'IP-REDACTED' if all(int(n)<=255 for n in m.group().split('.')) else m.group()
 return re.sub(r'(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])',ip,s)
manifest=[]
for row in result['files']:
 name=row['name'];assert Path(name).name==name
 (out/name).write_text(scrub(row.pop('text')))
 row['export']='screened complete text';manifest.append(row)
assert result['metadata']['host_xid_delta']==0
assert not any(result['metadata'][k] for k in ['qpid_exists','qemu_110767_alive','controller_110740_alive'])
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
(out/'post_stop_metadata.json').write_text(json.dumps(result['metadata'],indent=2)+'\n')
print('COLLECTED_WINDOWS_APPS_TEXT',len(manifest),result['metadata'])
