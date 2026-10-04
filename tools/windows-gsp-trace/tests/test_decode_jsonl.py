# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from test_decode import decode, header, record


def rows():
    blob = header() + record() + record(1)
    h = decode.FILE.unpack_from(blob)
    result = [dict(schema='kayfabe-gsp-text/1', kind='header', capture_complete=False,
                   **dict(zip(('magic','version','header_bytes','record_header_bytes','qpc_frequency','started_qpc','flags','reserved'),h[:8])), reserved2=list(h[8:]))]
    at = 64
    while at < len(blob):
        r = decode.RECORD.unpack_from(blob, at)
        result.append(dict(kind='record', **dict(zip(('magic','header_bytes','payload_bytes','direction','qpc','table_pa','queue_sequence','rpc_sequence','rpc_function','rpc_result','flags','missing_before','rpc_version','reserved'),r)), payload_hex=blob[at+64:at+64+r[2]].hex()))
        at += 64 + r[2]
    result.append(dict(kind='footer', records=2, source_bytes=len(blob), source_sha256=hashlib.sha256(blob).hexdigest(), file_export_complete=True, capture_complete=False, driver_stats={'recorded':2}))
    return result


class TextDecoderTests(unittest.TestCase):
    def parse(self, values, max_bytes=64*1024*1024):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'capture.jsonl'
            path.write_text(''.join(json.dumps(row)+'\n' for row in values))
            return decode.parse_jsonl(path, max_bytes)

    def test_lossless_pair_and_stats(self):
        trace=self.parse(rows())
        self.assertEqual(trace['records'],decode.parse(header()+record()+record(1))['records'])
        self.assertEqual(trace['export_stats'], {'recorded':2})
        self.assertTrue(decode.summarize(trace)['unambiguous_query_pairs'][0]['successful'])
        self.assertFalse(trace['complete'])

    def test_footer_hash_and_truncation(self):
        values=rows()
        for damaged in (values[:-1],values+[values[-1]]):
            with self.assertRaises(decode.InvalidTrace): self.parse(damaged)
        values[-1]['source_sha256']='0'*64
        with self.assertRaisesRegex(decode.InvalidTrace,'footer/hash'): self.parse(values)

    def test_limits_and_noncanonical_payload(self):
        with self.assertRaisesRegex(decode.InvalidTrace,'byte limit'): self.parse(rows(), 200)
        values=rows(); values[1]['payload_hex']='zz'+values[1]['payload_hex'][2:]
        with self.assertRaisesRegex(decode.InvalidTrace,'payload hex'): self.parse(values)
        values=rows(); values[1]['payload_hex']=' '*140001
        with self.assertRaisesRegex(decode.InvalidTrace,'140000'): self.parse(values)

    def test_valid_export_hash_does_not_bypass_rpc_validation(self):
        values=rows(); payload=bytearray.fromhex(values[1]['payload_hex']);payload[-1]^=1
        values[1]['payload_hex']=payload.hex()
        damaged=header()+record()[:64]+payload+record(1)
        values[-1]['source_sha256']=hashlib.sha256(damaged).hexdigest()
        with self.assertRaisesRegex(decode.InvalidTrace,'checksum'): self.parse(values)


if __name__ == '__main__': unittest.main()
