#!/usr/bin/env python3
"""Trusted host only; caller holds GPU lock. Delete only three completed own test disks."""
import json
from pathlib import Path
import subprocess

w = Path('/var/lib/kf-windows-20261005')
root = w / 'codex-fifo-refusal-20261011-b081f705'
assert not subprocess.run(['pgrep', 'qemu-system'], stdout=subprocess.DEVNULL).returncode == 0
assert (w / 'codex-late-fifo-20261011-89e8ddf8/EXIT').read_text().strip() == '0'
targets = {w / f'boundary-kayfabe-{n}/windows.qcow2' for n in (1490, 1495, 1510)}
protected = [w / 'boundary-kayfabe-1380/windows.qcow2',
             w / 'baseline/windows.qcow2', w / 'owner-state/manual1380/resume.qcow2']
assert all(p.is_file() and p not in targets for p in protected)
# Read every qcow2 header first. Any unreadable header or dependent disk aborts before deletion.
records = []
for p in sorted(w.rglob('*.qcow2')):
    if not p.is_file() or p.is_symlink():
        continue
    d = json.loads(subprocess.check_output(['qemu-img', 'info', '--output=json', str(p)]))
    parent = d.get('full-backing-filename')
    if parent:
        assert Path(parent).resolve() not in targets, (p, parent)
    records.append(dict(path=str(p), parent=parent, allocated=d.get('actual-size')))
assert all(t.is_file() for t in targets)
report = dict(protected=[str(p) for p in protected], checked_headers=len(records),
              removed=[dict(path=str(t), allocated=t.stat().st_blocks * 512) for t in sorted(targets)],
              records=records)
root.mkdir(parents=True, exist_ok=True)
(root / 'overlay-cleanup.json').write_text(json.dumps(report, indent=2) + '\n')
for t in sorted(targets):
    t.unlink()
print(json.dumps(dict(removed=[str(t) for t in sorted(targets)], protected=[str(p) for p in protected])))
