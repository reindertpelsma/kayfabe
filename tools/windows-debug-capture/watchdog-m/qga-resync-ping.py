#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""One bounded, serialized QGA framing reset + guest-ping; no guest execution.

Stop other QGA clients first: older clients do not honor the per-socket lock.
The rpc function is copied unchanged from vast-windows prepare/prepare.py.
"""
import argparse
import fcntl
import json
import os
import secrets
import socket
import time

def rpc(path, name, args=None, qmp=False, timeout=10):
    # QGA is one shared serial stream. QMP also allows only one client on our
    # socket backend. Serialize across processes, including evidence collectors.
    # Never unlink this lock file: another client may already have it open.
    deadline = time.monotonic() + timeout
    def remaining():
        value = deadline - time.monotonic()
        if value <= 0:
            raise TimeoutError(f'{name}: QEMU RPC exceeded {timeout}s')
        return value
    lock_fd = os.open(str(path) + '.lock', os.O_RDWR | os.O_CREAT, 0o600)
    with os.fdopen(lock_fd, 'a') as lock:
        while True:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                time.sleep(min(0.05, remaining()))
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(remaining())
            sock.connect(str(path))
            pending = bytearray()
            def receive_line():
                remaining()
                while b'\n' not in pending:
                    sock.settimeout(remaining())
                    chunk = sock.recv(65536)
                    if not chunk:
                        raise EOFError('QEMU channel closed')
                    pending.extend(chunk)
                    if len(pending) > 8 << 20:
                        raise ValueError('QEMU response exceeds 8 MiB')
                line, _, rest = pending.partition(b'\n')
                pending[:] = rest
                return line
            def write(command, arguments=None, prefix=b''):
                request = {'execute': command}
                if arguments is not None:
                    request['arguments'] = arguments
                sock.settimeout(remaining())
                sock.sendall(prefix + json.dumps(request).encode() + b'\n')
            def send(command, arguments=None):
                write(command, arguments)
                while True:
                    response = json.loads(receive_line())
                    if 'error' in response:
                        raise RuntimeError(f'{command}: {response["error"]}')
                    if 'return' in response:
                        return response['return']
            if qmp:
                if 'QMP' not in json.loads(receive_line()):
                    raise RuntimeError('invalid QMP greeting')
                send('qmp_capabilities')
            else:
                token = secrets.randbits(63)
                # Official QGA framing resets both parsers after a timeout or a
                # disconnected client. Ignore stale errors, replies and partial
                # JSON until our own delimited sync response arrives.
                write('guest-sync-delimited', {'id': token}, prefix=b'\xff')
                while True:
                    line = receive_line()
                    if b'\xff' not in line:
                        continue
                    try:
                        response = json.loads(line.rsplit(b'\xff', 1)[1])
                    except ValueError:
                        continue
                    if isinstance(response, dict) and response.get('return') == token:
                        break
            return send(name, args)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('socket')
    parser.add_argument('--timeout', type=int, choices=range(1, 16), default=8)
    args = parser.parse_args()
    print(json.dumps({'guest_ping': rpc(args.socket, 'guest-ping', timeout=args.timeout)}))
