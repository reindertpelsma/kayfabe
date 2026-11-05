from pathlib import Path
import datetime
import json
import subprocess
import time

D = Path(__file__).resolve().parent
alias = (D / 'alias').read_text().strip()
conf = D / 'ssh.conf'
manifest = json.loads((D / 'media_manifest.json').read_text())
assert manifest['all_verified'] and len(manifest['files']) == 4
started = datetime.datetime.now(datetime.timezone.utc).isoformat()
deadline = time.monotonic() + 1200
base = '/workspace/bw-capc-validation/cache'
ssh = ['ssh', '-F', str(conf), alias]
subprocess.run(ssh + [f'install -d -m 0755 {base}'], check=True, timeout=20)
for row in manifest['files']:
    name = row['file']
    assert '/' not in name and all(c.isalnum() or c in '._-' for c in name)
    remaining = int(deadline - time.monotonic())
    assert remaining > 0, 'total media upload budget expired'
    print('MEDIA_UPLOAD_BEGIN', name, row['bytes'], flush=True)
    with (D / 'media' / name).open('rb') as src:
        subprocess.run(
            ssh + [f'umask 077; cat > {base}/{name}.part'],
            stdin=src, check=True, timeout=remaining,
        )
    print('MEDIA_UPLOAD_SENT', name, flush=True)
verify = r'''
from pathlib import Path
import hashlib,json,datetime
base=Path('/workspace/bw-capc-validation/cache')
m=json.loads(Path('/root/sweep2/media_manifest.json').read_text())
assert m['all_verified'] and len(m['files'])==4
out=[]
for row in m['files']:
 name=row['file'];assert Path(name).name==name
 p=base/(name+'.part');assert p.stat().st_size==row['expected_bytes']
 h=hashlib.sha256()
 with p.open('rb') as f:
  for block in iter(lambda:f.read(8*1024*1024),b''):h.update(block)
 sha=h.hexdigest()
 assert sha==row['expected_sha256'],name
 out.append({'file':name,'bytes':p.stat().st_size,'sha256':sha})
for row in out:
 (base/(row['file']+'.part')).rename(base/row['file'])
r={'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
   'all_four_original_media_verified':True,'files':out,
   'next':'unmodified win_vm.sh fetch re-verifies the same pins'}
Path('/root/prov').mkdir(exist_ok=True)
Path('/root/prov/media_cache_receipt.json').write_text(json.dumps(r,indent=2)+'\n')
print(json.dumps(r))
'''
r = subprocess.run(ssh + ['python3 -I -'], input=verify, text=True,
                   capture_output=True, check=True, timeout=60)
receipt = json.loads(r.stdout)
receipt['upload_started_utc'] = started
(D / 'media_cache_receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
print('CACHED_MEDIA_ALL_VERIFIED', flush=True)
