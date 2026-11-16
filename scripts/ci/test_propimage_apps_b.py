#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/bench/windows/propimage_apps_b.py (2026-11-16): the guest script assembly, the non-elevated task, and the
host grading of each row from synthetic guest output (good files, blank files, stale files, a failed GPU assertion, launch-only rows)."""
import json
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "bench/windows"))
sys.path.insert(0, str(HERE))
import propimage_apps_b as B  # noqa: E402
import test_media_check as tm  # noqa: E402

GPU_OK = {"required": ["gpu"], "verdict": "PASS", "peak": {}, "baseline": {}, "samples": 10, "reasons": []}
GPU_BAD = {"required": ["gpu"], "verdict": "FAIL", "peak": {}, "baseline": {}, "samples": 10, "reasons": ["gpu engine idle: 0 sample(s)"]}


def png(path, w, h, noisy=True):
    path.write_bytes(tm.make_png(w, h, (lambda x, y: (x * 37 + y * 11) & 255) if noisy else (lambda x, y: 9)))


def psd(path, w, h):
    path.write_bytes(b"8BPS" + struct.pack(">H6sHIIHH", 1, b"\0" * 6, 3, h, w, 8, 3) + b"\0" * 20)


class T(unittest.TestCase):
    def setUp(self):
        self.td = tempfile.TemporaryDirectory()
        self.d = Path(self.td.name)

    def tearDown(self):
        self.td.cleanup()

    def result(self, app, outputs, **extra):
        r = {"app": app, "started": 1000.0, "ended": 1100.0, "error": None, "quit": "clean",
             "outputs": [{"file": f, "bytes": (self.d / f).stat().st_size, "modified": 1050.0} for f in outputs]}
        r.update(extra)
        (self.d / "result.json").write_text(json.dumps(r))

    def photoshop(self, w=2048, h=1339, filters="smartSharpen," * 3, noisy=True, gpuinfo="Graphics Processor: NVIDIA GeForce RTX 4070",
                  features="canvas8k,16bit,iccP3,layers100,smart_sharpen,brush_strokes", layers="101", blend="10"):
        png(self.d / "out.png", w, h, noisy)
        psd(self.d / "out.psd", w, h)
        self.result("photoshop", ["out.png", "out.psd"], source="ktiponissi_dirfys_4480x2928.jpg",
                    js={"w0": "4480", "h0": "2928", "w1": str(w), "h1": str(h), "features": filters and features, "layers": layers, "blend_modes": blend,
                       "gpuinfo": gpuinfo, "ms": "1234"})

    def test_render_and_task(self):
        s = B.render_check("photoshop", "a" * 32)
        self.assertIn("$D = 'C:\\kfapps\\" + "a" * 32 + "\\photoshop'", s)
        self.assertNotIn(". 'C:\\kfhw\\lib.ps1'", s)
        self.assertIn("function Dismiss-Dialogs", s)
        with self.assertRaises(ValueError):
            B.render_check("photoshop", "../x")
        t = B.limited_task("app_x")
        self.assertIn("-RunLevel Limited", t)
        self.assertNotIn("Highest", t)

    def test_every_check_has_a_script_and_grader(self):
        for name, c in B.CHECKS.items():
            self.assertTrue((B.APPS_DIR / c["script"]).is_file(), name)
            self.assertTrue(name in B.GRADERS or name in B.LAUNCH_ONLY, name)

    def test_script_run_names_match_the_check_names(self):
        import re
        for name, c in B.CHECKS.items():
            text = (B.APPS_DIR / c["script"]).read_text()
            m = re.search(r"Init-Run ('([a-z]+)'|\$APPNAME)", text)
            self.assertTrue(m, name)
            if m.group(2):
                self.assertIn(m.group(2), (name, "sniffers") if name == "gpudiag" else (name,), name)

    def test_photoshop_pass(self):
        self.photoshop()
        row = B.verdict("photoshop", "x" * 32, self.d, GPU_OK)
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["evidence"], "FILE")

    def test_photoshop_failures(self):
        self.photoshop(noisy=False)
        self.assertIn("blank", B.verdict("photoshop", "x", self.d, GPU_OK)["error"])
        self.photoshop(w=1000, h=500)
        self.assertIn("final width", B.verdict("photoshop", "x", self.d, GPU_OK)["error"])
        self.photoshop(features="canvas8k,16bit,iccP3,layers100")
        self.assertIn("smart_sharpen", B.verdict("photoshop", "x", self.d, GPU_OK)["error"])
        self.photoshop(blend="3")
        self.assertIn("blend modes", B.verdict("photoshop", "x", self.d, GPU_OK)["error"])
        self.photoshop(features="canvas8k,layers100,smart_sharpen")
        self.assertIn("fewer than 5", B.verdict("photoshop", "x", self.d, GPU_OK)["error"])
        self.photoshop()
        row = B.verdict("photoshop", "x", self.d, GPU_BAD)
        self.assertFalse(row["ok"])
        self.assertIn("GPU assertion failed", row["error"])
        self.photoshop(gpuinfo="GPU not supported")
        self.assertIn("NVIDIA", B.verdict("photoshop", "x", self.d, GPU_OK)["error"])

    def test_stale_output_rejected(self):
        self.photoshop()
        r = json.loads((self.d / "result.json").read_text())
        r["outputs"][0]["modified"] = 5.0
        (self.d / "result.json").write_text(json.dumps(r))
        self.assertIn("not written during this run", B.verdict("photoshop", "x", self.d, GPU_OK)["error"])

    def test_unclean_exit_and_guest_error(self):
        self.photoshop()
        r = json.loads((self.d / "result.json").read_text())
        r["quit"] = "forced"
        (self.d / "result.json").write_text(json.dumps(r))
        self.assertIn("exit cleanly", B.verdict("photoshop", "x", self.d, GPU_OK)["error"])
        r["quit"], r["error"] = "clean", "boom"
        (self.d / "result.json").write_text(json.dumps(r))
        self.assertEqual(B.verdict("photoshop", "x", self.d, GPU_OK)["error"], "boom")

    def test_launch_only_is_limited(self):
        (self.d / "result.json").write_text(json.dumps({"app": "premiere", "started": 1.0, "ended": 9.0, "error": None, "quit": "clean", "outputs": [],
                                                         "main_window": True, "alive": True, "responding": True, "titles": ["a"], "private_mb": 300}))
        row = B.verdict("premiere", "x", self.d, None)
        self.assertTrue(row["ok"] and row["limited"], row)
        self.assertEqual(row["status"], "PASS_LIMITED")
        self.assertIn("LIMITED", row["signal"])
        (self.d / "result.json").write_text(json.dumps({"app": "premiere", "started": 1.0, "ended": 9.0, "error": None, "quit": "clean", "outputs": [],
                                                         "main_window": False, "alive": True}))
        self.assertFalse(B.verdict("premiere", "x", self.d, None)["ok"])

    def test_sample_rows(self):
        png(self.d / "out.png", 400, 300)
        (self.d / "out.pdf").write_bytes(b"%PDF-1.4\n1 0 obj<</Type/Page>>endobj\n%%EOF\n")
        self.result("illustratorsample", ["out.png", "out.pdf"], source="x.ai", js={"w0": "200", "h0": "150", "paths": "9", "scale": "200"})
        self.assertTrue(B.verdict("illustratorsample", "x", self.d, None)["ok"])
        self.result("illustratorsample", ["out.png", "out.pdf"], source="x.ai", js={"w0": "999", "h0": "150"})
        self.assertFalse(B.verdict("illustratorsample", "x", self.d, None)["ok"])
        png(self.d / "out.png", 640, 480)
        self.result("animatesample", ["out.png"], source="Hippo.fla", jsfl_log="opened,size=640x480,frames=10,layers=2,png")
        self.assertTrue(B.verdict("animatesample", "x", self.d, None)["ok"])
        self.result("animatesample", ["out.png"], source="Hippo.fla", jsfl_log="ERR boom")
        self.assertIn("did not open", B.verdict("animatesample", "x", self.d, None)["error"])
        png(self.d / "out.png", 1920, 1080)
        self.result("photoshopsample", ["out.png"], source="s", js={"tw": "3840", "th": "2160", "w1": "1920", "h1": "1080", "sharpen": "4", "w0": "6000", "h0": "4000"})
        self.assertTrue(B.verdict("photoshopsample", "x", self.d, GPU_OK)["ok"])
        self.result("photoshopsample", ["out.png"], source="s", js={"tw": "100", "th": "2160", "w1": "1920", "h1": "1080", "sharpen": "4"})
        self.assertIn("3840x2160", B.verdict("photoshopsample", "x", self.d, GPU_OK)["error"])

    def test_project_corpus_rows(self):
        self.result("aeprojects", [], count=3, rows="OK a.aep:I3:C1:L4||OK b.aep:I1:C1:L1||ERR c.aep:missing footage")
        self.assertTrue(B.verdict("aeprojects", "x", self.d, None)["ok"])
        self.result("aeprojects", [], count=3, rows="ERR a||ERR b||OK c.aep:I1:C0:L0")
        self.assertIn("1 of 3", B.verdict("aeprojects", "x", self.d, None)["error"])
        self.result("aeprojects", [], count=3, rows="OK a")
        self.assertIn("reported 1 of 3", B.verdict("aeprojects", "x", self.d, None)["error"])
        (self.d / "q0.pdf").write_bytes(b"%PDF-1.4\n1 0 obj<</Type/Page>>endobj\n%%EOF\n")
        self.result("indesignprojects", ["q0.pdf"], count=1, rows="OK x.indd:P1")
        self.assertTrue(B.verdict("indesignprojects", "x", self.d, None)["ok"])

    def test_gpudiag(self):
        (self.d / "sniffer_premiere.txt").write_text("CUDA device found: NVIDIA GeForce RTX 4070\n")
        self.result("sniffers", ["sniffer_premiere.txt"])
        self.assertTrue(B.verdict("gpudiag", "x", self.d, None)["ok"])
        (self.d / "sniffer_premiere.txt").write_text("No CUDA devices Found\n")
        self.result("sniffers", ["sniffer_premiere.txt"])
        self.assertIn("no compute device", B.verdict("gpudiag", "x", self.d, None)["error"])


if __name__ == "__main__":
    unittest.main()
