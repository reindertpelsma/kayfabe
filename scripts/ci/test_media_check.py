#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/bench/windows/media_check.py (2026-11-16): the readers on files made here (PNG via zlib, a minimal MP4
box tree, a PSD header, a PDF) and on hostile input (truncated, wrong type)."""
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench/windows"))
import media_check as mc  # noqa: E402


def chunk(t, b):
    return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b) & 0xFFFFFFFF)


def make_png(w, h, fn, filt=0):
    rows = b"".join(bytes([filt]) + bytes(fn(x, y) for x in range(w) for _ in range(3)) for y in range(h))
    return mc.PNG_SIG + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def box(t, b):
    return struct.pack(">I4s", 8 + len(b), t) + b


class T(unittest.TestCase):
    def setUp(self):
        self.td = tempfile.TemporaryDirectory()
        self.d = Path(self.td.name)

    def tearDown(self):
        self.td.cleanup()

    def test_png_stats(self):
        p = self.d / "a.png"
        p.write_bytes(make_png(40, 30, lambda x, y: (x * 6) & 255))
        i = mc.png_info(p)
        self.assertEqual((i["width"], i["height"]), (40, 30))
        self.assertGreater(i["std"], 20)
        flat = self.d / "f.png"
        flat.write_bytes(make_png(40, 30, lambda x, y: 7))
        self.assertEqual(mc.png_info(flat)["std"], 0.0)

    def test_png_sub_filter(self):
        p = self.d / "s.png"
        p.write_bytes(make_png(8, 4, lambda x, y: 1, filt=1))     # Sub filter: decoded values accumulate
        self.assertGreater(mc.png_info(p)["mean"], 1)

    def test_not_png(self):
        (self.d / "x.png").write_bytes(b"hello")
        with self.assertRaises(ValueError):
            mc.png_info(self.d / "x.png")
        (self.d / "t.png").write_bytes(make_png(4, 4, lambda x, y: 1)[:-20])
        with self.assertRaises(ValueError):
            mc.png_info(self.d / "t.png")

    def test_psd_pdf(self):
        (self.d / "a.psd").write_bytes(b"8BPS" + struct.pack(">H6sHIIHH", 1, b"\0" * 6, 4, 10, 20, 8, 3) + b"\0" * 10)
        i = mc.psd_info(self.d / "a.psd")
        self.assertEqual((i["width"], i["height"]), (20, 10))
        (self.d / "a.pdf").write_bytes(b"%PDF-1.7\n1 0 obj<</Type/Pages/Count 2>>endobj\n2 0 obj<</Type/Page>>endobj\n3 0 obj<</Type /Page>>endobj\n%%EOF\n")
        self.assertEqual(mc.pdf_info(self.d / "a.pdf")["pages"], 2)
        (self.d / "b.pdf").write_bytes(b"%PDF-1.7\n1 0 obj")
        with self.assertRaises(ValueError):
            mc.pdf_info(self.d / "b.pdf")

    def test_mp4(self):
        mvhd = box(b"mvhd", bytes([0]) + b"\0" * 3 + b"\0" * 8 + struct.pack(">II", 1000, 3000) + b"\0" * 80)
        tkhd = box(b"tkhd", bytes([0]) + b"\0" * 3 + b"\0" * 72 + struct.pack(">II", 1280 << 16, 720 << 16))
        stsd = box(b"stsd", b"\0" * 4 + struct.pack(">I", 1) + box(b"avc1", b"\0" * 20))
        stbl = box(b"stbl", stsd)
        mdia = box(b"mdia", box(b"minf", stbl))
        moov = box(b"moov", mvhd + box(b"trak", tkhd + mdia))
        p = self.d / "a.mp4"
        p.write_bytes(box(b"ftyp", b"isom" + b"\0" * 8) + moov)
        i = mc.mp4_info(p)
        self.assertEqual((i["duration_s"], i["width"], i["height"], i["codec"]), (3.0, 1280, 720, "avc1"))
        (self.d / "b.mp4").write_bytes(b"\0" * 64)
        with self.assertRaises(ValueError):
            mc.mp4_info(self.d / "b.mp4")


if __name__ == "__main__":
    unittest.main()
