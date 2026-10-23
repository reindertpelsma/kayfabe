"""The existing provision scripts must actually use the reviewed local APT source."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SetupAptHook(unittest.TestCase):
    def invoke(self, operation):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            stub = root / "python3"
            stub.write_text("#!/usr/bin/python3\nimport json,sys\nprint(json.dumps(sys.argv[1:]))\n")
            stub.chmod(0o700)
            fake_apt = root / "apt-get"
            fake_apt.write_text("#!/bin/sh\nprintf '%s\\n' 'UNCACHED_APT'\n")
            fake_apt.chmod(0o700)
            p = subprocess.run(
                ["bash", "-c", 'source "$KF_PAYLOAD_ROOT/scripts/ci/vast_payload_hooks.sh"; apt-get "$1"', "test", operation],
                env={**os.environ, "PATH": str(root) + ":" + os.environ["PATH"],
                     "KF_PAYLOAD_ROOT": str(ROOT), "KF_APT_CACHE_ROOT": "/cache"},
                text=True, capture_output=True, timeout=5,
            )
            self.assertEqual(p.returncode, 0, p.stderr)
            return json.loads(p.stdout)

    def test_package_update_is_confined_to_the_local_cache(self):
        args = self.invoke("update")
        self.assertEqual(args, ["-I", str(ROOT / "scripts/ci/vast_setup.py"), "apt", "update"])

    def test_package_install_is_confined_to_the_local_cache(self):
        args = self.invoke("install")
        self.assertEqual(args, ["-I", str(ROOT / "scripts/ci/vast_setup.py"), "apt", "install"])
