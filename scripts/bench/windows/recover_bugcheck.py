#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""recover_bugcheck.py -- read the bugcheck of one stopped borrowed-PC Windows run.

Research fixture, a sibling of the host's `boundary-tools/recover_pc_watchdog.py` (same layout
rules, same read-only procedure): attach the run's overlay with a read-only NBD at the pinned
baseline's Windows partition offset, mount NTFS read-only, and read

- the first 8 KiB of `pagefile.sys`: a kernel crash dump is written there at bugcheck time and
  only moved to MEMORY.DMP on the NEXT boot, which an experiment never has. The header
  (`PAGE`/`DU64`) carries the bugcheck code and its four parameters;
- the newest `Windows/Minidump/*.dmp` header, if a minidump exists (same fields, `PAGEDU64`);
- copies of `Microsoft-Windows-DxgKrnl%4Admin.evtx` and `System.evtx` (private evidence).

It never starts Windows or a debugger and never writes to the image. Usage (root, on the PC,
after the run's supervisor has stopped): recover_bugcheck.py --run N. Output: the JSON summary
on stdout and in <run>/bugcheck-offline/bugcheck.json. The evtx copies stay private on the PC.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import time

BASE = Path('/var/lib/kf-windows-20261005')
# GPT partition 3 start of the pinned baseline (the same constant as recover_pc_watchdog.py).
WINDOWS_OFFSET = 649216 * 512
HEADER = 8192
EVTX_LIMIT = 64 * 1024 * 1024
LOGS = ('Microsoft-Windows-DxgKrnl%4Admin.evtx', 'System.evtx')


def run(*args):
    return subprocess.run(args, check=True, timeout=30)


def dump_header(data):
    """DUMP_HEADER64 fields: Signature 'PAGE', ValidDump 'DU64', MajorVersion, MinorVersion
    (build), MachineImageType @0x30, NumberProcessors @0x34, BugCheckCode @0x38, parameters
    @0x40/0x48/0x50/0x58."""
    if len(data) < 0x60:
        return {'valid': False, 'reason': 'short'}
    sig, valid = data[0:4], data[4:8]
    if sig != b'PAGE' or valid != b'DU64':
        return {'valid': False, 'signature': data[0:8].hex(),
                'all_zero': not any(data)}
    major, minor = struct.unpack_from('<II', data, 8)
    machine, cpus, code = struct.unpack_from('<III', data, 0x30)
    params = struct.unpack_from('<QQQQ', data, 0x40)
    return {'valid': True, 'signature': 'PAGEDU64', 'major': major, 'build': minor,
            'machine': hex(machine), 'processors': cpus, 'bugcheck_code': hex(code),
            'parameters': [hex(p) for p in params]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=int, choices=range(1, 100), required=True)
    args = parser.parse_args()
    if os.geteuid():
        parser.error('root required for the read-only NBD mount')
    os.umask(0o077)
    name = f'boundary-kayfabe-{args.run}'
    work = BASE / name
    with open('/tmp/kayfabe-fastguest.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        state = subprocess.run(['systemctl', 'show', 'kf-' + name, '-p', 'ActiveState', '--value'],
                               capture_output=True, text=True, check=True, timeout=30).stdout.strip()
        if state != 'inactive' or Path('/sys/block/nbd0/pid').exists():
            raise RuntimeError('Require a stopped supervisor and an unused nbd0')
        image = work / 'windows.qcow2'
        for proc in Path('/proc').glob('[0-9]*/cmdline'):
            try:
                argv = proc.read_bytes().split(b'\0')
            except (FileNotFoundError, ProcessLookupError, PermissionError):
                continue
            if b'qemu-system' in argv[0] and any(str(image).encode() in a for a in argv):
                raise RuntimeError('Experiment image still used by a process')
        out = work / 'bugcheck-offline'
        out.mkdir(mode=0o700)  # refuses to overwrite earlier evidence
        mount = out / 'mnt'
        mount.mkdir(mode=0o700)
        summary = {'schema': 'kayfabe-bugcheck-recovery/1', 'run': args.run,
                   'disk_access': 'read-only NBD and NTFS; supervisor stopped'}
        attached = False
        try:
            run('modprobe', 'nbd', 'max_part=16')
            attached = True
            run('qemu-nbd', '--read-only', '--format=qcow2', '--offset', str(WINDOWS_OFFSET),
                '--connect=/dev/nbd0', str(image))
            deadline = time.monotonic() + 5
            while int(Path('/sys/block/nbd0/size').read_text()) == 0:
                if time.monotonic() >= deadline:
                    raise RuntimeError('NBD capacity did not become visible')
                time.sleep(0.1)
            run('ntfs-3g', '-o', 'ro', '/dev/nbd0', str(mount))
            root = mount.resolve()

            def inside(p):
                return not p.is_symlink() and p.resolve().is_relative_to(root)

            page = mount / 'pagefile.sys'
            if page.is_file() and inside(page):
                with page.open('rb') as f:
                    summary['pagefile'] = dump_header(f.read(HEADER))
            else:
                summary['pagefile'] = {'valid': False, 'reason': 'absent'}
            mini = mount / 'Windows/Minidump'
            dumps = sorted((p for p in mini.glob('*.dmp') if p.is_file() and inside(p)),
                           key=lambda p: p.stat().st_mtime)[-1:] if mini.is_dir() and inside(mini) else []
            summary['minidump'] = None
            for p in dumps:
                with p.open('rb') as f:
                    summary['minidump'] = dict(dump_header(f.read(HEADER)), file=p.name)
            logs = mount / 'Windows/System32/winevt/Logs'
            for name_ in LOGS:
                p = logs / name_
                if p.is_file() and inside(p) and p.stat().st_size <= EVTX_LIMIT:
                    shutil.copyfile(p, out / name_)
                    summary.setdefault('evtx_copied', []).append(name_)
        finally:
            if os.path.ismount(mount):
                run('umount', str(mount))
            if attached and Path('/sys/block/nbd0/pid').exists():
                run('qemu-nbd', '--disconnect', '/dev/nbd0')
                deadline = time.monotonic() + 5
                while Path('/sys/block/nbd0/pid').exists():
                    if time.monotonic() >= deadline:
                        raise RuntimeError('NBD disconnect not confirmed')
                    time.sleep(0.1)
            mount.rmdir()
        summary['cleanup_verified'] = True
        (out / 'bugcheck.json').write_text(json.dumps(summary, indent=2) + '\n')
        print(json.dumps(summary))


if __name__ == '__main__':
    main()
