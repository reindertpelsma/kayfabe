#!/usr/bin/env python3
"""PC-specific Windows/Kayfabe experiment; one fresh overlay per run.

The immutable baseline disk and virtual OVMF variables must already be staged.
No physical GPU is unbound. The experiment writes only its new run directory.
"""
import argparse
import datetime
import fcntl
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--revision', required=True)
parser.add_argument('--name', required=True)
parser.add_argument('--pool-probe', action='store_true')
parser.add_argument('--base', type=Path, default=Path('/var/lib/kf-windows-20261005'))
args = parser.parse_args()
if not re.fullmatch('[0-9a-f]{8,40}', args.revision):
    parser.error('revision must identify the immutable build')
if not re.fullmatch('[a-z0-9-]{1,64}', args.name):
    parser.error('invalid run name')
os.umask(0o077)
base = args.base.resolve()
work = base / args.name
source = base / 'baseline/windows.qcow2'
qemu = base / 'kf3-bins' / args.revision[:8] / 'qemu-system-x86_64'
with open('/tmp/kayfabe-fastguest.lock', 'a') as lock:
 fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
 if work.exists(): raise RuntimeError('Run directory already exists; choose a fresh name')
 if not qemu.is_file(): raise RuntimeError('Pinned QEMU build missing')
 if not source.is_file(): raise RuntimeError('Staged immutable baseline missing')
 work.mkdir(mode=0o700)
 subprocess.run(['qemu-img', 'create', '-f', 'qcow2', '-F', 'qcow2', '-b', str(source), str(work/'windows.qcow2')], check=True)
 shutil.copyfile(base/'baseline/OVMF_VARS.fd', work/'OVMF_VARS.fd')
 cmd=[str(qemu),'-name','kayfabe-windows-'+args.name,'-nodefaults','-no-user-config',
 '-machine','pc,accel=kvm,smm=on,memory-backend=ram0','-object','memory-backend-memfd,id=ram0,size=8G,share=on',
 '-m','8192','-smp','8','-cpu','host,-vmx',
 '-drive','if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd',
 '-drive',f'if=pflash,format=raw,unit=1,file={work}/OVMF_VARS.fd',
 '-rtc','base=utc,driftfix=slew','-global','kvm-pit.lost_tick_policy=discard',
 '-device','VGA,addr=0x9','-display','none','-vnc',f'unix:{work}/vnc.sock',
 '-qmp',f'unix:{work}/qmp.sock,server=on,wait=off','-serial',f'file:{work}/serial.log',
 '-monitor','none','-action','reboot=shutdown,panic=none',
 '-netdev','user,id=n0,restrict=off,net=10.0.2.0/24,host=10.0.2.2,dns=10.0.2.3,dhcpstart=10.0.2.15,hostfwd=tcp:127.0.0.1:22225-10.0.2.15:22',
 '-device','virtio-net-pci,netdev=n0,mac=52:54:00:56:77:20,addr=0xa,disable-modern=on',
 '-device','virtio-serial-pci,id=serial0,addr=0x3,disable-modern=on',
 '-chardev',f'socket,id=qga0,path={work}/qga.sock,server=on,wait=off',
 '-device','virtserialport,bus=serial0.0,chardev=qga0,name=org.qemu.guest_agent.0',
 '-drive',f'file={work}/windows.qcow2,if=none,id=disk0,format=qcow2,discard=unmap',
 '-device','virtio-blk-pci,drive=disk0,addr=0x4,disable-modern=on,bootindex=1',
 '-global','i440FX-pcihost.pci-hole64-size=32G',
 '-device','kf3-gpu,fb-mb=4096,bar1-size=134217728,bar2-size=33554432,display=on,guest-driver=580.65.06,bus=pci.0,addr=0x6,id=kf0']
 env = dict(os.environ, KF3_RPC_TRACE='1')
 env.pop('KF3_GFX_POOL_PROBE', None)
 if args.pool_probe: env['KF3_GFX_POOL_PROBE'] = '1'
 (work/'command.json').write_text(json.dumps({
     'revision': args.revision, 'argv': cmd, 'pool_probe': args.pool_probe,
     'time_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
     'backing_image': str(source), 'vfio': False,
 }, indent=2)+'\n')
 print('KAYFABE_WINDOWS_START', args.name, flush=True)
 with (work/'qemu.log').open('wb') as out:
  result = subprocess.run(cmd, env=env, stdout=out, stderr=subprocess.STDOUT)
 print('KAYFABE_WINDOWS_EXIT='+str(result.returncode), flush=True)
 sys.exit(result.returncode)
