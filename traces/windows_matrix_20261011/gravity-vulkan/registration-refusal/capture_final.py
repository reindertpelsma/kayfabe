import datetime,gzip,json,shutil,subprocess
from pathlib import Path
w=Path('/var/lib/kf-windows-20261005')
root=w/'codex-fifo-refusal-20261011-b081f705'
run=w/'boundary-kayfabe-1540'
out=root/'refusal'
assert (root/'EXIT').read_text().strip()=='0'
assert subprocess.run(['pgrep','qemu-system'],stdout=subprocess.DEVNULL).returncode!=0
subprocess.run(['python3','/tmp/kf-summarize-fifo-refusal.py',str(run/'qemu.log'),str(out/'qemu-observation')],check=True)
with (run/'qemu.log').open('rb') as source,gzip.open(out/'qemu.log.gz','wb') as target:
 shutil.copyfileobj(source,target)
for source,target in [(run/'command.json',out/'launch.json'),(run/'marker.txt',out/'marker.txt'),(w/'codex-fifo-refusal-campaign.log',root/'campaign.log')]:
 shutil.copy2(source,target)
now=datetime.datetime.now(datetime.timezone.utc)
j=subprocess.run(['journalctl','-k','--since','2026-10-11 22:58:25 UTC','--until',now.isoformat(),'--no-pager','-o','short-iso'],capture_output=True,text=True)
assert j.returncode==0,j.stderr
(out/'kernel-journal-xids.txt').write_text('\n'.join(l for l in j.stdout.splitlines() if 'Xid' in l)+'\n')
(out/'host-window.json').write_text(json.dumps(dict(start='2026-10-11 22:58:25 UTC',end=now.isoformat(),journal_query_rc=j.returncode,xids=sum('Xid' in l for l in j.stdout.splitlines())),indent=2)+'\n')
lock=subprocess.run(['flock','-n','/tmp/kayfabe-fastguest.lock','true'])
report=dict(at=now.isoformat(),qemu_running=False,gpu_lock_available=lock.returncode==0,
 iommu_group11=(Path('/sys/kernel/iommu_groups/11/type').read_text().strip()),
 stop_file_present=Path('/tmp/kf-stop-winprod').exists(),available_bytes=shutil.disk_usage('/var/lib').free,
 personal_images={str(p):dict(exists=p.exists(),mode=oct(p.stat().st_mode & 0o777)) for p in [w/'boundary-kayfabe-1380/windows.qcow2',w/'baseline/windows.qcow2',w/'owner-state/manual1380/resume.qcow2']})
(root/'post-check.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report))
