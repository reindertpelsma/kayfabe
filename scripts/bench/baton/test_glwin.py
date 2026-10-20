#!/usr/bin/env python3
"""GPU-free integration test for glwin.c: builds it and runs it against Xvfb (Mesa llvmpipe).

Skipped when gcc, Xvfb or the X11/GL runtime libraries are missing.  Checks the log format
(read back with baton_parse), the dump CSV, calibration output and --require-renderer.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import baton_parse as bp  # noqa: E402


@unittest.skipUnless(shutil.which("gcc") and shutil.which("Xvfb"), "needs gcc and Xvfb")
class GlwinXvfb(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="glwin-test-")
        cls.exe = os.path.join(cls.tmp, "glwin")
        subprocess.run(["gcc", "-O2", "-Wall", "-Werror", "-o", cls.exe, os.path.join(HERE, "glwin.c"), "-ldl", "-lm"],
                       check=True)
        cls.disp = None
        for n in range(90, 110):
            if os.path.exists("/tmp/.X11-unix/X%d" % n) or os.path.exists("/tmp/.X%d-lock" % n):
                continue
            cls.xvfb = subprocess.Popen(["Xvfb", ":%d" % n, "-screen", "0", "800x600x24", "+extension", "GLX"],
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for _ in range(50):
                if os.path.exists("/tmp/.X11-unix/X%d" % n):
                    cls.disp = ":%d" % n
                    break
                time.sleep(0.1)
            break
        if not cls.disp:
            raise unittest.SkipTest("could not start Xvfb")
        time.sleep(0.5)
        cls.env = dict(os.environ, DISPLAY=cls.disp, LIBGL_ALWAYS_SOFTWARE="1")

    @classmethod
    def tearDownClass(cls):
        if getattr(cls, "xvfb", None):
            cls.xvfb.terminate()
            cls.xvfb.wait(10)
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def run_glwin(self, *args):
        return subprocess.run([self.exe, *args], env=self.env, capture_output=True, text=True, timeout=60)

    def test_log_is_baton_format_and_dump_agrees(self):
        dump = os.path.join(self.tmp, "A.csv")
        r = self.run_glwin("--label", "A", "--duration", "2", "--layers", "4", "--w", "200", "--h", "150",
                           "--dump-all", dump)
        self.assertEqual(r.returncode, 0, r.stderr)
        log = bp.parse_log(r.stdout)
        self.assertEqual(log["label"], "A")
        self.assertEqual(log["duration_s"], 2)
        self.assertIsNotNone(log["t0_epoch"])
        self.assertIsNotNone(log["summary"])
        its = bp.parse_dump(open(dump).read())
        self.assertGreater(len(its), 10)
        self.assertEqual(len(its), log["summary"]["iterations"])
        st = bp.compute_stats(its, 2)
        self.assertAlmostEqual(st["longest_gap_ms"], log["summary"]["longest_gap_ms"], places=0)

    def test_canary_flag_and_start_at(self):
        start = time.time() + 2
        r = self.run_glwin("--label", "C", "--duration", "1", "--canary", "--w", "100", "--h", "80",
                           "--start-at", "%.3f" % start)
        self.assertEqual(r.returncode, 0, r.stderr)
        log = bp.parse_log(r.stdout)
        self.assertGreaterEqual(log["t0_epoch"], start - 0.05)
        self.assertIn("canary=1", r.stdout)

    def test_calibrate_prints_layers(self):
        r = self.run_glwin("--calibrate-only", "--target-ms", "2", "--w", "100", "--h", "80")
        self.assertEqual(r.returncode, 0, r.stderr)
        line = [l for l in r.stdout.splitlines() if l.startswith("LAYERS=")]
        self.assertEqual(len(line), 1)
        self.assertGreaterEqual(int(line[0].split("=")[1]), 1)

    def test_require_renderer_refuses_software(self):
        r = self.run_glwin("--require-renderer", "NVIDIA", "--duration", "1")
        self.assertEqual(r.returncode, 3)
        self.assertIn("does not contain", r.stderr)

    def test_no_display_exits_2(self):
        env = dict(self.env, DISPLAY=":199")
        r = subprocess.run([self.exe, "--duration", "1"], env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 2)


if __name__ == "__main__":
    unittest.main()
