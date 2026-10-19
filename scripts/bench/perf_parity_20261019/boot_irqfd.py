#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Observe the public C FFI vector-to-eventfd getter before a P1 VM starts.

The C device calls kf3_irq_fd at realize and registers that exact descriptor.
Unlike an optimized Rust wrapper, the C ABI call cannot disappear into its caller.
Stop via the printed P1-owned stop file after the active QEMU is ready.
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time

import host_measure


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', required=True, type=Path)
    p.add_argument('--binary', required=True, type=Path)
    p.add_argument('tag')
    a = p.parse_args()
    r = a.root.resolve(strict=True)
    if r != Path('/data/codex-P1') or not re.fullmatch(r'[A-Za-z0-9_-]+', a.tag):
        p.error('P1 root and safe tag required')
    host_measure.require_lease(r)
    if subprocess.run(['pgrep', '-x', 'qemu-system-x86'], capture_output=True).stdout:
        p.error('attach before starting the one P1 VM')
    binary = a.binary.resolve(strict=True)
    if any(c in str(binary) for c in ':\n"'):
        p.error('unsafe binary path')
    d = r / (a.tag + '_irqfd')
    d.mkdir(exist_ok=False)
    program = (f'uprobe:{binary}:kf3_irq_fd {{ @v[pid,tid] = arg1 + 1; }}\n'
               f'uretprobe:{binary}:kf3_irq_fd /@v[pid,tid]/ {{ '
               'printf("I %llu %d %u %d\\n", nsecs, pid, @v[pid,tid] - 1, retval); delete(@v[pid,tid]); }\n')
    (d / 'trace.bt').write_text(program)
    stop = d / 'STOP'
    with (d / 'trace.txt').open('w') as out, (d / 'trace.err').open('w') as err:
        child = subprocess.Popen(['bpftrace', str(d / 'trace.bt')], stdout=out, stderr=err)
        try:
            time.sleep(3)
            if child.poll() is not None:
                raise RuntimeError('C FFI observer did not attach')
            (d / 'READY').write_text(str(child.pid))
            print('P1_IRQFD_OBSERVER_READY', d, flush=True)
            deadline = time.monotonic() + 300
            while not stop.exists():
                if child.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('observer stopped or was not completed within five minutes')
                time.sleep(.25)
        finally:
            if child.poll() is None:
                child.send_signal(signal.SIGINT)
                child.wait(timeout=15)
    if re.search(r'lost|ERROR|failed', (d / 'trace.err').read_text(), re.I):
        raise RuntimeError('C FFI observer lost records or failed')
    qpid = int((r / 'active-qpid').read_text())
    if Path(f'/proc/{qpid}/exe').resolve(strict=True) != binary:
        raise RuntimeError('active process does not execute this observed binary')
    vectors = {}
    for line in (d / 'trace.txt').read_text().splitlines():
        parts = line.split()
        if len(parts) == 5 and parts[0] == 'I' and int(parts[2]) == qpid:
            vector, fd = int(parts[3]), int(parts[4])
            if fd < 0 or vector in vectors and vectors[vector] != fd:
                raise RuntimeError('inconsistent or refused C FFI descriptor')
            vectors[vector] = fd
    if 0 not in vectors:
        raise RuntimeError('vector zero was not observed; no inferred fd fallback')
    result = {'qpid': qpid, 'proc_start': Path(f'/proc/{qpid}/stat').read_text().split()[21],
              'binary': str(binary), 'method': 'public kf3_irq_fd C ABI entry/return at realize',
              'vectors': vectors, 'irqfd': vectors[0]}
    (r / (a.tag + '_irqfd.json')).write_text(json.dumps(result, indent=2))
    print('P1_IRQFD_IDENTITY', json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
