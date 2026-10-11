#!/usr/bin/env python3
"""Preserve the owner's local installed image after shutdown. No cloud/Git upload.
The image is a convenience save, not an irreplaceable/offsite backup.
"""
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

W = Path('/var/lib/kf-windows-20261005')
run = W / 'boundary-kayfabe-1380'
saved = W / 'owner-state/manual1380'
assert subprocess.run(['pgrep', 'qemu-system'], stdout=subprocess.DEVNULL).returncode == 1
assert Path('/sys/kernel/iommu_groups/11/type').read_text().strip() == 'DMA-FQ'
disk = run / 'windows.qcow2'
chain = json.loads(subprocess.check_output(['qemu-img', 'info', '--backing-chain', '--output=json', str(disk)]))
assert [e['filename'] for e in chain] == [str(disk), str(W / 'baseline/windows.qcow2')]
assert all(e['format'] == 'qcow2' for e in chain)
saved.mkdir(parents=True, exist_ok=True)
assert not (saved / 'KEEP.json').exists(), 'preservation already recorded; inspect before changing'
check = subprocess.run(['qemu-img', 'check', str(disk)], capture_output=True, text=True)
(saved / 'qemu-img-check.txt').write_text(check.stdout + check.stderr + f'\nexit={check.returncode}\n')
shutil.copy2(run / 'OVMF_VARS.fd', saved / 'OVMF_VARS.fd')
(saved / 'saved-windows.qcow2').symlink_to(disk)
# This independent writable child is reserved for the owner's next personal boot.
# Benchmark guests continue using their own baseline-backed disposable disks.
subprocess.run(['qemu-img', 'create', '-q', '-f', 'qcow2', '-F', 'qcow2', '-b',
                str(disk), str(saved / 'resume.qcow2')], check=True)
disk.chmod(0o444)
manifest = dict(status='PRESERVED_LOCAL_CONVENIENCE',
                utc=datetime.now(timezone.utc).isoformat(),
                host='172.22.1.20', installed_state=str(disk),
                writable_personal_child=str(saved / 'resume.qcow2'),
                firmware=str(saved / 'OVMF_VARS.fd'),
                firmware_sha256=hashlib.sha256((saved / 'OVMF_VARS.fd').read_bytes()).hexdigest(),
                chain=chain, image_check_exit=check.returncode,
                image_check_log=str(saved / 'qemu-img-check.txt'),
                retention='Never garbage-collect/rebase/mutate original1380 or baseline parent; no test uses this personal chain.',
                privacy='Disk and sign-in state stay on trusted host; no disk/credentials in Git or on Vast.',
                source_marker=(run / 'marker.txt').read_text())
(saved / 'KEEP.json').write_text(json.dumps(manifest, indent=2) + '\n')
(run / 'OWNER_KEEP').write_text(str(saved / 'KEEP.json') + '\n')
print(json.dumps({k: manifest[k] for k in ['status','installed_state','writable_personal_child','image_check_exit']}))
