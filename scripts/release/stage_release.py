#!/usr/bin/env python3
"""Stage a private, version-consistent local install layout; never publish binaries."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

root=Path(__file__).resolve().parents[2]
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--qemu-bin',type=Path,required=True)
p.add_argument('--broker-bin',type=Path,required=True)
p.add_argument('--out',type=Path,required=True)
args=p.parse_args()
version=(root/'VERSION').read_text().strip()
if args.out.exists():p.error('--out must not already exist')
if subprocess.check_output(['git','status','--porcelain'],cwd=root,text=True).strip():
    p.error('refuse a dirty checkout; commit and review before staging')
for binary,options in [(args.qemu_bin,['-device','help']),(args.broker_bin,['--version'])]:
    result=subprocess.run([str(binary.resolve()),*options],capture_output=True,text=True,check=True)
    if 'kayfabe '+version not in result.stdout+result.stderr:
        p.error(f'{binary}: version does not match canonical VERSION')
for name in ['pc-bios','qemu-bundle']:
    if not (args.qemu_bin.parent/name).is_dir():p.error(f'missing firmware/bundle directory {name}')
args.out.mkdir(parents=True)
bin_dir=args.out/'bin';bin_dir.mkdir()
shutil.copy2(args.qemu_bin,bin_dir/'qemu-system-x86_64')
shutil.copy2(args.broker_bin,bin_dir/'nvkvm-display-broker')
for name in ['pc-bios','qemu-bundle']:
    shutil.copytree(args.qemu_bin.parent/name,bin_dir/name,symlinks=False)
for name in ['kayfabe.py','kayfabe-run.sh','kayfabe-doctor.sh','kayfabe-install.sh','kayfabe-uninstall.sh']:
    shutil.copy2(root/'scripts'/name,bin_dir/name)
shutil.copy2(root/'VERSION',args.out/'VERSION')
for name in ['LICENSE','LICENSE-APACHE','LICENSE-GPL']:
    shutil.copy2(root/name,args.out/name)
(args.out/'docs').mkdir()
for name in ['INSTALL.md','RUNNING_KAYFABE.md','SUPPORT_MATRIX.md','SUPPORT_MATRIX.tsv','WINDOWS_GUEST_SETUP.md']:
    shutil.copy2(root/'docs'/name,args.out/'docs'/name)
manifest=dict(version=version,source_sha=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),
              status='BETA CANDIDATE; unsigned local staging; no external support',
              files={str(path.relative_to(args.out)):hashlib.sha256(path.read_bytes()).hexdigest()
                     for path in args.out.rglob('*') if path.is_file()})
(args.out/'release.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(args.out.resolve())
