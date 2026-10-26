#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Integrity boundary for the native rehearsal, using only tiny synthetic files."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

PATH = Path(__file__).resolve().parents[1] / 'rehearse/verify_pull.py'
SPEC = importlib.util.spec_from_file_location('verify_pull', PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PullTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / 'failure.txt').write_bytes(b'private fixture\n')
        self.row = {'path': 'failure.txt', 'bytes': 16,
                    'sha256': hashlib.sha256(b'private fixture\n').hexdigest()}
        self.write_manifest()

    def write_manifest(self, rows=None):
        (self.root / 'manifest.json').write_text(json.dumps(rows or [self.row]))

    def test_executable_name_refused_before_pull(self):
        self.row['path'] = 'payload.exe'
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'unsupported'):
            MODULE.manifest_names(self.root)

    def test_unlisted_text_refused_before_pull(self):
        self.row['path'] = 'arbitrary.txt'
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'unsupported'):
            MODULE.manifest_names(self.root)

    def test_exact_files_verified(self):
        self.assertEqual(MODULE.verify(self.root)['files'], 1)
        self.assertTrue(MODULE.verify(self.root)['hashes_ok'])

    def test_changed_bytes_refused(self):
        (self.root / 'failure.txt').write_bytes(b'PRIVATE fixture\n')
        with self.assertRaisesRegex(ValueError, 'mismatch'):
            MODULE.verify(self.root)

    def test_missing_file_refused(self):
        (self.root / 'failure.txt').unlink()
        with self.assertRaises(FileNotFoundError):
            MODULE.verify(self.root)

    def test_extra_file_refused(self):
        (self.root / 'unexpected').write_text('extra')
        with self.assertRaisesRegex(ValueError, 'unmanifested'):
            MODULE.verify(self.root)

    def test_parent_and_drive_paths_refused(self):
        for path in ('../failure.txt', '/failure.txt', 'C:/failure.txt', 'a\\failure.txt'):
            with self.subTest(path=path):
                self.row['path'] = path
                self.write_manifest()
                with self.assertRaisesRegex(ValueError, 'unsafe'):
                    MODULE.verify(self.root)

    def test_duplicate_paths_refused(self):
        self.write_manifest([self.row, self.row])
        with self.assertRaisesRegex(ValueError, 'repeated'):
            MODULE.verify(self.root)

    def test_link_outside_refused(self):
        (self.root / 'failure.txt').unlink()
        (self.root / 'failure.txt').symlink_to('/etc/hostname')
        with self.assertRaisesRegex(ValueError, 'escapes'):
            MODULE.verify(self.root)


if __name__ == '__main__':
    unittest.main()
