# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Execute the actual mount script against a file and recording inbox-cmdlet doubles."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT=Path(__file__).resolve().parents[1]/'diesweep/desktop/mount_app_disk.ps1'
PWSH=shutil.which('pwsh') or '/root/am/pwsh/pwsh'


@unittest.skipUnless(Path(PWSH).is_file(),'needs PowerShell')
class MountTests(unittest.TestCase):
    def run_mount(self, sha=None, size=16):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);iso=root/'apps.iso';iso.write_bytes(b'fixture app disk')
            wrapper=root/'run.ps1';wrapper.write_text(r'''
$global:S4Mounted=$false
function Get-DiskImage {param($ImagePath) [pscustomobject]@{Attached=$global:S4Mounted}}
function Mount-DiskImage {param($ImagePath,$StorageType,$Access) if($Access -ne 'ReadOnly'){throw 'read-only access required'}; 'MOUNT_ACCESS='+$Access; $global:S4Mounted=$true}
& $env:MOUNT_SCRIPT -ImagePath $env:TEST_ISO -ExpectedSha256 $env:TEST_SHA -ExpectedSize $env:TEST_SIZE
''')
            env=dict(os.environ,MOUNT_SCRIPT=str(SCRIPT),TEST_ISO=str(iso),TEST_SHA=sha or hashlib.sha256(iso.read_bytes()).hexdigest(),TEST_SIZE=str(size))
            return subprocess.run([PWSH,'-NoProfile','-File',str(wrapper)],env=env,capture_output=True,text=True,timeout=20)

    def test_verified_media_mounts_read_only(self):
        p=self.run_mount();self.assertEqual(p.returncode,0,p.stderr)
        self.assertIn('S4_APPDISK_MOUNTED',p.stdout)

    def test_mount_receipt_carries_the_verified_digest(self):
        p=self.run_mount();self.assertEqual(p.returncode,0,p.stderr)
        self.assertIn(hashlib.sha256(b'fixture app disk').hexdigest(),p.stdout)

    def test_wrong_digest_cannot_be_mounted(self):
        self.assertNotEqual(self.run_mount(sha='0'*64).returncode,0)

    def test_wrong_length_cannot_be_mounted(self):
        self.assertNotEqual(self.run_mount(size=1).returncode,0)
