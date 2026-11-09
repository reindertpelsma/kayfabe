#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Tests for scrub_prom.py. STATUS: LIVE, 2026-11-07.

The records are synthetic (no ROM content): a PROM read value is any non-zero number here.
"""

import contextlib
import importlib.util
import io
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
spec = importlib.util.spec_from_file_location("scrub_prom", HERE / "scrub_prom.py")
sp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sp)

HDRLEN = 160


def header(hdrlen=HDRLEN):
    h = struct.pack(sp.HDR_FMT, sp.MAGIC, 1, hdrlen, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
    return h + b"provenance text\0".ljust(hdrlen - len(h), b"\0")


def rec(seq, kind, bar, a, b, payload=b"", width=4):
    r = struct.pack(sp.ENT_FMT, seq, kind, width, bar, 0, len(payload), a, b) + payload
    return r + b"\0" * ((8 - (len(payload) & 7)) & 7)


def sample():
    parts = [
        rec(0, 1, 0, 0x300000, 0xAA55),  # PROM read: scrubbed
        rec(1, 1, 0, 0x300004, 0x52434950),  # PROM read: scrubbed
        rec(2, 1, 0, 0x3FFFFC, 0x1234),  # last PROM word: scrubbed
        rec(3, 1, 0, 0x2FFFFC, 0x77),  # just below the window: kept
        rec(4, 1, 0, 0x400000, 0x88),  # just above the window: kept
        rec(5, 1, 1, 0x300000, 0x99),  # BAR1 at the same offset: kept
        rec(6, 2, 0, 0x300000, 0xBB),  # a write to the window: kept
        rec(7, 3, 0, 0x1000, 0, payload=b"abcde"),  # GuestRead with padded payload: kept
        rec(8, 1, 0, 0x700000, 0xCC),  # PRAMIN: kept
        rec(9, 1, 0, 0x300008, 0),  # PROM read already zero: unchanged
    ]
    return header() + b"".join(parts), parts


class ScrubTests(unittest.TestCase):
    def test_only_prom_read_values_change(self):
        blob, _parts = sample()
        new, stats = sp.scrub_bytes(blob)
        self.assertEqual(len(new), len(blob))
        self.assertEqual(stats, {"records": 10, "prom_reads": 4, "nonzero_before": 3})
        diff = [i for i in range(len(blob)) if blob[i] != new[i]]
        # every differing byte lies in the 8-byte `b` field of records 0, 1, 2
        fields = [HDRLEN + 32 * k + 24 for k in range(3)]  # records 0..2 have no payload
        allowed = {f + j for f in fields for j in range(8)}
        self.assertTrue(diff and set(diff) <= allowed, diff)
        for f in fields:
            self.assertEqual(new[f : f + 8], b"\0" * 8)

    def test_idempotent_and_check(self):
        blob, _ = sample()
        nz, markers = sp.check_bytes(blob)
        self.assertEqual(nz, 3)
        once, _ = sp.scrub_bytes(blob)
        twice, stats = sp.scrub_bytes(once)
        self.assertEqual(once, twice)
        self.assertEqual(stats["nonzero_before"], 0)
        self.assertEqual(sp.check_bytes(once), (0, []))

    def test_check_flags_a_marker_outside_the_prom_window(self):
        blob, _ = sample()
        blob += rec(10, 4, 0, 0x2000, 0, payload=b"xxPCIRxx")  # guest write carrying a marker
        _, markers = sp.check_bytes(blob)
        self.assertEqual(markers, ["PCIR"])

    def test_truncated_tail_and_padding_are_preserved(self):
        blob, _ = sample()
        tail = rec(11, 1, 0, 0x300000, 0xFF)[:20]  # a partial record at the end
        new, _ = sp.scrub_bytes(blob + tail)
        self.assertEqual(new[-20:], tail)

    def test_prom_read_with_payload_is_refused(self):
        blob = header() + rec(0, 1, 0, 0x300000, 1, payload=b"12345678")
        with self.assertRaises(sp.ScrubError):
            sp.scrub_bytes(blob)

    def test_bad_magic_and_short_input(self):
        with self.assertRaises(sp.ScrubError):
            sp.scrub_bytes(b"\0" * 200)
        with self.assertRaises(sp.ScrubError):
            sp.scrub_bytes(b"\0" * 10)

    def test_cli_scrub_plain_and_zstd(self):
        blob, _ = sample()
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stdout(io.StringIO()):
            d = Path(d)
            (d / "in.rec").write_bytes(blob)
            self.assertEqual(sp.main(["check", str(d / "in.rec")]), 1)
            self.assertEqual(sp.main(["scrub", str(d / "in.rec"), str(d / "out.rec")]), None)
            self.assertEqual(sp.main(["check", str(d / "out.rec")]), 0)
            if shutil.which("zstd"):
                subprocess.run(["zstd", "-q", "-o", str(d / "in.rec.zst"), str(d / "in.rec")], check=True)
                sp.main(["scrub", str(d / "in.rec.zst"), str(d / "out.rec.zst")])
                self.assertEqual(sp.load(d / "out.rec.zst"), sp.load(d / "out.rec"))
                self.assertEqual(sp.main(["check", str(d / "out.rec.zst")]), 0)


if __name__ == "__main__":
    unittest.main()
