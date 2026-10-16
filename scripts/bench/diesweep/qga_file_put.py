#!/usr/bin/env python3
"""Bounded outbound probe staging; verify the resulting hash in Windows before execution."""
import base64
import hashlib
import json
from pathlib import Path
import secrets
import socket
import sys

sock_path, source, destination = sys.argv[1:]
p = Path(source)
if not 0 < p.stat().st_size <= 4 * 1024 * 1024:
    raise SystemExit('REFUSED source size')
data = p.read_bytes()
if len(data) > 4 * 1024 * 1024 or len(destination) > 1024:
    raise SystemExit('REFUSED size changed or destination too long')
s = socket.socket(socket.AF_UNIX)
s.settimeout(15)
s.connect(sock_path)
f = s.makefile('rwb', buffering=0)

def call(command, args):
    f.write(json.dumps({'execute': command, 'arguments': args}).encode() + b'\n')
    raw = f.readline(32769)
    if not raw.endswith(b'\n') or len(raw) > 32768:
        raise RuntimeError('REFUSED oversized/incomplete QGA reply')
    reply = json.loads(raw)
    if not isinstance(reply, dict) or 'error' in reply or 'return' not in reply:
        raise RuntimeError('QGA request failed: ' + command)
    return reply['return']

handle = None
try:
    token = secrets.randbelow(1 << 31)
    if call('guest-sync', {'id': token}) != token:
        raise RuntimeError('QGA sync failed')
    handle = call('guest-file-open', {'path': destination, 'mode': 'wb'})
    if type(handle) is not int or not 0 <= handle < (1 << 63):
        handle = None
        raise RuntimeError('REFUSED invalid QGA handle')
    for offset in range(0, len(data), 32768):
        chunk = data[offset:offset + 32768]
        reply = call('guest-file-write', {'handle': handle, 'buf-b64': base64.b64encode(chunk).decode()})
        if not isinstance(reply, dict) or type(reply.get('count')) is not int or reply['count'] != len(chunk):
            raise RuntimeError('REFUSED short/invalid QGA write')
    call('guest-file-flush', {'handle': handle})
    call('guest-file-close', {'handle': handle})
    handle = None
    print(f'QGA_PUT bytes={len(data)} sha256={hashlib.sha256(data).hexdigest()}')
finally:
    if handle is not None:
        try:
            call('guest-file-close', {'handle': handle})
        except (OSError, ValueError, RuntimeError):
            pass
    s.close()
