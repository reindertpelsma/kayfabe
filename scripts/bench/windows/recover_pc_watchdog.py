#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Read one fresh WATCHDOG dump from a stopped borrowed-PC experiment.

Research fixture only: fixed Windows partition offset of the pinned baseline.
Runs on the Linux PC; never starts Windows or a debugger. NBD and NTFS are both
read-only. The output is private evidence, never suitable for a public commit.
"""
import argparse
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time

BASE = Path('/var/lib/kf-windows-20261005')
BASELINE_SHA = '9ff194d31f0a871757fd436f14ab0da2e1e8a6360dc36bb925c12af12a64f85c'
# GPT partition3 start, verified from this immutable baseline (512-byte sectors).
WINDOWS_OFFSET = 649216 * 512
LIMIT = 8 * 1024 * 1024


def run(*args, **kwargs):
    return subprocess.run(args, check=True, timeout=30, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=int, choices=range(1, 100), required=True)
    args = parser.parse_args()
    if os.geteuid():
        parser.error('root required for read-only NBD mount')
    os.umask(0o077)
    name = f'boundary-kayfabe-{args.run}'
    work = BASE / name
    with open('/tmp/kayfabe-fastguest.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        state = run('systemctl', 'show', 'kf-'+name, '-p', 'ActiveState',
                    '--value', capture_output=True, text=True).stdout.strip()
        if state != 'inactive' or Path('/sys/block/nbd0/pid').exists():
            raise RuntimeError('Require stopped supervisor and unused nbd0')
        image = work / 'windows.qcow2'
        for proc in Path('/proc').glob('[0-9]*/cmdline'):
            try:
                argv = proc.read_bytes().split(b'\0')
                if b'qemu-system' in argv[0] and any(str(image).encode() in a for a in argv):
                    raise RuntimeError('Experiment image still used by a process')
            except (FileNotFoundError, ProcessLookupError):
                pass
        command = json.loads((work / 'command.json').read_text())
        if command['baseline_sha256'] != BASELINE_SHA:
            raise RuntimeError('Unknown baseline layout')
        with (BASE / 'baseline/windows.qcow2').open('rb') as stream:
            if hashlib.file_digest(stream, 'sha256').hexdigest() != BASELINE_SHA:
                raise RuntimeError('Baseline changed')
        start = datetime.datetime.fromisoformat(command['time_utc']).timestamp()
        out = work / 'watchdog-offline'
        out.mkdir(mode=0o700)  # Refuse overwriting previous evidence.
        mount = out / 'mnt'
        mount.mkdir(mode=0o700)
        attach_attempted = False
        launcher_pending = False
        receipt = None
        def interrupted(*_):
            raise KeyboardInterrupt()
        for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
            signal.signal(sig, interrupted)
        try:
            run('modprobe', 'nbd', 'max_part=16')
            attach_attempted = True
            launcher_pending = True
            run('qemu-nbd', '--read-only', '--format=qcow2', '--offset',
                str(WINDOWS_OFFSET), '--connect=/dev/nbd0', str(image))
            launcher_pending = False
            deadline = time.monotonic() + 5
            while int(Path('/sys/block/nbd0/size').read_text()) == 0:
                if time.monotonic() >= deadline:
                    raise RuntimeError('NBD capacity did not become visible')
                time.sleep(0.1)
            launcher_pending = True
            run('ntfs-3g', '-o', 'ro', '/dev/nbd0', str(mount))
            launcher_pending = False
            folder = mount / 'Windows/LiveKernelReports/WATCHDOG'
            if any(part.is_symlink() for part in (mount / 'Windows',
                   mount / 'Windows/LiveKernelReports', folder)):
                raise RuntimeError('Dump directory path contains a symlink')
            if not folder.resolve().is_relative_to(mount.resolve()):
                raise RuntimeError('Dump folder escapes the mounted filesystem')
            candidates = []
            for index, src in enumerate(folder.glob('WATCHDOG-*.dmp')):
                if index >= 16:
                    raise RuntimeError('Too many dump candidates')
                if not re.fullmatch(r'WATCHDOG-\d{8}-\d{4}\.dmp', src.name):
                    raise RuntimeError('Unexpected dump filename')
                if (src.is_symlink() or not src.is_file()
                        or not src.resolve().is_relative_to(mount.resolve())):
                    raise RuntimeError('Dump must be an ordinary file')
                stat = src.stat()
                if stat.st_mtime >= start:
                    candidates.append((src, stat))
            if len(candidates) != 1:
                raise RuntimeError('Require exactly one fresh dump from this boot')
            src, stat = candidates[0]
            if not 0 < stat.st_size <= LIMIT:
                raise RuntimeError('Dump exceeds fixed read bound')
            dest = out / 'watchdog.dmp'
            with src.open('rb') as stream:
                data = stream.read(LIMIT + 1)
            if len(data) != stat.st_size:
                raise RuntimeError('Dump changed or exceeded its declared size')
            with dest.open('xb') as stream:
                stream.write(data)
            receipt = dict(schema='kayfabe-private-watchdog-recovery/1', run=args.run,
                           filename=src.name, bytes=stat.st_size,
                           sha256=hashlib.sha256(data).hexdigest(),
                           dump_mtime_utc=datetime.datetime.fromtimestamp(
                               stat.st_mtime, datetime.timezone.utc).isoformat(),
                           experiment_start_utc=command['time_utc'],
                           disk_access='read-only NBD and NTFS; supervisor stopped',
                           baseline_sha256=BASELINE_SHA, windows_offset=WINDOWS_OFFSET)
        finally:
            for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
                signal.signal(sig, signal.SIG_IGN)
            if launcher_pending:
                (out / 'manual-cleanup-required.txt').write_text(
                    'Launcher did not finish. A daemon may attach after inspection; '
                    'verify processes, mount and NBD manually before another run.\n')
            try:
                # A daemon may attach/mount before its launcher times out. Inspect
                # actual state even when the launch did not return successfully.
                if os.path.ismount(mount):
                    run('umount', str(mount))
                if os.path.ismount(mount):
                    raise RuntimeError('Read-only mount remains; do not detach NBD')
                if attach_attempted and Path('/sys/block/nbd0/pid').exists():
                    run('qemu-nbd', '--disconnect', '/dev/nbd0')
                    deadline = time.monotonic() + 5
                    while Path('/sys/block/nbd0/pid').exists():
                        if time.monotonic() >= deadline:
                            raise RuntimeError('NBD disconnect not confirmed')
                        time.sleep(0.1)
                mount.rmdir()
                if launcher_pending:
                    raise RuntimeError('Daemon startup uncertain; manual cleanup check required')
            except Exception as error:
                (out / 'cleanup-error.txt').write_text(str(error)+'\n')
                raise
        receipt['cleanup_verified'] = True
        (out / 'receipt.json').write_text(json.dumps(receipt, indent=2)+'\n')
        print(json.dumps(receipt))


if __name__ == '__main__':
    main()
