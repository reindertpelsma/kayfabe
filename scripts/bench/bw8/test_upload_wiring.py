"""Synthetic current-ledger and current-kit upload regression; no provider access."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

try:
    from upload_wiring import relocate_upload
except ImportError:

    def relocate_upload(text, **paths):
        # Reproduce the failed string-only relocation: derived K/ledger remain historical.
        for key in ("D", "W", "O"):
            text = text.replace('Path("/fixture/' + key + '")', repr_path(paths[key]))
        return text


def repr_path(path):
    return "Path(" + repr(str(path)) + ")"


SOURCE = """from pathlib import Path
import json, os, tarfile, hashlib, subprocess
D = Path("/fixture/D")
W = Path("/fixture/W")
O = Path("/fixture/O")
K = W / "history/run_kit"
probes = Path("/fixture/probes")
iid = int((D / "own-id").read_text())
r = next(x for x in json.loads((W / "history/owned_instances.json").read_text())
         if x["id"] == iid)
assert r["auto_destroy_verified"]
os.kill(r["auto_destroy_pid"], 0)
files = {"controller.sh": K / "controller.sh", "ledger.json": D / "own-id"}
bundle = D / "bundle.tar.gz"
with tarfile.open(bundle, "w:gz") as tar:
    for name, path in files.items():
        tar.add(path, arcname=name, recursive=False)
(D / "bundle_receipt.json").write_text(json.dumps({
    "bundle_sha256": hashlib.sha256(bundle.read_bytes()).hexdigest(),
    "files": {name: hashlib.sha256(path.read_bytes()).hexdigest()
              for name, path in files.items()}}))
"""


class UploadBindings(unittest.TestCase):
    def fixture(self, root):
        paths = {key: root / key for key in ("D", "W", "O", "K", "probes")}
        for p in paths.values():
            p.mkdir()
        history = paths["W"] / "history"
        (history / "run_kit").mkdir(parents=True)
        (history / "owned_instances.json").write_text("[]")
        (history / "run_kit/controller.sh").write_text("WRONG HISTORICAL KIT")
        ledger = root / "current_owned.json"
        ledger.write_text(
            json.dumps([dict(id=77, auto_destroy_verified=True, auto_destroy_pid=os.getpid())])
        )
        paths["ledger"] = ledger
        (paths["D"] / "own-id").write_text("77")
        (paths["K"] / "controller.sh").write_text("CURRENT COMPLETION OVERLAY")
        extra = root / "extra.py"
        extra.write_text("CURRENT OBSERVER")
        paths["extra_files"] = {"guard/observer.py": str(extra)}
        return paths

    def run_upload(self, paths, source=SOURCE):
        program = paths["D"] / "upload.py"
        program.write_text(relocate_upload(source, **paths))
        return subprocess.run(["python3", str(program)], capture_output=True, text=True)

    def test_empty_historical_ledger_uses_current_lease(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = self.fixture(Path(tmp))
            result = self.run_upload(paths)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_historical_wrong_id_uses_current_lease(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = self.fixture(Path(tmp))
            (paths["W"] / "history/owned_instances.json").write_text('[{"id":88}]')
            result = self.run_upload(paths)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_current_kit_is_in_bundle(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = self.fixture(Path(tmp))
            (paths["W"] / "history/owned_instances.json").write_text(paths["ledger"].read_text())
            result = self.run_upload(paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            with tarfile.open(paths["D"] / "bundle.tar.gz") as tar:
                self.assertEqual(
                    tar.extractfile("controller.sh").read(), b"CURRENT COMPLETION OVERLAY"
                )

    def test_current_kit_hash_is_receipted(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = self.fixture(Path(tmp))
            (paths["W"] / "history/owned_instances.json").write_text(paths["ledger"].read_text())
            result = self.run_upload(paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt = json.loads((paths["D"] / "bundle_receipt.json").read_text())
            self.assertEqual(
                receipt["files"]["controller.sh"],
                hashlib.sha256((paths["K"] / "controller.sh").read_bytes()).hexdigest(),
            )

    def test_observer_extra_in_bundle(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = self.fixture(Path(tmp))
            result = self.run_upload(paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            with tarfile.open(paths["D"] / "bundle.tar.gz") as tar:
                self.assertEqual(tar.extractfile("guard/observer.py").read(), b"CURRENT OBSERVER")

    def test_observer_extra_hash_receipted(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = self.fixture(Path(tmp))
            result = self.run_upload(paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt = json.loads((paths["D"] / "bundle_receipt.json").read_text())
            self.assertIn("guard/observer.py", receipt["files"])

    def test_missing_ledger_anchor_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = self.fixture(Path(tmp))
            with self.assertRaises(ValueError):
                relocate_upload(SOURCE.replace("owned_instances.json", "unknown.json"), **paths)

    def test_duplicate_ledger_anchor_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = self.fixture(Path(tmp))
            with self.assertRaises(ValueError):
                relocate_upload(
                    SOURCE + '\nx = (W / "owned_instances.json").read_text()\n', **paths
                )

    def test_missing_kit_binding_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):
                relocate_upload(SOURCE.replace("K = W", "KK = W"), **self.fixture(Path(tmp)))

    def test_duplicate_kit_binding_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):
                relocate_upload(SOURCE + "\nK = W\n", **self.fixture(Path(tmp)))
