# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Execute the deployed desktop phase's asset guard with the rental's Python 3.10 API."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
PHASE = ROOT / 'traces/sweep_dies_20261018/lease_57193877/harness/desktop.sh'


class AssetGuardTests(unittest.TestCase):
    def guard(self, expected=None, data=b'fixture app disk'):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); iso=root/'fixture.iso'; iso.write_bytes(data)
            man=root/'scripts/bench/windows/appmatrix/manifest.json';man.parent.mkdir(parents=True)
            image=expected or dict(size=len(data),sha256=hashlib.sha256(data).hexdigest())
            man.write_text(json.dumps(dict(image=image)))
            body=re.search(r"python3 - <<'VERIFY'\n(.*?)\nVERIFY",PHASE.read_text(),re.S).group(1)
            body=body.replace('/workspace/bench/kfapps.iso',str(iso))
            prelude="import hashlib\nif hasattr(hashlib,'file_digest'): del hashlib.file_digest\n"
            return subprocess.run(['python3','-c',prelude+body],cwd=root,capture_output=True,text=True,timeout=10)

    def test_valid_disk_passes_with_python310_hashlib(self):
        p=self.guard();self.assertEqual(p.returncode,0,p.stderr)

    def test_python310_guard_emits_its_verified_marker(self):
        self.assertIn('APPDISK_HASH_OK',self.guard().stdout)

    def test_changed_payload_is_rejected(self):
        self.assertNotEqual(self.guard(dict(size=16,sha256='0'*64)).returncode,0)

    def test_wrong_size_is_rejected(self):
        self.assertNotEqual(self.guard(dict(size=1,sha256=hashlib.sha256(b'fixture app disk').hexdigest())).returncode,0)
