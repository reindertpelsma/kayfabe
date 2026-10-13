import tarfile,io,gzip,subprocess,sys,json
from pathlib import Path
short,rootname,*bases=sys.argv[1:]
w=Path('/var/lib/kf-windows-20261005');root=w/rootname
assert (root/'EXIT').is_file()
assert subprocess.run(['pgrep','qemu-system'],stdout=subprocess.DEVNULL).returncode!=0
end=subprocess.check_output(['date','-Is'],text=True).strip()
with tarfile.open(fileobj=sys.stdout.buffer,mode='w|') as t:
 for name in ['START','EXIT','relay-console.log']:
  t.add(root/name,arcname=name,recursive=False)
 for p in sorted((root/'relay').rglob('*')):
  if p.is_file() and p.suffix in ['.json','.txt','.log','.res','.md','.png']:
   assert p.stat().st_size<64<<20
   t.add(p,arcname='relay/'+str(p.relative_to(root/'relay')),recursive=False)
 for base in bases:
  b=w/('boundary-kayfabe-'+base)
  for name in ['command.json','marker.txt']:
   if (b/name).is_file():t.add(b/name,arcname=base+'/'+name,recursive=False)
  if (b/'qemu.log').is_file():
   data=gzip.compress((b/'qemu.log').read_bytes());i=tarfile.TarInfo(base+'/qemu.log.gz');i.size=len(data);t.addfile(i,io.BytesIO(data))
 start=(root/'START').read_text().strip()
 data=subprocess.check_output(['journalctl','-k','--since',start,'--until',end,'--no-pager']);data=b'\n'.join(x for x in data.splitlines() if b'NVRM: Xid' in x)+b'\n';i=tarfile.TarInfo('bounded-host-xid.log');i.size=len(data);t.addfile(i,io.BytesIO(data))
