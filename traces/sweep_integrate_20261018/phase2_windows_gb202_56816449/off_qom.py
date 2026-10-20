from pathlib import Path
import subprocess,json,datetime
base=['python3','/root/kayfabe/scripts/bench/windows/qmp.py','/workspace/sweep-win-56816449/kfwin/run/qmp.sock','cmd','qom-get']
rows={}
for prop in ['display','gop','gop-efi','guest-driver']:
 r=subprocess.run(base+[json.dumps({'path':'/machine/peripheral/kf0','property':prop})],capture_output=True,text=True,timeout=15,check=True)
 rows[prop]=json.loads(r.stdout)
assert rows=={'display':False,'gop':False,'gop-efi':'','guest-driver':'580.65.06'},rows
j={'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'actual_QOM_properties':rows,'source':'24906bf334665c9000b16b271005593daa3586c7','limit':'No Code43/TDR/deviceinit verdict inferred from these properties'}
Path('/root/prov/bw580_off_no_rom_effective_qom.json').write_text(json.dumps(j,indent=2)+'\n');print(json.dumps(j))
