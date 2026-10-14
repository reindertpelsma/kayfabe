#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Verify pinned installation/refusal against source from a local QEMU checkout."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

source = Path(sys.argv[1]).resolve()
tool = Path(__file__).resolve().parents[1]
manifest = json.loads((tool / 'upstream-sha256.json').read_text())
with tempfile.TemporaryDirectory() as tmp:
    target = Path(tmp)
    for name in list(manifest) + ['VERSION']:
        p = target / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(subprocess.check_output(['git', 'show', 'v10.2.4:' + name], cwd=source))
    sentinel = target / 'hw/kf3-unrelated-change'
    sentinel.write_text('preserve me')
    command = [sys.executable, str(tool / 'apply.py'), str(target)]
    def run(*args, good=True):
        result = subprocess.run(command + list(args), text=True, capture_output=True)
        assert (result.returncode == 0) == good, result.stdout + result.stderr
    run('--check')
    region = target / 'hw/vfio/region.c'
    original = region.read_bytes()
    region.write_bytes(original + b'\n/* changed */\n')
    run('--check', good=False)
    region.write_bytes(original)
    occupied = target / 'hw/vfio/observer.c'
    occupied.write_text('must not overwrite')
    run(good=False)
    assert occupied.read_text() == 'must not overwrite'
    occupied.unlink()
    run()
    assert sentinel.read_text() == 'preserve me'
    assert (target / 'hw/vfio/observer.c').read_bytes() == (tool / 'core/observer.c').read_bytes()
    assert 'vfio_gsp_reset(vdev);' in (target / 'hw/vfio/pci.c').read_text()
    run(good=False)
print('Pinned installer application/refusal and unrelated-source preservation passed')
