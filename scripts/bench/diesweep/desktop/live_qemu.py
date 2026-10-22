#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""List QEMU processes that prevent the next serial lane phase."""
import subprocess


def live_qemu():
    result = subprocess.run(['pgrep', '-x', 'qemu-system-x86'], capture_output=True, text=True)
    if result.returncode not in (0, 1):
        raise RuntimeError('process census failed')
    return [int(x) for x in result.stdout.split()]


if __name__ == '__main__':
    print(' '.join(map(str, live_qemu())))
