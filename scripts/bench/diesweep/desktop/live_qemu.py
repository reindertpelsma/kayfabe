#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""List QEMU processes that prevent the next serial lane phase."""
import subprocess
from pathlib import Path
import sys


def live_qemu():
    result = subprocess.run(['pgrep', '-x', 'qemu-system-x86'], capture_output=True, text=True)
    if result.returncode not in (0, 1):
        raise RuntimeError('process census failed')
    live = []
    for word in result.stdout.split():
        pid = int(word)
        try:
            state = Path('/proc', word, 'stat').read_text().rpartition(')')[2].split()[0]
        except FileNotFoundError:
            continue
        if state not in ('Z', 'X'):
            live.append(pid)
    return live


if __name__ == '__main__':
    pids = live_qemu()
    if '--assert-none' in sys.argv and pids:
        sys.exit('live QEMU prevents next phase: ' + ' '.join(map(str, pids)))
    print(' '.join(map(str, pids)))
