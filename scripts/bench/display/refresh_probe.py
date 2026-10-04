#!/usr/bin/env python3
"""Probe real HMP/QMP screendump completion, with a bounded wait and pixel census.

Run against a booted display guest. A timeout is a failure, not an empty picture.
The caller supplies a unique output path on the box; only the JSON report needs
to be retained as evidence. This exercises QEMU's actual coroutine dispatcher.
"""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import socket
import time


def hmp(sock, command):
    def prompt():
        data = bytearray()
        while b"(qemu) " not in data:
            chunk = sock.recv(65536)
            if not chunk:
                raise EOFError("HMP closed before its prompt")
            data.extend(chunk)
        return bytes(data)

    prompt()
    sock.sendall(command.encode() + b"\n")
    return prompt()


def qmp(sock, filename, device):
    stream = sock.makefile("rwb", buffering=0)

    def reply():
        while True:
            line = stream.readline()
            if not line:
                raise EOFError("QMP closed before its reply")
            packet = json.loads(line)
            if "event" not in packet:
                return packet

    reply()
    stream.write(b'{"execute":"qmp_capabilities"}\n')
    if "return" not in reply():
        raise RuntimeError("QMP capabilities refused")
    stream.write(json.dumps({
        "execute": "screendump",
        "arguments": {"filename": filename, "device": device},
    }).encode() + b"\n")
    packet = reply()
    if "return" not in packet:
        raise RuntimeError(str(packet))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("interface", choices=["hmp", "qmp"])
    parser.add_argument("socket")
    parser.add_argument("output")
    parser.add_argument("--device", default="kf0")
    parser.add_argument("--timeout", type=float, default=8)
    args = parser.parse_args()
    out = Path(args.output)
    if out.exists():
        parser.error("output already exists; refusing a stale screenshot")
    started = time.monotonic()
    result = {"interface": args.interface, "socket": args.socket}
    try:
        with socket.socket(socket.AF_UNIX) as sock:
            sock.settimeout(args.timeout)
            sock.connect(args.socket)
            if args.interface == "hmp":
                # QEMU's HMP string syntax accepts JSON's quoted plain paths.
                hmp(sock, "screendump %s %s" % (
                    json.dumps(args.output), json.dumps(args.device)))
            else:
                qmp(sock, args.output, args.device)
        data = out.read_bytes()
        magic, size, maximum, pixels = data.split(b"\n", 3)
        width, height = map(int, size.split())
        if magic != b"P6" or maximum != b"255" or len(pixels) != width * height * 3:
            raise ValueError("not a complete RGB PPM")
        counts = Counter(pixels[i:i + 3].hex() for i in range(0, len(pixels), 3))
        result.update(ok=True, width=width, height=height,
                      sha256=hashlib.sha256(data).hexdigest(),
                      dominant=counts.most_common(3))
    except (OSError, ValueError, EOFError, RuntimeError) as error:
        result.update(ok=False, error=str(error) or type(error).__name__)
    result["elapsed_ms"] = round((time.monotonic() - started) * 1000)
    print(json.dumps(result), flush=True)
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
