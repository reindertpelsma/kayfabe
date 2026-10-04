# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import importlib.util
from pathlib import Path
import struct
import unittest
spec = importlib.util.spec_from_file_location('decode', Path(__file__).parents[1] / 'decode.py')
decode = importlib.util.module_from_spec(spec)
spec.loader.exec_module(decode)


def header():
    return decode.FILE.pack(0x5457474b, 1, 64, 64, 10_000_000, 0, 1, 0, 0, 0, 0)


def record(direction=0, seq=10, rpc_seq=7, status=0, max_slots=8):
    p = bytearray(160)
    struct.pack_into('<II', p, 36, seq, 1)
    struct.pack_into('<8I', p, 48, 0x03000000, 0x43505256, 112, 76, 0, 0, rpc_seq, 0)
    struct.pack_into('<5I', p, 80, 0xcafe, 0xbeef, 0x2080121f, status, 40)
    struct.pack_into('<II4Q', p, 120, max_slots, 4096 if direction else 0, 512, 256, 32768, 4096)
    check = 0
    for (v,) in struct.iter_unpack('<I', p):
        check ^= v
    struct.pack_into('<I', p, 32, check)
    return decode.RECORD.pack(0x5247474b, 64, len(p), direction, 33, 0x1000,
                              seq, rpc_seq, 76, 0, 3, 0, 0x03000000, 0) + p


class DecoderTest(unittest.TestCase):
    def test_query_pair(self):
        result = decode.summarize(decode.parse(header() + record() + record(1)))
        self.assertFalse(result['complete'])
        self.assertTrue(result['unambiguous_query_pairs'][0]['successful'])
        self.assertEqual(result['gfx_pool_observations'][1]['gfx_pool']['poolSize'], 32768)

    def test_no_pair_or_failure(self):
        for records in (record(), record() + record(1, rpc_seq=99), record() + record(1, max_slots=9)):
            self.assertEqual(decode.summarize(decode.parse(header() + records))['unambiguous_query_pairs'], [])
        self.assertFalse(decode.summarize(decode.parse(header() + record() + record(1, status=0x56)))['unambiguous_query_pairs'][0]['successful'])

    def test_large_control_continuation_is_preserved(self):
        blob = bytearray(header() + record())
        struct.pack_into('<I', blob, 128 + 88, 0x20800a32)
        struct.pack_into('<I', blob, 128 + 96, 100000)
        struct.pack_into('<I', blob, 128 + 32, 0)
        check = 0
        for (value,) in struct.iter_unpack('<I', blob[128:]):
            check ^= value
        struct.pack_into('<I', blob, 128 + 32, check)
        control = decode.parse(blob)['records'][0]['control']
        self.assertFalse(control['params_complete'])
        self.assertEqual(control['params_bytes_observed'], 40)
        self.assertEqual(control['params_bytes_declared'], 100000)

    def test_every_truncation_refused(self):
        blob = header() + record()
        for at in range(len(blob)):
            if at == 64:
                continue  # empty observation file is valid, never a successful attachment
            with self.subTest(at=at), self.assertRaises(decode.InvalidTrace):
                decode.parse(blob[:at])

    def test_corruption_refused(self):
        blob = bytearray(header() + record())
        blob[-1] ^= 1
        with self.assertRaisesRegex(decode.InvalidTrace, 'checksum'):
            decode.parse(blob)

    def test_unrecognized_version_refused(self):
        blob = bytearray(header() + record())
        struct.pack_into('<I', blob, 64 + 56, 0x04000000)
        with self.assertRaises(decode.InvalidTrace):
            decode.parse(blob)


if __name__ == '__main__':
    unittest.main()
