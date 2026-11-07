#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Scrub the option-ROM (PROM) read values out of an nvkvm replay trace (`.rec`, `.rec.zst`).

STATUS: LIVE, 2026-11-07. Owner decision (docs/OWNER_QUESTIONS_2026-10-03.md section 3, option
(b); inventory stage 6): replace every PROM read value with zero, keep every other record
byte-identical. Git history still holds the original files until the history update before release.

Record layout (the layout of `archive/nvkvm/scripts/mode2_diag/rec_dump.py`):
  header   96 bytes `<QIIIIQQQQQQ3Q` (magic, version, hdr_len, rec_size, rsv, props, mask,
           n_records, n_bytes, n_errors, t0_ns, rsv1[3]) followed by the provenance block up to hdr_len;
  record   32 bytes `<QBBBBIQQ` (seq, kind, width, bar, pad, payload_len, a, b), then `payload_len`
           payload bytes, then zero padding to a multiple of 8.
  kind 1 = MmioRead: `bar` and `a` = register offset, `b` = the value the device returned.

Scrubbed record: kind 1, bar 0, 0x300000 <= a <= 0x3fffff (BAR0 PROM window); only the 8-byte `b`
field is written (to 0). A PROM read that carries a payload is refused (the format has none).

    scrub_prom.py scrub IN OUT     .rec or .rec.zst (chosen by suffix; .zst is rewritten with
                                   `zstd -19 --long -T1`); prints counts only, never data
    scrub_prom.py check FILE...    exit 1 if any PROM read value is non-zero, or the decoded stream
                                   contains an option-ROM marker (`PCIR`, `NPDS`, `NPDE`)

No ROM content is ever printed. Standard library only (plus the `zstd` command before Python 3.14).
"""

import argparse
import struct
import subprocess
import sys
from pathlib import Path

MAGIC = 0x4352434552564B4E
HDR_FMT = "<QIIIIQQQQQQ3Q"
HDR_SIZE = struct.calcsize(HDR_FMT)
ENT_FMT = "<QBBBBIQQ"
ENT_SIZE = struct.calcsize(ENT_FMT)
MMIO_READ = 1
PROM_LO, PROM_HI = 0x300000, 0x3FFFFF
MARKERS = (b"PCIR", b"NPDS", b"NPDE")
assert HDR_SIZE == 96 and ENT_SIZE == 32


class ScrubError(Exception):
    pass


def records(blob):
    """Yield (offset, kind, bar, payload_len, a, b) for every complete record."""
    if len(blob) < HDR_SIZE:
        raise ScrubError("shorter than a header")
    magic, _ver, hdrlen = struct.unpack_from("<QII", blob, 0)
    if magic != MAGIC:
        raise ScrubError("bad magic")
    if hdrlen < HDR_SIZE or hdrlen > len(blob):
        raise ScrubError("bad header length")
    off = hdrlen
    while off + ENT_SIZE <= len(blob):
        _seq, kind, _width, bar, _pad, ln, a, b = struct.unpack_from(ENT_FMT, blob, off)
        end = off + ENT_SIZE + ln
        if end > len(blob):
            return  # truncated tail: left as it is
        yield off, kind, bar, ln, a, b
        off = end + ((8 - (ln & 7)) & 7)


def is_prom_read(kind, bar, a):
    return kind == MMIO_READ and bar == 0 and PROM_LO <= a <= PROM_HI


def scrub_bytes(blob):
    """Return (scrubbed bytes, stats). Only the `b` field of PROM reads changes."""
    out = bytearray(blob)
    stats = {"records": 0, "prom_reads": 0, "nonzero_before": 0}
    for off, kind, bar, ln, a, b in records(blob):
        stats["records"] += 1
        if not is_prom_read(kind, bar, a):
            continue
        if ln:
            raise ScrubError(f"PROM read at offset {off} carries a payload")
        stats["prom_reads"] += 1
        if b:
            stats["nonzero_before"] += 1
            struct.pack_into("<Q", out, off + 24, 0)
    return bytes(out), stats


def check_bytes(blob):
    """Return (nonzero PROM read values, option-ROM markers found anywhere in the stream)."""
    nonzero = sum(
        1 for _o, kind, bar, _ln, a, b in records(blob) if is_prom_read(kind, bar, a) and b
    )
    return nonzero, [m.decode() for m in MARKERS if m in blob]


def zstd_read(path):
    try:
        from compression import zstd  # Python 3.14+

        return zstd.decompress(Path(path).read_bytes())
    except ImportError:
        return subprocess.run(
            ["zstd", "-dc", "--long=31", str(path)], check=True, capture_output=True
        ).stdout


def zstd_write(path, data):
    subprocess.run(
        ["zstd", "-q", "-f", "-19", "--long", "-T1", "-o", str(path)],
        input=data,
        check=True,
    )


def load(path):
    return zstd_read(path) if str(path).endswith(".zst") else Path(path).read_bytes()


def cmd_scrub(args):
    blob = load(args.input)
    new, stats = scrub_bytes(blob)
    if len(new) != len(blob):
        raise ScrubError("length changed")
    if str(args.output).endswith(".zst"):
        zstd_write(args.output, new)
    else:
        Path(args.output).write_bytes(new)
    nz, markers = check_bytes(new)
    if nz or markers:
        raise ScrubError(f"scrubbed output still holds {nz} non-zero PROM values, markers {markers}")
    print(
        f"{args.input}: records={stats['records']} prom_reads={stats['prom_reads']} "
        f"nonzero_before={stats['nonzero_before']} nonzero_after=0 markers_after=none"
    )


def cmd_check(args):
    bad = 0
    for path in args.files:
        nz, markers = check_bytes(load(path))
        status = "clean" if not nz and not markers else "HOLDS PROM DATA"
        bad += status != "clean"
        print(f"{path}: nonzero_prom_reads={nz} markers={','.join(markers) or 'none'} {status}")
    return 1 if bad else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("scrub")
    s.add_argument("input")
    s.add_argument("output")
    c = sub.add_parser("check")
    c.add_argument("files", nargs="+")
    args = ap.parse_args(argv)
    try:
        return cmd_scrub(args) if args.cmd == "scrub" else cmd_check(args)
    except ScrubError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
