import tarfile,sys
from pathlib import Path
w=Path('/var/lib/kf-windows-20261005');short='0578a7ab8'
a=w/('codex-user-worker-'+short);b=w/('codex-memory-access-linux-'+short);c=w/('codex-linux-desktop-'+short)
assert (a/'EXIT').read_text().strip()=='0'
assert (b/'EXIT').read_text().strip()=='0'
assert (c/'EXIT').read_text().strip()=='0'
with tarfile.open(fileobj=sys.stdout.buffer,mode='w|') as t:
 t.add('/data/kf-user-span-linux-'+short+'.log',arcname='campaign.log',recursive=False)
 t.add('/tmp/kf-user-span-linux-'+short+'.sh',arcname='run.sh',recursive=False)
 for p in sorted((a/'native-large').iterdir()):
  if p.is_file(): t.add(p,arcname='native-large/'+p.name,recursive=False)
 for name in ['START','EXIT','binary.sha256','host-xid.log']:
  t.add(b/name,arcname='result/'+name,recursive=False)
 for p in sorted((b/'logs').iterdir()):
  if p.is_file():t.add(p,arcname='result/logs/'+p.name,recursive=False)
 t.add(b/'bench/codexMemoryAccess_suite.out',arcname='result/codexMemoryAccess_suite.out',recursive=False)
 for name in ['START','EXIT','guest-smoke.log','host-xid.log','iommu-after.txt','remaining-qemu.log']:
  t.add(c/name,arcname='desktop/'+name,recursive=False)
