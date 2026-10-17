#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Record an owned interactive QEMU; shut it down with HMP powerdown then TERM."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import time


def identity(pid):
    try:
        proc = Path('/proc') / str(pid)
        stat = (proc / 'stat').read_text().rsplit(') ', 1)[1].split()
        if stat[0] == 'Z':
            return None
        command = (proc / 'cmdline').read_bytes()
        return {'pid': pid, 'exe': os.readlink(proc / 'exe'),
            'start_ticks': int(stat[19]),
            'command_sha256': hashlib.sha256(command).hexdigest()}, command
    except (FileNotFoundError, ProcessLookupError):
        return None


def record(pid, executable, monitor, receipt):
    executable = str(Path(executable).resolve())
    deadline = time.monotonic() + 2
    while True:
        found = identity(pid)
        if found is None:
            raise ValueError('owned QEMU exited before its identity was recorded')
        data, command = found
        if data['exe'] == executable:
            break
        if time.monotonic() >= deadline:
            raise ValueError('owned executable differs')
        time.sleep(.02)
    args = command.rstrip(b'\0').split(b'\0')
    for option, value in [(b'-name', b'kayfabe-interactive'),
            (b'-monitor', f'unix:{monitor},server,nowait'.encode())]:
        if not any(a == option and b == value for a, b in zip(args, args[1:])):
            raise ValueError('owned QEMU name or monitor differs')
    data['monitor'] = monitor
    with open(receipt, 'x') as out:
        os.chmod(receipt, 0o600)
        json.dump(data, out)
    print('OWNED_QEMU_RECORDED pid=' + str(pid), flush=True)


def still_owned(data):
    found = identity(data['pid'])
    if found is None:
        return False
    observed, _ = found
    if any(observed[key] != data[key] for key in observed):
        raise ValueError('process identity changed; refusing to signal it')
    return True


def wait_for_exit(data, seconds):
    deadline = time.monotonic() + seconds
    while still_owned(data):
        left = deadline - time.monotonic()
        if left <= 0:
            return False
        time.sleep(min(.05, left))
    return True


def stop_process(data, acpi_seconds, term_seconds):
    if not still_owned(data):
        print('OWNED_QEMU_STOP already-exited', flush=True)
        return 0
    try:
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(2)
            connection.connect(data['monitor'])
            connection.sendall(b'system_powerdown\n')
    except OSError:
        pass
    if wait_for_exit(data, acpi_seconds):
        print('OWNED_QEMU_STOP how=acpi', flush=True)
        return 0
    # Recheck PID start time, executable and entire argv immediately before TERM.
    try:
        descriptor = os.pidfd_open(data['pid'])
    except ProcessLookupError:
        return 0
    try:
        if still_owned(data):
            signal.pidfd_send_signal(descriptor, signal.SIGTERM)
    except ProcessLookupError:
        return 0
    finally:
        os.close(descriptor)
    if wait_for_exit(data, term_seconds):
        print('OWNED_QEMU_STOP how=term', flush=True)
        return 0
    print('OWNED_QEMU_STOP incomplete; process and receipt retained', flush=True)
    return 1


def stop(receipt, acpi_seconds, term_seconds):
    with open(receipt) as owned:
        # A concurrent stop cannot send another TERM during cleanup.
        fcntl.flock(owned, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return stop_process(json.load(owned), acpi_seconds, term_seconds)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    capture = commands.add_parser('record')
    capture.add_argument('pid', type=int)
    capture.add_argument('executable')
    capture.add_argument('monitor')
    capture.add_argument('receipt')
    finish = commands.add_parser('stop')
    finish.add_argument('receipt')
    finish.add_argument('--acpi-seconds', type=float, default=60)
    finish.add_argument('--term-seconds', type=float, default=15)
    args = parser.parse_args()
    try:
        if args.command == 'record':
            record(args.pid, args.executable, args.monitor, args.receipt)
            return 0
        if not (0 <= args.acpi_seconds <= 120 and 0 <= args.term_seconds <= 120):
            raise ValueError('shutdown waits must be bounded to0..120seconds')
        return stop(args.receipt, args.acpi_seconds, args.term_seconds)
    except (OSError, ValueError, KeyError) as error:
        print('OWNED_QEMU_REFUSED ' + str(error), flush=True)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
