#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""RM-call census of a kf3 QEMU log window (stdlib only).

STATUS: LIVE, 2026-11-16 (GPU-free tests: scripts/ci/test_kf3_census.py; the exact line formats of a hardware run are read from
crates/kf-qemu/src/device.rs and crates/kf-chan; widen KEYWORDS when a run shows a new one).

    census(lines) -> {"lines": N, "counts": {keyword: n}, "ids": {keyword: [distinct ids, at most 40]}, "xid": n}

For one application row: what kf3 saw and did NOT serve (UNSERVICED RPC functions, ForeignClass refusals, refused ranges, DEAD channels,
walk problems) plus Xid/TDR lines. Identifiers are the hex numbers on the matching lines; UUIDs, MACs and serial-like strings are
stripped first so the census can go into public evidence. An unserviced control that the application survives is a FINDING, not a failure.
"""
import re
from collections import OrderedDict

KEYWORDS = OrderedDict([
    ("unserviced", re.compile(r"UNSERVICED", re.I)),
    ("foreign_class", re.compile(r"ForeignClass|foreign_class", re.I)),
    ("refused", re.compile(r"\brefused\b|\bREFUSED\b")),
    ("dead_channel", re.compile(r"\bDEAD\b")),
    ("walk", re.compile(r"\bwalk\b.*\b(fail|miss|error|timeout)", re.I)),
    ("derived_miss", re.compile(r"derived.*(miss|absent|unknown)", re.I)),
    ("xid", re.compile(r"NVRM: Xid|\bXid\b")),
    ("tdr", re.compile(r"\bTDR\b|Timeout Detection|video_tdr", re.I)),
])
UUID = re.compile(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b|GPU-[0-9a-f-]{20,}")
MAC = re.compile(r"\b(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}\b")
HEXID = re.compile(r"\b0x[0-9a-fA-F]{1,16}\b")
MAX_IDS = 40


def clean(line):
    return MAC.sub("<mac>", UUID.sub("<uuid>", line))


def census(lines):
    counts = OrderedDict((k, 0) for k in KEYWORDS)
    ids = OrderedDict((k, OrderedDict()) for k in KEYWORDS)
    n = 0
    for raw in lines:
        n += 1
        line = clean(raw)
        for k, rx in KEYWORDS.items():
            if rx.search(line):
                counts[k] += 1
                for h in HEXID.findall(line):
                    if len(ids[k]) < MAX_IDS:
                        ids[k][h.lower()] = True
    return {"lines": n, "counts": {k: v for k, v in counts.items() if v}, "ids": {k: list(v) for k, v in ids.items() if v},
            "xid": counts["xid"]}


def window(path, lo, hi):
    """Lines lo..hi (1-based, inclusive, hi None = end) of a log file, decoded leniently."""
    out = []
    try:
        with open(path, "rb") as f:
            for i, raw in enumerate(f, 1):
                if i < lo:
                    continue
                if hi is not None and i > hi:
                    break
                out.append(raw.decode("utf-8", "replace").rstrip("\n"))
    except OSError:
        pass
    return out


def count_lines(path):
    try:
        with open(path, "rb") as f:
            return sum(1 for _ in f)
    except OSError:
        return 0
