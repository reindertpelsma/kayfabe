# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import glob
import os
import py_compile
import re
import shutil
import subprocess
import tempfile
import unittest

import helpers as H


class GuestScripts(unittest.TestCase):
    def test_powershell_scripts_parse(self):
        pw = H.find_pwsh()
        if not pw:
            self.skipTest("no pwsh (set KF_PWSH)")
        files = sorted(glob.glob(os.path.join(H.AM, "guest", "*.ps1")) + [os.path.join(H.AM, "..", "d3d12_signal_probe.ps1")])
        self.assertGreaterEqual(len(files), 9)
        out = H.pwsh_parse(files)
        self.assertIn("PARSED_BAD=0", out, out)

    def test_powershell_5_1_compatibility(self):
        """The guest has Windows PowerShell 5.1: no ??, no ?:, no -AsHashtable, no && / || pipeline chains."""
        for f in glob.glob(os.path.join(H.AM, "guest", "*.ps1")):
            txt = re.sub(r"'[^'\n]*'", "''", open(f).read())      # ignore string literals
            txt = "\n".join(l.split("#", 1)[0] for l in txt.splitlines())
            for bad in (r"\?\?", r"-AsHashtable", r"\)\s*\?\s", r"\s&&\s", r"\|\|"):
                self.assertIsNone(re.search(bad, txt), f"{os.path.basename(f)} uses PS7-only syntax {bad}")

    def test_python_files_compile(self):
        for f in glob.glob(os.path.join(H.AM, "guest", "py", "*.py")) + glob.glob(os.path.join(H.AM, "*.py")) + glob.glob(os.path.join(H.AM, "tests", "*.py")):
            with tempfile.TemporaryDirectory() as d:
                py_compile.compile(f, cfile=os.path.join(d, "x.pyc"), doraise=True)

    def test_web_pages_script_syntax(self):
        node = shutil.which("node")
        if not node:
            self.skipTest("no node")
        for f in glob.glob(os.path.join(H.AM, "guest", "web", "*.html")):
            js = re.search(r"<script>(.*)</script>", open(f).read(), re.S).group(1)
            with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False) as t:
                t.write(js)
            r = subprocess.run([node, "--check", t.name], capture_output=True, text=True)
            os.unlink(t.name)
            self.assertEqual(r.returncode, 0, f + r.stderr)

    def test_every_pushed_file_exists(self):
        import winapps
        for g, rel in winapps.PS_PUSH.items():
            self.assertTrue(os.path.exists(os.path.join(H.AM, rel)), rel)
        for p in winapps.PY_OWN:
            self.assertTrue(os.path.exists(os.path.join(H.AM, "guest", "py", p)), p)
        for p in winapps.PY_LINUX:
            self.assertTrue(os.path.exists(os.path.join(H.REPO, "scripts", "apps", "src", p)), p)
        for w in winapps.WEB:
            self.assertTrue(os.path.exists(os.path.join(H.AM, "guest", "web", w + ".html")), w)

    def test_win_cuda_equiv_lists_every_test_the_inventory_uses(self):
        import json
        doc = H.load_apps()
        src = open(os.path.join(H.AM, "guest", "py", "win_cuda_equiv.py")).read()
        defined = set(re.findall(r"^def t_(\w+)\(", src, re.M)) | {"stream_" + n for n in ("default", "created", "nonblocking", "perthread", "two", "created2nd")}
        for a in doc["apps"]:
            m = re.search(r"win_cuda_equiv\.py', '(\w+)'", a["ps"])
            if m:
                self.assertIn(m.group(1), defined, a["id"])


if __name__ == "__main__":
    unittest.main()
