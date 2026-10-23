"""Restoring dependency files must not bypass the existing host APT safety guard."""
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import vast_setup as setup


class GuardOrder(unittest.TestCase):
    def restored_operations(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "scripts/ci").mkdir(parents=True)
            lock = {
                "tag": "setup-cache-test",
                "assets": [{"kind": k, "name": k + ".tar.gz", "size": 4, "sha256": "0" * 64}
                           for k in ["debs", "vendor"]],
            }
            (root / "scripts/ci/setup-cache.lock.json").write_text(json.dumps(lock))
            operations = []

            def download(locator, asset, dest):
                Path(dest).write_bytes(b"data")

            def extract(archive, dest, kind):
                (Path(dest) / kind).mkdir(parents=True)

            def apt(cache, args):
                operations.append(args[0])
                return 0

            with patch.object(setup, "validate_lock"), patch.object(setup, "wait_for_ready"), patch.object(
                setup, "policy_digest", return_value="policy"
            ), patch.object(setup, "package_names", return_value=["python3"]), patch.object(
                setup.subprocess, "check_output", return_value=b"{}"
            ), patch.object(setup, "download_asset", side_effect=download), patch.object(
                setup, "unpack_checked", side_effect=extract
            ), patch.object(setup, "configure_vendor"), patch.object(
                setup, "apt", side_effect=apt
            ), patch.object(setup.subprocess, "run", return_value=SimpleNamespace(returncode=0)) as run, redirect_stdout(io.StringIO()):
                setup.restore(root, root / "cache")
                for call in run.call_args_list:
                    command = call.args[0]
                    if Path(command[0]).name == "apt-get":
                        operations.append(command[1])
            return operations

    def test_update_waits_for_the_existing_provisioner_guard(self):
        self.assertNotIn("update", self.restored_operations())

    def test_install_waits_for_the_existing_provisioner_guard(self):
        self.assertNotIn("install", self.restored_operations())
