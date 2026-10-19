#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Steel Nomad ETW plus an owned external new-content broker observer.

Requires a fence-value clock calibration from THIS QEMU process, so guest ETW
and compositor feedback use the same graphics-test window. No global process kill.
"""
import argparse
from datetime import datetime
import json
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

import broker_content
import host_measure


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--broker', type=Path, required=True)
    parser.add_argument('--broker-pid', type=int, required=True)
    parser.add_argument('--qpid', type=int, required=True)
    parser.add_argument('--clock-map', type=Path, required=True)
    parser.add_argument('--offset', type=int, required=True)
    parser.add_argument('--seconds', type=int, default=105)
    parser.add_argument('tag')
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    if root != Path('/data/codex-P1') or not (root / 'GPU_LEASE_HELD').is_file():
        parser.error('P1 lease required')
    host_measure.require_lease(root)
    if not re.fullmatch(r'[a-zA-Z0-9_-]+', args.tag):
        parser.error('unsafe tag')
    binary = args.broker.resolve(strict=True)
    if Path(f'/proc/{args.broker_pid}/exe').resolve(strict=True) != binary:
        parser.error('broker PID/executable mismatch')
    clock = json.loads(args.clock_map.read_text())
    if clock['qpid'] != args.qpid or clock['proc_start'] != Path(f'/proc/{args.qpid}/stat').read_text().split()[21]:
        parser.error('clock calibration belongs to another QEMU process')
    destination = root / (args.tag + '_broker')
    destination.mkdir(exist_ok=False)
    program = subprocess.check_output([sys.executable, str(host_measure.HERE / 'broker_content.py'),
                                       'gen', str(binary), str(args.broker_pid), str(args.offset)], text=True)
    (destination / 'trace.bt').write_text(program)
    streams = [open(destination / 'trace.txt', 'w'), open(destination / 'trace.err', 'w')]
    observer = subprocess.Popen(['bpftrace', str(destination / 'trace.bt')], stdout=streams[0], stderr=streams[1])
    try:
        time.sleep(2)
        if observer.poll() is not None:
            raise RuntimeError('broker observer did not attach')
        host_measure.capture(root, args.tag, args.seconds)
    finally:
        if observer.poll() is None:
            observer.send_signal(signal.SIGINT)
            observer.wait(timeout=15)
        for stream in streams:
            stream.close()
    if re.search(r'lost|ERROR|failed', (destination / 'trace.err').read_text(), re.IGNORECASE):
        raise RuntimeError('broker BPF loss/error; no complete content census')
    start_text = (root / args.tag / 'start.txt').read_text()
    capture = re.search(r'CAPSTART (\S+) qpc=(\d+) freq=(\d+)', start_text)
    if not capture:
        raise RuntimeError('missing guest UTC/QPC anchor')
    guest_epoch_ns = datetime.fromisoformat(capture[1]).timestamp() * 1e9
    qpc_ns = int(capture[2]) * 1e9 / int(capture[3])
    offset = clock['analysis']['host_minus_guest_qpc_ns']['median']
    graphics = json.loads((root / args.tag / (args.tag + '_metrics.json')).read_text())
    window = [int(offset + qpc_ns + (ticks * 100 - 11644473600000000000 - guest_epoch_ns))
              for ticks in graphics['window_filetime_100ns']]
    summary = broker_content.summarize(destination / 'trace.txt', *window)
    summary['window_monotonic_ns'] = window
    summary['clock_calibration'] = str(args.clock_map)
    summary['clock_limit'] = 'fence observer precedes guest poll; calibration spread recorded; UTC/QPC anchor sampled consecutively'
    (root / args.tag / (args.tag + '_broker.json')).write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == '__main__':
    main()
