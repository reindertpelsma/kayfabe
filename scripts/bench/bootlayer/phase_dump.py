#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Capture store [0,G) through an explicitly supplied existing diagnostic adapter.

Adapter argv: READER OFFSET LENGTH RAW_OUTPUT. Its stdout is a JSON object with
space="store", offset, length, and a nonempty source identifying the diagnostic.
A BAR1 dump is not a store dump while BAR1 is virtual or scratch.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

G = 0x7F0000
PHASES = ("realized", "fn72", "armed", "unloaded", "reboot_scrubbed")


def dump(out, phase, reader, revision, timeout=20):
    stamp = {"phase": phase, "revision": revision, "start_ns": time.time_ns(),
             "space": "store", "offset": 0, "length": G}
    raw = out / (phase + ".store.bin")
    meta = out / (phase + ".json")
    if meta.exists():
        raise RuntimeError("phase already captured: " + phase)
    try:
        if not reader:
            raise RuntimeError("UNAVAILABLE: no existing store diagnostic adapter supplied")
        command = [str(Path(reader).resolve()), "0", str(G), str(raw.resolve())]
        run = subprocess.run(command, capture_output=True, timeout=timeout, check=True)
        identity = json.loads(run.stdout)
        if (identity.get("space"), identity.get("offset"), identity.get("length")) != ("store", 0, G):
            raise RuntimeError("REFUSED: diagnostic did not identify store [0,G)")
        if not identity.get("source"):
            raise RuntimeError("REFUSED: diagnostic source missing")
        if raw.stat().st_size != G:
            raise RuntimeError("REFUSED: short or oversized store read")
        data = raw.read_bytes()
        stamp.update(status="CAPTURED", diagnostic=identity,
                     reader_sha256=hashlib.sha256(Path(reader).read_bytes()).hexdigest(),
                     sha256=hashlib.sha256(data).hexdigest(),
                     nonzero=sum(b != 0 for b in data))
        # Full coverage, with identical adjacent rows collapsed as in hexdump -C.
        with (out / (phase + ".store.hex")).open("x") as f:
            previous = None
            collapsed = False
            for off in range(0, len(data), 16):
                row = data[off:off + 16]
                if row == previous:
                    if not collapsed:
                        f.write("*\n")
                    collapsed = True
                    continue
                f.write(f"{off:08x}  {row.hex(' ')}\n")
                previous, collapsed = row, False
            f.write(f"{len(data):08x}\n")
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        stamp.update(status="UNAVAILABLE", error=str(error))
    stamp["end_ns"] = time.time_ns()
    meta.write_text(json.dumps(stamp, indent=2) + "\n")
    return stamp


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("out", type=Path)
    p.add_argument("phase", choices=PHASES)
    p.add_argument("--reader")
    p.add_argument("--revision", required=True)
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    result = dump(a.out, a.phase, a.reader, a.revision)
    print(json.dumps(result))
    return 0 if result["status"] == "CAPTURED" else 78


if __name__ == "__main__":
    raise SystemExit(main())
