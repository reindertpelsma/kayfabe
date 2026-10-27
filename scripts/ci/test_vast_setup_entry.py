"""The real entry shell must finish verified setup before starting any project work."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import vast_setup as setup

ROOT = Path(__file__).resolve().parents[2]


class SetupEntry(unittest.TestCase):
    def invoke(self, fail=False):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            stub = root / "python3"
            stub.write_text(
                "#!/usr/bin/python3\nimport os,sys,json\nfrom pathlib import Path\n"
                'kind="setup" if any(x.endswith("vast_setup.py") for x in sys.argv) else "payload"\n'
                'with open(os.environ["KF_TRACE"],"a") as f:f.write(json.dumps({"kind":kind,'
                '"offline":os.environ.get("CARGO_NET_OFFLINE"),'
                '"cache":os.environ.get("KF_APT_CACHE_ROOT")})+"\\n")\n'
                'if kind=="setup" and os.environ.get("KF_FAKE_SETUP_FAIL"):sys.exit(2)\n'
            )
            stub.chmod(0o700)
            trace = root / "trace"
            p = subprocess.run(
                ["bash", str(ROOT / "scripts/ci/vast_payload.sh"), "a" * 40, "b" * 32],
                env={
                    **os.environ,
                    "PATH": str(root) + ":" + os.environ["PATH"],
                    "KF_TRACE": str(trace),
                    "KF_FAKE_SETUP_FAIL": "1" if fail else "",
                },
                capture_output=True,
                text=True,
                timeout=5,
            )
            rows = [json.loads(s) for s in trace.read_text().splitlines()]
            return p.returncode, rows

    def test_cache_is_ready_before_project_work(self):
        rc, rows = self.invoke()
        self.assertEqual(rc, 0)
        self.assertEqual([x["kind"] for x in rows], ["setup", "payload"])

    def test_failed_setup_cannot_start_project_work(self):
        rc, rows = self.invoke(fail=True)
        self.assertEqual(rc, 2)
        self.assertEqual([x["kind"] for x in rows], ["setup"])

    def test_project_uses_only_verified_vendor_and_local_apt(self):
        rc, rows = self.invoke()
        self.assertEqual(rc, 0)
        self.assertEqual(rows[-1]["offline"], "true")
        self.assertEqual(rows[-1]["cache"], "/workspace/kf-setup-cache")

    def test_setup_failure_has_a_structured_result(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            with patch.object(
                setup, "restore", side_effect=setup.CacheRefused("secret-url")
            ), patch.dict(os.environ, {"KF_CI_WORKSPACE": d}), patch(
                "sys.argv", ["vast_setup.py", "restore", str(ROOT)]
            ):
                self.assertEqual(setup.main(), 2)
            result = json.loads((root / "kf-result.json").read_text())
            self.assertEqual(result["verdict"], "FAIL")
            self.assertEqual(
                result["suites"], [{"name": "setup_cache", "pass": 0, "fail": 1, "skip": 0}]
            )
            self.assertNotIn("secret-url", json.dumps(result))


if __name__ == "__main__":
    unittest.main()
