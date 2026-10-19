#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""One tiny probe with owned BPF/fence observers; caller holds the GPU lease.

No broad kill, PID discovery by name, fixed IRQ number, or fixed irqfd number.
The L/J records associate a guest-tree latch with its actual eventfd write.
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

import host_measure
import iteration_hops


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--qpid', type=int, required=True)
    parser.add_argument('--queue', type=int, choices=[0, 2, 3], default=0)
    parser.add_argument('--iterations', type=int, default=3000)
    parser.add_argument('--irqfd-map', type=Path)
    parser.add_argument('--fence', action='store_true')
    parser.add_argument('tag')
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    if root != Path('/data/codex-P1') or not (root / 'GPU_LEASE_HELD').is_file():
        parser.error('P1 lease is required')
    host_measure.require_lease(root)
    if not re.fullmatch(r'[a-zA-Z0-9_-]+', args.tag):
        parser.error('unsafe tag')
    binary = args.binary.resolve(strict=True)
    if Path(f'/proc/{args.qpid}/exe').resolve(strict=True) != binary:
        parser.error('QPID does not execute the requested immutable binary')
    command = Path(f'/proc/{args.qpid}/cmdline').read_bytes().replace(b'\0', b' ').decode()
    if str(root / 'W') not in command:
        parser.error('QPID is not a P1 Windows guest')
    irqfd_identity = None
    if args.irqfd_map:
        irqfd_identity = json.loads(args.irqfd_map.read_text())
        if (irqfd_identity['qpid'] != args.qpid or
                irqfd_identity['proc_start'] != Path(f'/proc/{args.qpid}/stat').read_text().split()[21] or
                Path(irqfd_identity['binary']).resolve(strict=True) != binary):
            raise RuntimeError('C ABI descriptor identity belongs to another process/binary')
        fd = iteration_hops.resolve_irqfd([], irqfd_identity)
        if os.readlink(f'/proc/{args.qpid}/fd/{fd}') != 'anon_inode:[eventfd]':
            raise RuntimeError('observed descriptor is no longer an eventfd')
    destination = root / (args.tag + '_trace')
    destination.mkdir(exist_ok=False)
    (destination / 'command.txt').write_text(command)
    program = subprocess.check_output(
        [sys.executable, str(host_measure.HERE / 'gen_bt.py'), str(binary), str(args.qpid), 'hostpath'],
        text=True,
    )
    symbols = subprocess.check_output(['nm', '--defined-only', str(binary)], text=True).splitlines()
    latch = [line.split()[0] for line in symbols if line.endswith('6Device17latch_and_deliver')]
    if len(latch) != 1:
        raise RuntimeError('latch symbol is not unique')
    address = '0x' + latch[0]
    program += (f'\nuprobe:{binary}:{address} /pid == {args.qpid}/ {{ '
                '@p1_latch[tid] = 1; printf("L %llu %d vector=%u\\n", nsecs, tid, arg1); }\n'
                f'uretprobe:{binary}:{address} /pid == {args.qpid}/ {{ delete(@p1_latch[tid]); }}\n'
                f'tracepoint:syscalls:sys_enter_write /pid == {args.qpid} && @p1_latch[tid]/ {{ '
                'printf("J %llu %d fd=%d n=%d\\n", nsecs, tid, args->fd, args->count); }\n')
    program += (f'tracepoint:syscalls:sys_enter_write /pid == {args.qpid} && args->count == 8/ {{ '
                'printf("K %llu %d %s fd=%d n=%d\\n", nsecs, tid, comm, args->fd, args->count); }\n')
    clocks = [line.split()[0] for line in symbols if line.endswith('9ChanPlane10nsi_now_ns')]
    if len(clocks) == 1:
        program += (f'uretprobe:{binary}:0x{clocks[0]} /pid == {args.qpid}/ {{ '
                    'printf("T %llu %d nsi_ns=%llu\\n", nsecs, tid, retval); }\n')
    notes = [line.split()[0] for line in symbols
             if '8nsitrace' in line and line.endswith('5Trace4note')]
    if len(notes) == 1:
        program += (f'uprobe:{binary}:0x{notes[0]} /pid == {args.qpid}/ {{ '
                    'printf("C %llu %d nsi_ns=%llu\\n", nsecs, tid, arg1); }\n')
    program = program.replace('nsecs', 'nsecs(monotonic)')
    (destination / 'trace.bt').write_text(program)
    outputs = [open(destination / 'trace.txt', 'w'), open(destination / 'trace.err', 'w')]
    tracer = subprocess.Popen(['bpftrace', str(destination / 'trace.bt')], stdout=outputs[0], stderr=outputs[1],
                              env={**os.environ, 'BPFTRACE_PERF_RB_PAGES': '1024'})
    observer = None
    observer_outputs = []
    try:
        time.sleep(3)
        if tracer.poll() is not None:
            raise RuntimeError('bpftrace failed to attach; inspect trace.err')
        if args.fence:
            descriptors = [p for p in Path(f'/proc/{args.qpid}/fd').iterdir()
                           if 'memory-backend-memfd' in os.readlink(p)]
            if len(descriptors) != 1:
                raise RuntimeError('guest RAM memfd is not unique')
            observer_outputs = [open(destination / 'fence.log', 'w')]
            observer = subprocess.Popen(
                [str(root / 'fencepoll'), str(args.qpid), descriptors[0].name,
                 hex(host_measure.fence_high(args.tag)), '0xC8', '20', str(destination / 'fence.txt')],
                stdout=observer_outputs[0], stderr=subprocess.STDOUT,
            )
        (destination / 'observers.json').write_text(json.dumps({
            'tracer_pid': tracer.pid, 'fence_pid': observer.pid if observer else None,
            'clock_monotonic_ns': time.monotonic_ns(), 'clock_epoch_ns': time.time_ns(),
        }, indent=2))
        host_measure.probe(root, args.tag, args.queue, args.iterations, 15000 if observer else 0)
        if observer:
            observer.wait(timeout=45)
            if observer.returncode:
                raise RuntimeError(f'fence observer failed rc={observer.returncode}')
    finally:
        if tracer.poll() is None:
            tracer.send_signal(signal.SIGINT)
            tracer.wait(timeout=15)
        if observer and observer.poll() is None:
            observer.terminate()
            observer.wait(timeout=10)
        for stream in outputs + observer_outputs:
            stream.close()
    error = (destination / 'trace.err').read_text()
    if re.search(r'lost|ERROR|failed', error, re.IGNORECASE):
        raise RuntimeError('BPF loss/error: trace cannot support a complete event census')
    if args.fence:
        analysis = iteration_hops.analyze(root / args.tag, destination, irqfd_identity)
        (destination / 'hops.json').write_text(json.dumps(analysis, indent=2))
        calibration = {'qpid': args.qpid,
                       'proc_start': Path(f'/proc/{args.qpid}/stat').read_text().split()[21],
                       'analysis': analysis}
        (root / (args.tag + '_clock.json')).write_text(json.dumps(calibration, indent=2))
    print('P1_TRACE_COMPLETE', destination, flush=True)


if __name__ == '__main__':
    main()
