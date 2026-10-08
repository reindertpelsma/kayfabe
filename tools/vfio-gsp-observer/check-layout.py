#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Check the 580 profile against compiler-derived public TU102 and GA102 layouts."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

source = Path(sys.argv[1]).resolve()
tool = Path(__file__).resolve().parent
expected = json.loads((tool / 'layout-580.json').read_text())
revision = expected.pop('revision')
expected.pop('source')
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip() == revision
roots = ['src/common/sdk/nvidia/inc', 'src/common/uproc/os/common/include',
         'src/nvidia/inc/kernel/gpu/gsp', 'src/common/inc/swref/published/turing/tu102',
         'src/common/inc/swref/published/ampere/ga102']
assert not subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=all',
                                   '--'] + roots, cwd=source), 'layout include roots must be clean'
with tempfile.TemporaryDirectory() as tmp:
    for family in ('turing/tu102', 'ampere/ga102'):
        includes = ['src/common/sdk/nvidia/inc', 'src/common/uproc/os/common/include',
                    'src/nvidia/inc/kernel/gpu/gsp', 'src/common/inc/swref/published/' + family]
        exe = Path(tmp) / 'layout'
        subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror'] +
            ['-I' + str(source / name) for name in includes] +
            [str(tool / 'layout.c'), '-o', str(exe)], check=True)
        assert json.loads(subprocess.check_output([exe])) == expected, family
print('Pinned OGKM TU102/GA102 compiler layouts match the 580 profile')
