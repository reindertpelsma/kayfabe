import json,subprocess
from pathlib import Path
w=Path('/var/lib/kf-windows-20261005');root=w/'codex-user-shadow-windows-d8afb1e3c'
assert subprocess.run(['pgrep','qemu-system'],stdout=subprocess.DEVNULL).returncode!=0
assert (root/'EXIT').read_text().strip()=='0'
targets=[w/f'boundary-kayfabe-{n}/windows.qcow2' for n in [1610,1611]]
protected=[w/'boundary-kayfabe-1380/windows.qcow2',w/'baseline/windows.qcow2',w/'owner-state/manual1380/resume.qcow2']
assert all(p.is_file() and all(p.resolve()!=t.resolve() for t in targets) for p in protected)
assert all(t.is_file() and not t.is_symlink() for t in targets)
records=[]
for p in sorted(w.rglob('*.qcow2')):
 if not p.is_file() or p.is_symlink():continue
 d=json.loads(subprocess.check_output(['qemu-img','info','--output=json',str(p)]))
 parent=d.get('full-backing-filename')
 if parent:assert all(Path(parent).resolve()!=t.resolve() for t in targets),(p,parent)
 records.append(dict(path=str(p),parent=parent,allocated=d.get('actual-size')))
for short in ['0578a7ab8','30c887c86','4df9c0095','4ec9f9a24','6ff5aaf29','7f3f9fe16','a38602015','b50774a7a']:
 d=w/('codex-memory-access-linux-'+short)
 assert (d/'EXIT').read_text().strip()=='0'
 for name in ['initrd.cpio.gz','vmlinuz']:
  p=d/'fastguest'/name
  assert p.is_file() and not p.is_symlink()
  targets.append(p)
report=dict(protected=[str(p) for p in protected],removed=[dict(path=str(t),allocated=t.stat().st_blocks*512) for t in targets],checked_headers=len(records),headers=records)
(root/'cleanup.json').write_text(json.dumps(report,indent=2)+'\n')
for t in targets:t.unlink()
print('REMOVED_COMPLETED_OWN_REGENERATABLE',len(targets),'bytes',sum(r['allocated'] for r in report['removed']))
