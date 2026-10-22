"""Synthetic public RPC headers: preserve useful status without exporting bodies."""
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

HELPER = Path(__file__).resolve().parents[1] / 'diesweep/desktop/alloc_headers.py'


def record(function=103, cls=0xCFB0, status=0, params=16, saved=64):
    body = bytearray(b'PRIVATE-HANDLE' + b'\0' * 51)
    struct.pack_into('<III', body, 12, cls, status, params)
    return ('kf-gsp: reply-body record=1 message_seq=7 function=' + hex(function) +
            ' selector=' + hex(cls) + ' rpc_result=0x0 rpc_result_private=0x0 total=64 saved=' +
            str(saved) + ' truncated=' + str(int(saved < 64)) + ' body=' + body[:saved].hex() + '\n')


class AllocationProjection(unittest.TestCase):
    def project(self, source):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td); log = root / 'qemu.log'; out = root / 'alloc.jsonl'
            log.write_text(source)
            proc = subprocess.run([sys.executable, str(HELPER), str(log), str(out)],
                                  capture_output=True, text=True)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            return ([json.loads(x) for x in out.read_text().splitlines()],
                    out.read_text(), json.loads(out.with_suffix('.receipt.json').read_text()))

    def test_nonzero_body_status_is_retained(self):
        rows, text, receipt = self.project(record(status=0x56, params=24))
        self.assertEqual(rows[0]['body_status'], '0x56')
        self.assertEqual(rows[0]['declared_params_bytes'], 24)
        self.assertEqual(rows[0]['class_from_header'], '0xcfb0')
        self.assertNotIn('PRIVATE-HANDLE', text)
        self.assertNotIn('body=', text)
        self.assertNotIn(b'PRIVATE-HAND'.hex(), text)
        self.assertNotIn('body', rows[0])
        self.assertFalse(rows[0]['private_body_exported'])
        self.assertEqual(receipt['posted_allocation_records'], 1)

    def test_other_class_and_status_are_decoded_independently(self):
        rows, text, _ = self.project(record(cls=0xCA7A, status=0x1F, params=40))
        self.assertEqual(rows[0]['body_status'], '0x1f')
        self.assertEqual(rows[0]['declared_params_bytes'], 40)
        self.assertEqual(rows[0]['class_from_header'], '0xca7a')
        self.assertNotIn('PRIVATE-HANDLE', text)
        self.assertNotIn('body=', text)
        self.assertNotIn(b'PRIVATE-HAND'.hex(), text)
        self.assertNotIn('body', rows[0])
        self.assertFalse(rows[0]['private_body_exported'])

    def test_nonallocation_reply_is_excluded(self):
        rows, _, _ = self.project(record(function=76))
        self.assertEqual(rows, [])

    def test_mixed_replies_count_only_allocations(self):
        rows, _, receipt = self.project(record(function=76) + record(status=7))
        self.assertEqual(len(rows), 1)
        self.assertEqual(receipt['posted_allocation_records'], 1)
        self.assertEqual(rows[0]['body_status'], '0x7')

    def test_short_saved_header_remains_explicitly_truncated(self):
        rows, _, _ = self.project(record(saved=16))
        self.assertEqual(rows[0]['header_decode'], 'TRUNCATED')
        self.assertNotIn('body_status', rows[0])


if __name__ == '__main__':
    unittest.main()
