#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Install opt-in diagnostic sources into a compatible QEMU source checkout.

Only named VFIO source files must be pristine. Other changes (including kf3)
are allowed. This script never builds or replaces an executable.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('qemu', type=Path)
    parser.add_argument('--check', action='store_true', help='validate without changing files')
    args = parser.parse_args()
    target = args.qemu.resolve()
    source = Path(__file__).resolve().parent
    if (target / 'VERSION').read_text().strip() != '10.2.4':
        raise ValueError('requires QEMU 10.2.4')
    expected = json.loads((source / 'upstream-sha256.json').read_text())
    for name, digest in expected.items():
        path = target / name
        if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f'upstream source hash mismatch: {name}')
    copies = {p: target / 'hw/vfio' / p.name
              for folder in ('core', 'qemu') for p in (source / folder).iterdir()
              if p.suffix in ('.h', '.c')}
    for dest in copies.values():
        if dest.exists() or dest.is_symlink():
            raise ValueError(f'refusing existing observer destination: {dest}')
    command = ['git', 'apply', '--check', str(source / 'integration.patch')]
    subprocess.run(command, cwd=target, check=True)
    if args.check:
        print('Pinned VFIO sources and patch validated; no files changed')
        return
    # Capture backups before mutation so a copy/write error can roll back.
    backup = {name: (target / name).read_bytes() for name in expected}
    created = []
    try:
        for src, dest in copies.items():
            with dest.open('xb') as stream:
                created.append(dest)
                stream.write(src.read_bytes())
        subprocess.run(['git', 'apply', str(source / 'integration.patch')], cwd=target, check=True)
    except BaseException:
        for name, content in backup.items():
            (target / name).write_bytes(content)
        for path in created:
            path.unlink(missing_ok=True)
        raise
    print(f'Installed diagnostic sources in {target}; no binary was built or replaced')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(f'REFUSED: {error}')
