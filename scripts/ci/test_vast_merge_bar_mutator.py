#!/usr/bin/env python3
"""GPU-free checks of the merge-bar mutator's real scratch-module imports."""

import json
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SanitizerMutationScratch(unittest.TestCase):
    def check_mutant(self, name):
        # FALSIFIER: a scratch module cannot import its production dependency, or
        # a missing dependency is accepted as a mutation kill instead of a red baseline.
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts/ci/merge_bar_mutants.py"), name],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=300,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("BASELINE RED", result.stderr)
        rows = json.loads(result.stdout)["cases"]
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["mutant"], name)
        self.assertGreaterEqual(rows[0]["count"], 2)
        self.assertGreaterEqual(len(set(rows[0]["killers"])), 2)
        self.assertFalse(any(name.endswith(".test.js") for name in rows[0]["killers"]))

    def test_scratch_module_keeps_tokens_private(self):
        self.check_mutant("sanitize-keeps-every-long-run")

    def test_scratch_module_keeps_test_names_visible(self):
        self.check_mutant("sanitize-keeps-no-long-run")


if __name__ == "__main__":
    unittest.main()
