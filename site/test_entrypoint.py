"""Direct test-file execution must include the public-copy claim guards."""

from pathlib import Path
import subprocess
import sys
import unittest


class DirectEntryPoint(unittest.TestCase):
    # FALSIFIER: unittest.main runs before the final guards are defined.
    def check_selector(self, selector):
        run = subprocess.run(
            [sys.executable, str(Path(__file__).with_name("test_build.py")), selector],
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertEqual(run.returncode, 0, run.stderr[-2000:])

    def test_direct_guard_group(self):
        self.check_selector("FrontpageClaims")

    def test_direct_individual_guard(self):
        self.check_selector("FrontpageClaims.test_readme_has_no_banned_claim")
