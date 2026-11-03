#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded outbound probe staging; verify the resulting hash in Windows before execution."""
import base64
import hashlib
import json
from pathlib import Path
import secrets
import socket
import sys
import time

sock_path, source, destination = sys.argv[1:]
p = Path(source)
if not 0 < p.stat().st_size <= 4 * 1024 * 1024:
    raise SystemExit('REFUSED source size')
data = p.read_bytes()
if len(data) > 4 * 1024 * 1024 or len(destination) > 1024:
    raise SystemExit('REFUSED size changed or destination too long')
deadline = time.monotonic() + 120

def synced_connection():
    for attempt in range(4):
        session = socket.socket(socket.AF_UNIX)
        reader = None
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError('QGA synchronization deadline exceeded')
            session.settimeout(min(15, remaining))
            session.connect(sock_path)
            reader = session.makefile('rwb', buffering=0)
            token = secrets.randbelow(1 << 31)
            session.sendall(b'\xff' + json.dumps({'execute':'guest-sync','arguments':{'id':token}}).encode() + b'\n')
            for _ in range(64):
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError('QGA synchronization deadline exceeded')
                session.settimeout(min(15, remaining))
                raw = reader.readline(32769)
                if not raw:
                    raise EOFError('agent closed during synchronization')
                if not raw.endswith(b'\n') or len(raw) > 32768:
                    raise RuntimeError('REFUSED oversized/incomplete QGA sync reply')
                try:
                    reply = json.loads(raw)
                except ValueError:
                    continue
                if isinstance(reply, dict) and reply.get('return') == token:
                    return session, reader
            raise RuntimeError('QGA sync did not echo the token within64replies')
        except (OSError, EOFError):
            if reader is not None:
                reader.close()
            session.close()
            if attempt == 3 or time.monotonic() >= deadline:
                raise
            print(f'QGA_SYNC_RECONNECT attempt={attempt+1}; no file command was sent')
        except BaseException:
            if reader is not None:
                reader.close()
            session.close()
            raise
    raise RuntimeError('QGA synchronization attempts exhausted')

s, f = synced_connection()

def call(command, args):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError('QGA file staging deadline exceeded')
    s.settimeout(min(15, remaining))
    s.sendall(json.dumps({'execute': command, 'arguments': args}).encode() + b'\n')
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError('QGA file staging deadline exceeded')
    s.settimeout(min(15, remaining))
    raw = f.readline(32769)
    if not raw.endswith(b'\n') or len(raw) > 32768:
        raise RuntimeError('REFUSED oversized/incomplete QGA reply')
    reply = json.loads(raw)
    if not isinstance(reply, dict) or 'error' in reply or 'return' not in reply:
        raise RuntimeError('QGA request failed: ' + command)
    return reply['return']

handle = None
try:
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
    closing, handle = handle, None
    # A lost close response is uncertain too; never close that handle a second time.
    call('guest-file-close', {'handle': closing})
    print(f'QGA_PUT bytes={len(data)} sha256={hashlib.sha256(data).hexdigest()}')
finally:
    if handle is not None:
        try:
            call('guest-file-close', {'handle': handle})
        except (OSError, ValueError, RuntimeError):
            pass
    f.close()
    s.close()
