import json, subprocess
from pathlib import Path
w=Path('/var/lib/kf-windows-20261005')
root=w/'codex-user-worker-4ec9f9a24'
assert subprocess.run(['pgrep','qemu-system'],stdout=subprocess.DEVNULL).returncode != 0
assert (w/'codex-user-worker-windows-a38602015/EXIT').read_text().strip() == '143'
target=w/'boundary-kayfabe-1570/windows.qcow2'
protected=[w/'boundary-kayfabe-1380/windows.qcow2',w/'baseline/windows.qcow2',w/'owner-state/manual1380/resume.qcow2']
assert all(p.is_file() and p.resolve()!=target.resolve() for p in protected)
assert target.is_file() and not target.is_symlink()
records=[]
for p in sorted(w.rglob('*.qcow2')):
 if not p.is_file() or p.is_symlink(): continue
 d=json.loads(subprocess.check_output(['qemu-img','info','--output=json',str(p)]))
 parent=d.get('full-backing-filename')
 if parent: assert Path(parent).resolve()!=target.resolve(), (p,parent)
 records.append(dict(path=str(p),parent=parent,allocated=d.get('actual-size')))
report=dict(protected=[str(p) for p in protected],removed=str(target),allocated=target.stat().st_blocks*512,checked_headers=len(records),records=records)
(root/'overlay-cleanup.json').write_text(json.dumps(report,indent=2)+'\n')
target.unlink()
print('REMOVED_OWN_DISPOSABLE_1570',report['allocated'])
