#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free fixtures for the Resolve GPU lane: the GPU-use assertion, the media readers, the FCPXML generator and the
asset manifest. No VM, network or third-party media (the PNG and MP4 headers are built here)."""

import importlib.util
import json
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path
from xml.dom import minidom

WIN = Path(__file__).resolve().parents[2] / "scripts/bench/windows"


def load(name):
    spec = importlib.util.spec_from_file_location("kf_" + name, WIN / (name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["kf_" + name] = mod
    spec.loader.exec_module(mod)
    return mod


G = load("gpu_use_assert")
M = load("resolve_media")


def csv(rows, t0="2026/11/16 03:00:00"):
    out = []
    for i, (gpu, mem, enc, dec, used) in enumerate(rows):
        sec = i // 2
        out.append(f"2026/11/16 03:00:{sec:02d}.{(i % 2) * 5}00, {gpu}, {mem}, {enc}, {dec}, {used}")
    return "\n".join(out)


class Assertion(unittest.TestCase):
    def need(self, **kw):
        return G.Need(**kw)

    def test_busy_render_passes(self):
        rows = [(2, 1, 0, 0, 900)] * 4 + [(80, 40, 30, 0, 2500)] * 20 + [(3, 1, 0, 0, 950)] * 2
        s = G.parse_nvidia_smi_csv(csv(rows))
        t0 = s[4].t - 0.1
        v = G.assert_gpu_used(s, self.need(encoder=True), (t0, s[23].t + 0.1))
        self.assertTrue(v["ok"], v["reasons"])
        self.assertEqual(v["summary"]["gpu"]["peak"], 80)
        self.assertEqual(v["summary"]["mem_used"]["growth"], 1600)

    def test_idle_gpu_fails_every_way(self):
        s = G.parse_nvidia_smi_csv(csv([(1, 0, 0, 0, 900)] * 24))
        v = G.assert_gpu_used(s, self.need(encoder=True, decoder=True), (s[4].t, s[22].t))
        self.assertFalse(v["ok"])
        text = " ".join(v["reasons"])
        for frag in ("utilization.gpu peaked", "memory.used grew", "utilization.encoder", "utilization.decoder"):
            self.assertIn(frag, text)

    def test_na_everywhere_is_not_a_pass(self):
        rows = "\n".join(f"2026/11/16 03:00:{i:02d}.000, [N/A], [N/A], [N/A], [N/A], 900" for i in range(12))
        s = G.parse_nvidia_smi_csv(rows)
        v = G.assert_gpu_used(s, self.need(), (s[0].t, s[-1].t))
        self.assertFalse(v["ok"])
        self.assertIn("N/A in every sample", " ".join(v["reasons"]))

    def test_single_spike_is_not_enough(self):
        rows = [(1, 0, 0, 0, 900)] * 10 + [(90, 10, 0, 0, 1500)] + [(1, 0, 0, 0, 900)] * 10
        s = G.parse_nvidia_smi_csv(csv(rows))
        v = G.assert_gpu_used(s, self.need(), (s[2].t, s[-1].t))
        self.assertFalse(v["ok"])
        self.assertIn("only 1 samples", " ".join(v["reasons"]))

    def test_too_few_samples(self):
        s = G.parse_nvidia_smi_csv(csv([(90, 10, 0, 0, 1500)] * 3))
        self.assertFalse(G.assert_gpu_used(s, self.need(), (s[0].t, s[-1].t))["ok"])
        self.assertFalse(G.assert_gpu_used([], self.need())["ok"])

    def test_garbage_lines_skipped(self):
        s = G.parse_nvidia_smi_csv("junk\n2026/11/16 03:00:00.100, 5, 1, 0, 0, 900\n2026/11/16 03:00:00, x\n")
        self.assertEqual(len(s), 1)

    def test_utc_offset(self):
        a = G.parse_nvidia_smi_csv("2026/11/16 03:00:00.000, 5, 1, 0, 0, 900")[0].t
        b = G.parse_nvidia_smi_csv("2026/11/16 03:00:00.000, 5, 1, 0, 0, 900", utc_offset_s=3600)[0].t
        self.assertEqual(a - b, 3600)

    def test_run_with_sampling_judges_and_stops(self):
        class S:
            stopped = False
            def start(self): pass
            def stop(self):
                self.stopped = True
                return csv([(50, 5, 0, 0, 1500)] * 30)
        clock = iter([1_795_000_000.0 + 3.0, 1_795_000_000.0 + 12.0])
        base = G._epoch("2026/11/16 03:00:00")
        times = iter([base + 3, base + 12])
        s = S()
        v, samples = G.run_with_sampling(lambda: "done", s, G.Need(mem_growth_mib=None), settle_s=0, clock=lambda: next(times), sleep=lambda x: None)
        self.assertTrue(s.stopped)
        self.assertTrue(v["ok"], v["reasons"])
        s2 = S()
        times = iter([base + 3, base + 12])
        def boom(): raise RuntimeError("x")
        v, _ = G.run_with_sampling(boom, s2, G.Need(mem_growth_mib=None), settle_s=0, clock=lambda: next(times), sleep=lambda x: None)
        self.assertFalse(v["ok"])
        self.assertTrue(s2.stopped)

    def test_guest_fragments(self):
        self.assertIn("utilization.encoder", G.guest_sampler_start(r"C:\kfapps\x\smi.csv"))
        with self.assertRaises(ValueError):
            G.guest_sampler_start("C:\\a'b")


def png(w, h, fn):
    raw = b"".join(b"\x00" + bytes(v for x in range(w) for v in fn(x, y)) for y in range(h))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def box(t, body):
    return struct.pack(">I4s", 8 + len(body), t) + body


def mp4(codec=b"avc1", w=1920, h=1080, n=48, ts=24):
    mdhd = box(b"mdhd", bytes(4) + bytes(8) + struct.pack(">II", ts, n) + bytes(4))
    hdlr = box(b"hdlr", bytes(8) + b"vide" + bytes(12))
    entry = struct.pack(">I", 86) + codec + bytes(6) + struct.pack(">H", 1) + bytes(16) + struct.pack(">HH", w, h) + bytes(86 - 8 - 2 - 6 - 16 - 4 - 8 + 8)
    stsd = box(b"stsd", bytes(4) + struct.pack(">I", 1) + entry)
    stts = box(b"stts", bytes(4) + struct.pack(">I", 1) + struct.pack(">II", n, 1))
    stbl = box(b"stbl", stsd + stts)
    minf = box(b"minf", stbl)
    mdia = box(b"mdia", mdhd + hdlr + minf)
    return box(b"ftyp", b"isom" + bytes(4)) + box(b"moov", box(b"trak", mdia))


class Media(unittest.TestCase):
    def test_png_and_flat_detection(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a.png"
            p.write_bytes(png(64, 48, lambda x, y: (x * 4 % 256, y * 5 % 256, 90)))
            i = M.png_info(p)
            self.assertEqual((i["width"], i["height"]), (64, 48))
            self.assertGreater(i["luma_std"], 10)
            q = Path(d) / "b.png"
            q.write_bytes(png(64, 48, lambda x, y: (0, 0, 0)))
            j = M.png_info(q)
            self.assertEqual(j["luma_mean"], 0)
            self.assertNotEqual(i["pixel_sha256"], j["pixel_sha256"])

    def test_isobmff_reader(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a.mp4"
            p.write_bytes(mp4())
            i = M.probe_isobmff(p)
            self.assertEqual((i["codec"], i["frames"], i["width"], i["height"]), ("avc1", 48, 1920, 1080))
            self.assertAlmostEqual(i["duration"], 2.0)
            p.write_bytes(mp4()[:40])
            with self.assertRaises(M.MediaError):
                M.probe_isobmff(p)
            p.write_bytes(b"")
            with self.assertRaises(M.MediaError):
                M.probe(p)

    def test_fcpxml_is_wellformed(self):
        x = M.build_fcpxml([
            {"name": "a & b", "path": "C:/kfmedia/a b.mp4", "frames": 48, "src_frames": 100, "fps": 24, "speed": 0.5, "scale": 2.0},
            {"name": "s", "path": "C:/kfmedia/s.jpg", "frames": 24},
        ])
        doc = minidom.parseString(x)
        self.assertEqual(len(doc.getElementsByTagName("asset-clip")), 2)
        self.assertIn("a%20b.mp4", x)
        self.assertEqual(doc.getElementsByTagName("sequence")[0].getAttribute("duration"), "7200/2400s")

    def test_manifest_is_pinned_and_open(self):
        a = M.load_manifest()
        self.assertGreaterEqual(len(a), 3)
        for x in a:
            self.assertIn(x["license"], M.LICENSES)
            self.assertNotIn("NC", x["license"])
        bad = json.loads(M.MANIFEST.read_text())
        bad["assets"][0]["license"] = "CC-BY-NC-4.0"
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "m.json"
            p.write_text(json.dumps(bad))
            with self.assertRaises(M.MediaError):
                M.load_manifest(p)
            bad["assets"][0]["license"] = "CC0-1.0"
            bad["assets"][0]["sha256"] = None
            p.write_text(json.dumps(bad))
            with self.assertRaises(M.MediaError):
                M.load_manifest(p)

    def test_fetch_refuses_wrong_hash(self):
        import io
        class R(io.BytesIO):
            def geturl(self): return "https://x/"
            def __enter__(self): return self
            def __exit__(self, *a): pass
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(M.MediaError):
                M.fetch_assets(Path(d), opener=lambda req, timeout: R(b"x"))


if __name__ == "__main__":
    unittest.main()
