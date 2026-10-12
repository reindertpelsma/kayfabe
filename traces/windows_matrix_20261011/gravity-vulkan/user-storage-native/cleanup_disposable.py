#!/usr/bin/env python3
"""Delete exactly three published, reproducible, own native-reference overlays.
Caller holds /tmp/kayfabe-fastguest.lock; every backing link is checked first.
"""
import datetime,json,pathlib,subprocess,os
w=pathlib.Path('/var/lib/kf-windows-20261005')
targets=[w/'codex-vfio-ring-p0-20261012/retry-1/vfio/disk/windows.qcow2',w/'codex-vfio-userd-cursors-20261012/vfio/disk/windows.qcow2',w/'codex-vfio-userd-cursors-reviewed-20261012/vfio/disk/windows.qcow2']
keeps=[w/'boundary-kayfabe-1380/windows.qcow2',w/'baseline/windows.qcow2',w/'owner-state/manual1380/resume.qcow2']
def stat(p):
    s=p.stat();return dict(path=str(p),inode=s.st_ino,size=s.st_size,mtime_ns=s.st_mtime_ns,mode=s.st_mode)
def free():return os.statvfs(w).f_bavail*os.statvfs(w).f_frsize
assert subprocess.run(['pgrep','-f','[q]emu-system'],stdout=subprocess.DEVNULL).returncode==1
before=[stat(p) for p in keeps]; infos={}
for p in w.rglob('*.qcow2'):
    d=json.loads(subprocess.check_output(['qemu-img','info','--output=json',str(p)],text=True,timeout=10))
    backing=d.get('full-backing-filename')
    if backing:backing=str((p.parent/backing).resolve()) if not pathlib.Path(backing).is_absolute() else str(pathlib.Path(backing).resolve())
    infos[str(p.resolve())]=backing
plan=[]
for p in targets:
    name=str(p.resolve());assert name in infos
    children=[k for k,v in infos.items() if v==name];assert not children,(name,children)
    assert p.resolve() not in [k.resolve() for k in keeps]
    plan.append(dict(path=name,children=children,allocated_bytes=p.stat().st_blocks*512))
start_free=free()
for p in targets:p.unlink()
after=[stat(p) for p in keeps];assert before==after
print(json.dumps(dict(utc=str(datetime.datetime.now(datetime.timezone.utc)),scanned=len(infos),deleted=plan,keep_before=before,keep_after=after,free_before=start_free,free_after=free()),indent=2))
