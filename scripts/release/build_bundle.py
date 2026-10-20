#!/usr/bin/env python3
"""Build generated data referenced by QEMU's bundle before publishing a revision."""
from pathlib import Path
import subprocess
import sys


def complete(build):
    build=Path(build).resolve()
    bundle=build/'qemu-bundle'
    if not bundle.is_dir():
        raise ValueError('QEMU build has no qemu-bundle directory')
    dangling=[path for path in bundle.rglob('*') if path.is_symlink() and not path.exists()]
    outputs=subprocess.check_output(['ninja','-C',str(build),'-t','targets','all'],text=True)
    targets={line.rsplit(': ',1)[0] for line in outputs.splitlines() if ': ' in line}
    required=set()
    for link in dangling:
        target=link.resolve()
        if not target.is_relative_to(build):
            raise ValueError(f'missing bundle data outside the build: {link.name}')
        name=target.relative_to(build).as_posix()
        if name not in targets:
            raise ValueError(f'missing bundle data has no Ninja target: {name}')
        required.add(name)
    if required:
        subprocess.run(['ninja','-j2','-C',str(build),*sorted(required)],check=True)
    if any(not path.exists() for path in dangling):
        raise ValueError('generated bundle data is still missing after Ninja')
    print(f'KAYFABE_BUNDLE_COMPLETE generated_targets={len(required)}')


if __name__=='__main__':complete(sys.argv[1])
