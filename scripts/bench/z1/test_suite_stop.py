"""A safety refusal must stop the suite before another guest can launch."""

from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[3]


class SuiteStop(unittest.TestCase):
    def check_stop(self, second):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            shutil.copy(REPO / "scripts/fastguest/fast_suite.sh", root / "fast_suite.sh")
            runner = root / "run_fast_guest.sh"
            runner.write_text(
                '#!/bin/sh\necho "$1" >> "$CALLS"\n'
                'echo "FAST_VERDICT=REFUSED (rootless audit unavailable)"\n'
                "exit 3\n"
            )
            result = subprocess.run(
                ["bash", str(root / "fast_suite.sh"), "fixture", "180", "--timer", second],
                env={**os.environ, "BENCH_DIR": str(root), "CALLS": str(root / "calls")},
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((root / "calls").read_text().splitlines(), ["fixture_timer"])
            self.assertIn("REFUSED", (root / "fixture_suite.out").read_text())

    def test_refusal_prevents_next_engines_guest(self):
        self.check_stop("--engines")

    def test_refusal_prevents_next_concurrency_guest(self):
        self.check_stop("--concurrency")


if __name__ == "__main__":
    unittest.main()
