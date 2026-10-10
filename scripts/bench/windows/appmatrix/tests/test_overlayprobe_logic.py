# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host-compiles tools/src/kf_overlayprobe_logic_test.cpp (pattern, YUV values, decoder, tracker, verdict, scenarios,
JSON writer of kf_overlayprobe) and runs it. Skipped when no host C++ compiler is installed."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest

import helpers as H


class OverlayProbeLogic(unittest.TestCase):
    def test_logic(self):
        cxx = shutil.which("g++") or shutil.which("c++")
        if not cxx:
            self.skipTest("no host C++ compiler")
        src = os.path.join(H.AM, "tools", "src", "kf_overlayprobe_logic_test.cpp")
        with tempfile.TemporaryDirectory() as d:
            exe = os.path.join(d, "t")
            subprocess.run([cxx, "-O1", "-std=c++17", "-Wall", "-Wextra", "-o", exe, src], check=True)
            r = subprocess.run([exe], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout)
        lines = r.stdout.strip().splitlines()
        self.assertTrue(lines[0].startswith("OK "), lines[0])
        self.assertEqual(json.loads(lines[-1])["arr"][2], {"x": "y"})


if __name__ == "__main__":
    unittest.main()
