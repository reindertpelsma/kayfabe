# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""CPU-only execution of GravityMark's generated preamble; the benchmark is never launched.

Falsifier: failed removal leaves a stale result but the preamble still reaches the launch boundary.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import helpers as H
import gen_apps as G

PWSH = shutil.which("pwsh")


class GravityMark(unittest.TestCase):
    def test_every_mode_uses_the_checked_cleanup_preamble(self):
        for app in H.load_apps()["apps"]:
            if app["id"].startswith("gravitymark_"):
                self.assertTrue(app["ps"].startswith(G.GM), app["id"])

    def preamble(self, stale, refuse_removal=False):
        with tempfile.TemporaryDirectory(prefix="kf-gm-preamble-") as tmp:
            root = Path(tmp)
            for name in stale:
                (root / name).write_bytes(b"stale synthetic result")
            quote = str(root).replace("'", "''")
            script = "$K = '%s'\n$OUT = '%s'\n" % (quote, quote)
            script += "function Find-KfFile { param($Root, $Name) return (Join-Path $Root $Name) }\n"
            if refuse_removal:
                # Model access denial/locked output while preserving the preamble's SilentlyContinue behavior.
                script += "function Remove-Item { param([string[]]$LiteralPath, [switch]$Force, [string]$ErrorAction) }\n"
            script += G.GM + "'PREAMBLE_OK'\n"
            path = root / "preamble.ps1"
            path.write_text(script)
            env = dict(os.environ, POWERSHELL_TELEMETRY_OPTOUT="1", DOTNET_CLI_TELEMETRY_OPTOUT="1",
                       XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"))
            p = subprocess.run([PWSH, "-NoLogo", "-NoProfile", "-NonInteractive", "-File", str(path)],
                               capture_output=True, text=True, env=env, timeout=30)
            remaining = [name for name in stale if (root / name).exists()]
            return p, remaining

    @unittest.skipUnless(PWSH, "PowerShell required for the CPU-only preamble tests")
    def test_old_outputs_are_removed_before_launch(self):
        p, remaining = self.preamble(["gm.png", "gm_times.txt"])
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("PREAMBLE_OK", p.stdout)
        self.assertEqual(remaining, [])

    @unittest.skipUnless(PWSH, "PowerShell required for the CPU-only preamble tests")
    def test_locked_benchmark_image_prevents_launch(self):
        p, remaining = self.preamble(["gm.png"], refuse_removal=True)
        self.assertNotEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("CHECK gravitymark_fresh FAIL", p.stdout)
        self.assertNotIn("PREAMBLE_OK", p.stdout)
        self.assertEqual(remaining, ["gm.png"])

    @unittest.skipUnless(PWSH, "PowerShell required for the CPU-only preamble tests")
    def test_locked_timing_file_prevents_launch(self):
        p, remaining = self.preamble(["gm_times.txt"], refuse_removal=True)
        self.assertNotEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("CHECK gravitymark_fresh FAIL", p.stdout)
        self.assertNotIn("PREAMBLE_OK", p.stdout)
        self.assertEqual(remaining, ["gm_times.txt"])


if __name__ == "__main__":
    unittest.main()
