"""Exercise the gate runner's verdict/exit/census logic without a GPU."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class GateRunnerTests(unittest.TestCase):
    def run_fixture(self, mode):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            commands = root / "bin"
            commands.mkdir()
            for name, body in (("cargo", "exit 0"), ("nvidia-smi", "echo fixture")):
                path = commands / name
                path.write_text("#!/bin/sh\n" + body + "\n")
                path.chmod(0o700)
            target = root / "custom-target" / "release"
            target.mkdir(parents=True)
            for n in range(1, 10):
                if mode == "missing" and n == 9:
                    continue
                path = target / f"kf-gate{n}"
                verdict = "FAIL" if mode == "fail" and n == 9 else "PASS"
                code = 7 if mode == "bad-exit" and n == 9 else 0
                path.write_text(f"#!/bin/sh\necho GATE{n}_VERDICT={verdict}\nexit {code}\n")
                path.chmod(0o700)
            env = dict(os.environ, PATH=f"{commands}:/usr/bin:/bin",
                       CARGO_TARGET_DIR=str(target.parent))
            return subprocess.run(["bash", str(ROOT / "scripts/bench/v3_gates.sh"),
                                   str(root / "result.log")], env=env,
                                  capture_output=True, text=True, timeout=20)

    def test_all_nine_pass_in_custom_target(self):
        result = self.run_fixture("pass")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("V3_GATES_SUMMARY pass=9 fail=0", result.stdout)

    def test_failed_missing_and_nonzero_exit_cannot_report_success(self):
        for mode in ("fail", "missing", "bad-exit"):
            with self.subTest(mode=mode):
                result = self.run_fixture(mode)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("V3_GATES_SUMMARY pass=8 fail=1", result.stdout)
