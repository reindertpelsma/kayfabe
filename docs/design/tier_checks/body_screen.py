#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Structural screen of RM-control bodies in observer captures, for the scrub allow-list review.

STATUS: RESEARCH, 2026-10-13. Reads observer `gsp.jsonl` files (host-only, never copied) and prints, per control
id and direction, ONLY counts and booleans - never a body byte. It answers one question about each control: does
any observed request or reply body LOOK like it could carry an identifier (a UUID/serial/ROM-like blob)?

usage: python3 -I body_screen.py GSP_RM_CONTROL_FUNCTION_ID < gsp.jsonl   (or several files as arguments)
       python3 -I body_screen.py 76 file1.jsonl file2.jsonl > screen.tsv

Criteria (a screen, not a proof: an identifier that happens to look structured passes it, and a table of
hashes or pointers fails it):
  ascii  a run of >= 6 printable ASCII bytes with >= 4 alphanumerics        (serial / name / version string)
  hent   a 16-byte window with >= 14 distinct byte values and >= 12 non-zero (random-looking: UUID, hash, key)
  rom    the 0x55 0xAA ROM signature at a 512-byte boundary                  (VBIOS / expansion ROM)
Bodies are de-duplicated by content before counting (a body repeated 600 times counts once). Large controls
(a head plus continuation records) are counted as `large` and not screened (their bodies are tables that the
scrub treats the same way: only the head is looked at).
"""
import collections
import hashlib
import json
import re
import sys

ASCII = re.compile(rb"[\x20-\x7e]{6,}")


def flags(b):
    f = set()
    for m in ASCII.finditer(b):
        if sum(chr(c).isalnum() for c in m.group()) >= 4:
            f.add("ascii")
            break
    for i in range(0, max(0, len(b) - 15)):
        w = b[i : i + 16]
        if len(set(w)) >= 14 and sum(1 for x in w if x) >= 12:
            f.add("hent")
            break
    for i in range(0, len(b) - 1, 512):
        if b[i : i + 2] == b"\x55\xaa":
            f.add("rom")
            break
    return f


def main():
    ctl_fn = int(sys.argv[1])
    paths = sys.argv[2:]
    seen = collections.defaultdict(set)      # (cmd, dir) -> content hashes
    sizes = collections.defaultdict(set)
    flagged = collections.defaultdict(lambda: collections.Counter())
    large = collections.Counter()
    streams = [open(p, "rb") for p in paths] if paths else [sys.stdin.buffer]
    for s in streams:
        for line in s:
            if not line.startswith(b'{"kind":"record"'):
                continue
            r = json.loads(line)
            b = bytes.fromhex(r["payload_hex"])
            if len(b) < 116 or int.from_bytes(b[60:64], "little") != ctl_fn:
                continue
            rpc_len = int.from_bytes(b[56:60], "little")
            cmd = int.from_bytes(b[88:92], "little")
            psize = int.from_bytes(b[96:100], "little")
            body = b[116 : 48 + rpc_len]
            d = r["direction"]
            if len(body) < psize:
                large[(cmd, d)] += 1
                continue
            key = (cmd, d)
            h = hashlib.sha256(body).digest()
            if h in seen[key]:
                continue
            seen[key].add(h)
            sizes[key].add(len(body))
            for f in flags(body):
                flagged[key][f] += 1
    print("cmd\tdir\tdistinct_bodies\tsizes\tascii\thent\trom\tlarge_heads_not_screened")
    for key in sorted(set(seen) | set(large)):
        cmd, d = key
        fl = flagged[key]
        print(
            f"{cmd:#010x}\t{'req' if d == 0 else 'rep'}\t{len(seen[key])}\t"
            f"{','.join(str(x) for x in sorted(sizes[key])[:6])}{'...' if len(sizes[key]) > 6 else ''}\t"
            f"{fl['ascii']}\t{fl['hent']}\t{fl['rom']}\t{large[key]}"
        )


if __name__ == "__main__":
    main()
