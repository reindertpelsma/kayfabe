"""Vast VM non-login environment falsifiers; never print contract values."""

import importlib.util
import json
from pathlib import Path
import shlex
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "v7_report", Path(__file__).with_name("vast_report.py")
)
report = importlib.util.module_from_spec(spec)
sys.modules["v7_report"] = report
spec.loader.exec_module(report)


class ProviderEnvTests(unittest.TestCase):
    def load(self, text, env=None):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "environment"
            path.write_text(text)
            return report.load_provider_env(env or {}, str(path))

    def test_loads_vm_protocol_from_provider_file(self):
        self.assertEqual(self.load("KF_PROTOCOL=5\n")["KF_PROTOCOL"], "5")

    def test_loads_quoted_vm_repository(self):
        self.assertEqual(self.load('KF_REPO="owner/repo"\n')["KF_REPO"], "owner/repo")

    def test_preserves_explicit_process_protocol(self):
        self.assertEqual(self.load("KF_PROTOCOL=4\n", {"KF_PROTOCOL": "5"})["KF_PROTOCOL"], "5")

    def test_preserves_explicit_process_repository(self):
        self.assertEqual(
            self.load("KF_REPO=other/repo\n", {"KF_REPO": "owner/repo"})["KF_REPO"], "owner/repo"
        )

    def test_never_imports_billed_account_key(self):
        self.assertNotIn("VAST_API_KEY", self.load("VAST_API_KEY=must-not-import\n"))

    def test_never_imports_provider_instance_key(self):
        self.assertNotIn("CONTAINER_API_KEY", self.load("CONTAINER_API_KEY=must-not-import\n"))

    def test_shell_substitution_is_data(self):
        self.assertEqual(
            self.load('KF_REPO="$(touch /must-not-execute)"\n')["KF_REPO"],
            "$(touch /must-not-execute)",
        )

    def test_duplicate_missing_protocol_refuses(self):
        with self.assertRaises(ValueError):
            self.load("KF_PROTOCOL=4\nKF_PROTOCOL=5\n")

    def test_duplicate_missing_repository_refuses(self):
        with self.assertRaises(ValueError):
            self.load("KF_REPO=a/b\nKF_REPO=c/d\n")

    def test_bounded_provider_file(self):
        with self.assertRaises(ValueError):
            self.load("#" + "x" * 65536)

    def test_provider_file_cannot_have_two_words(self):
        with self.assertRaises(ValueError):
            self.load("KF_PROTOCOL=5 extra\n")

    def test_no_provider_file_still_fails_closed_on_missing(self):
        env = report.load_provider_env({}, "/nonexistent-v7-provider-environment")
        self.assertTrue(report.validate_env(env))

    def test_check_env_uses_provider_file(self):
        from test_vast_box import good_env

        env = good_env("https://broker.example/prefix")
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "environment"
            path.write_text("".join(k + "=" + shlex.quote(v) + "\n" for k, v in env.items()))
            out = Path(d) / "checked"
            self.assertEqual(
                report.main(
                    ["check-env", "--provider-env", str(path), "--write", str(out)], env={}
                ),
                0,
            )
            self.assertIn("export KF_PROTOCOL=5", out.read_text())


if __name__ == "__main__":
    unittest.main()
