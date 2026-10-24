#!/usr/bin/env python3
"""Bounded concurrent SSH chunks; publish only after full size/SHA validation."""
from concurrent.futures import ThreadPoolExecutor, as_completed
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import time


def plan(size, workers):
    if type(size) is not int or size <= 0 or type(workers) is not int or not 1 <= workers <= 16:
        raise ValueError('positive size and 1..16 workers required')
    n = min(size, workers)
    quotient, remainder = divmod(size, n)
    out = []; offset = 0
    for i in range(n):
        count = quotient + (i < remainder)
        out.append((offset, count)); offset += count
    return out


def stream(source, sink, offset, count):
    source.seek(offset)
    while count:
        block = source.read(min(1024 * 1024, count))
        if not block:
            raise ValueError('source truncated during transfer')
        if sink.write(block) != len(block):
            raise ValueError('short transport write')
        count -= len(block)


def dispatch(ranges, send, publish):
    with ThreadPoolExecutor(max_workers=len(ranges)) as pool:
        jobs = [pool.submit(send, index, offset, count) for index, (offset, count) in enumerate(ranges)]
        for job in as_completed(jobs):
            job.result()
    return publish()


def publish_command(destination, parts, size, digest):
    if size <= 0 or not re.fullmatch('[0-9a-f]{64}', digest) or not parts:
        raise ValueError('invalid publication contract')
    quote = shlex.quote
    temporary = destination + '.assembling'
    inputs = ' '.join(quote(p) for p in parts)
    return (f'set -eu; cat -- {inputs} > {quote(temporary)}; '
            f'test "$(stat -c %s -- {quote(temporary)})" = {size}; '
            f'test "$(sha256sum -- {quote(temporary)} | cut -d\' \' -f1)" = {quote(digest)}; '
            f'mv -f -- {quote(temporary)} {quote(destination)}; rm -f -- {inputs}; echo WHOLE_FILE_PIN_OK')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'ssh_config', 'destination', 'size', 'sha256'):
        parser.add_argument(name)
    parser.add_argument('--workers', type=int, default=8)
    args = parser.parse_args()
    source = Path(args.source); size = int(args.size)
    ranges = plan(size, args.workers)
    if source.stat().st_size != size:
        raise ValueError('source size mismatch')
    sha = hashlib.sha256()
    with source.open('rb') as stream_in:
        for block in iter(lambda: stream_in.read(1024 * 1024), b''):
            sha.update(block)
    if sha.hexdigest() != args.sha256:
        raise ValueError('source SHA mismatch')
    base = args.destination + '.s4chunks'
    ssh = ['ssh', '-F', args.ssh_config, 's4-own']
    subprocess.run([*ssh, 'mkdir -p -- ' + shlex.quote(base)], check=True, timeout=60)
    parts = [base + '/' + str(i) for i in range(len(ranges))]
    started = time.monotonic()

    def send(index, offset, count):
        cmd = 'umask 077; cat > ' + shlex.quote(parts[index])
        child = subprocess.Popen(['timeout', '-k', '60', '1200', *ssh, cmd], stdin=subprocess.PIPE,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            with source.open('rb') as stream_in:
                stream(stream_in, child.stdin, offset, count)
            child.stdin.close()
            if child.wait() != 0:
                raise RuntimeError('chunk transfer failed: ' + str(index))
        finally:
            if child.poll() is None:
                child.terminate(); child.wait(timeout=30)
        print(json.dumps({'chunk': index, 'offset': offset, 'bytes': count,
                          'elapsed_s': round(time.monotonic() - started, 3)}), flush=True)

    def publish():
        cmd = publish_command(args.destination, parts, size, args.sha256)
        result = subprocess.run([*ssh, cmd], capture_output=True, text=True, check=True, timeout=180)
        if result.stdout.strip() != 'WHOLE_FILE_PIN_OK':
            raise ValueError('receiver did not acknowledge final hash')
        print(json.dumps({'kind': 'MEASURED', 'bytes': size, 'sha256': args.sha256,
                          'workers': len(ranges), 'elapsed_s': round(time.monotonic() - started, 3),
                          'receipt': result.stdout.strip()}), flush=True)
    dispatch(ranges, send, publish)


if __name__ == '__main__':
    main()
