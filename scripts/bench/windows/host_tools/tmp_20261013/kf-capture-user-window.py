import tarfile,io,gzip,subprocess,sys
from pathlib import Path
base,short,stem,start,end=sys.argv[1:]
w=Path('/var/lib/kf-windows-20261005');root=w/('codex-user-worker-windows-'+short);b=w/('boundary-kayfabe-'+base)
assert (root/'EXIT').read_text().strip()=='143'
with tarfile.open(fileobj=sys.stdout.buffer,mode='w|') as t:
 for p,name in [(Path('/data/'+stem+'.log'),'campaign.log'),(root/'START','START'),(root/'EXIT','EXIT'),(root/'relay-console.log','relay-console.log'),(b/'command.json','command.json'),(b/'marker.txt','marker.txt')]:t.add(p,arcname=name,recursive=False)
 for name in ['session.log','matrix.json','summary.md']:
  if (root/'relay'/name).is_file():t.add(root/'relay'/name,arcname=name,recursive=False)
 data=gzip.compress((b/'qemu.log').read_bytes());i=tarfile.TarInfo('qemu.log.gz');i.size=len(data);t.addfile(i,io.BytesIO(data))
 data=subprocess.check_output(['journalctl','-k','--since',start,'--until',end,'--no-pager']);data=b'\n'.join(x for x in data.splitlines() if b'NVRM: Xid' in x)+b'\n';i=tarfile.TarInfo('bounded-host-xid.log');i.size=len(data);t.addfile(i,io.BytesIO(data))
