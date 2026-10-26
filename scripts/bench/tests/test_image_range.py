#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Producer emits exact byte ranges to a pipe, and rejects invalid ranges before emitting."""
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'diesweep' / 'stream_image_range.py'


class ImageRangeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / 'image with spaces'
        self.data = bytes(range(256)) * 1025
        self.path.write_bytes(self.data)

    def read_range(self, offset, count, path=None):
        return subprocess.run(['python3', str(SCRIPT), str(path or self.path), str(offset), str(count)],
                              capture_output=True, timeout=5)

    def test_exact_ranges_include_multiblock_and_last_byte(self):
        for offset, count in [(0, 1), (0, len(self.data)), (700, 132000), (len(self.data)-1, 1)]:
            with self.subTest(offset=offset, count=count):
                r = self.read_range(offset, count)
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertEqual(r.stdout, self.data[offset:offset+count])

    def test_invalid_or_truncated_range_emits_no_bytes(self):
        for offset, count in [(-1, 1), (0, 0), (0, (1 << 30)+1), ((1 << 63), 1),
                              (len(self.data), 1), (len(self.data)-1, 2)]:
            with self.subTest(offset=offset, count=count):
                r = self.read_range(offset, count)
                self.assertNotEqual(r.returncode, 0)
                self.assertEqual(r.stdout, b'')

    def test_non_regular_source_is_refused(self):
        r = self.read_range(0, 1, Path('/dev/zero'))
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(r.stdout, b'')


if __name__ == '__main__':
    unittest.main()
