# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Resource identity checks before acting on a recorded PID."""
from pathlib import Path


def is_qemu(argv, number):
    if not argv or not Path(argv[0]).name.startswith('qemu-system-'):
        return False
    try:
        return argv[argv.index('-name')+1]==f'boundary-kayfabe-{number}'
    except (ValueError,IndexError):
        return False


def is_collector(argv, environ, root):
    if not argv or environ.get('N1_ROOT')!=root:
        return False
    return Path(argv[0]).name=='nvidia-smi' or (len(argv)>1 and Path(argv[1]).parent==Path(root) and 'userd_probe' in Path(argv[1]).name)
