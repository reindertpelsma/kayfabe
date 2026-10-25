"""Synthetic pre-upload failure capture; no remote access."""

import datetime as dt
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from backend import backend_type
from test_backend import Base


class ConsoleBase(Base):
    def capture(self, iid):
        return b"provider console", hashlib.sha256(b"provider console").hexdigest()


class Preupload(unittest.TestCase):
    def fixture(self, root):
        provider = SimpleNamespace(
            Backend=ConsoleBase,
            utc=lambda: "synthetic",
            save=lambda path, obj: path.write_text(json.dumps(obj)),
        )
        end = dt.datetime.now(dt.timezone.utc) + dt.timedelta(minutes=15)
        config = dict(
            workspace=str(root),
            evidence=str(root / "evidence"),
            report=str(root / "report.md"),
            operator=str(root),
            kit=str(root),
            bw=str(root),
        )
        b = backend_type(provider)({}, dict(deadline_utc=end.isoformat()), 32, config)
        row = dict(
            id=77,
            ssh_ready_utc="synthetic",
            created_utc=dt.datetime.now(dt.timezone.utc).isoformat(),
        )
        (root / "evidence/owned.json").write_text(json.dumps([row]))
        return b

    def test_admitted_but_no_upload_uses_console(self):
        with tempfile.TemporaryDirectory() as tmp:
            b = self.fixture(Path(tmp))
            self.assertEqual(b.capture(77)[0], b"provider console")

    def test_admitted_bundle_not_uploaded_uses_console(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            b = self.fixture(root)
            (root / "bundle.tar.gz").write_bytes(b"local only")
            self.assertEqual(b.capture(77)[0], b"provider console")

    def failure(self, kind):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            b = self.fixture(root)
            modules = {
                "b1n_authority": SimpleNamespace(build=lambda **kw: {}),
                "saved_xid32_observation": SimpleNamespace(DIE_PAIRS={}),
            }
            response = dict(return_value=subprocess.CompletedProcess([], 1, "", "failed"))
            if kind == "exception":
                response = dict(side_effect=FileNotFoundError("fixture"))
            with patch.dict(sys.modules, modules), patch("subprocess.run", **response):
                with self.assertRaises((ValueError, FileNotFoundError)):
                    b.run(77)
            row = json.loads((root / "evidence/owned.json").read_text())[0]
            self.assertFalse(row.get("work_started", False))

    def test_upload_exit_failure_not_remote_work(self):
        self.failure("exit")

    def test_upload_launch_exception_not_remote_work(self):
        self.failure("exception")
