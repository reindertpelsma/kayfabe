# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Profiler reader contracts, committed before its implementation."""
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('prof_reader',Path(__file__).with_name('reader.py'))
reader=importlib.util.module_from_spec(spec)
spec.loader.exec_module(reader)
FILE_BYTES=16777216
RECORD_BASE=13635584

def fixture():
    b=bytearray(FILE_BYTES);b[:8]=b'KFPROF01';struct.pack_into('<IIQ',b,8,1,0x01020304,FILE_BYTES)
    row={'schema':1,'start_ns':100,'end_ns':200,'threads':[], 'ledger':{'oldest':[]},'counters':{'rmcontrol_seen':10,'host_rm_calls':20,'display_new_frames':2,'broker_presented_new_frames':1}}
    body=json.dumps(row,separators=(',',':')).encode();struct.pack_into('<8Q',b,RECORD_BASE,2,100,200,len(body),1,0,0,0);b[RECORD_BASE+64:RECORD_BASE+64+len(body)]=body
    return b,row
class ReaderTests(unittest.TestCase):
    def file(self,b):
        p=Path(self.tmp.name)/'prof.mmap';p.write_bytes(b);return p
    def setUp(self):self.tmp=tempfile.TemporaryDirectory()
    def tearDown(self):self.tmp.cleanup()
    def test_golden_record_and_explicit_frame_denominators(self):
        b,row=fixture();rows=reader.read_mmap(self.file(b))['records'];self.assertEqual(rows,[row])
        self.assertEqual(reader.ratios(row)['rmcontrol_per_display_new'],5)
        self.assertEqual(reader.ratios(row)['rmcontrol_per_broker_new'],10)
        self.assertEqual(reader.ratios(row)['host_rm_per_display_new'],10)
    def test_odd_record_is_not_a_completed_observation(self):
        b,_=fixture();struct.pack_into('<Q',b,RECORD_BASE,3);self.assertEqual(reader.read_mmap(self.file(b))['records'],[])
    def test_odd_record_with_valid_payload_still_is_not_accepted(self):
        b,_=fixture();struct.pack_into('<Q',b,RECORD_BASE,1);self.assertFalse(reader.read_mmap(self.file(b))['records'])
    def test_header_cannot_expand_file_or_point_past_actual_mapping(self):
        for n in [0,4096,12587007,2**64-1]:
            b,_=fixture();struct.pack_into('<Q',b,16,n)
            with self.assertRaises(reader.InvalidProfile):reader.read_mmap(self.file(b))
    def test_short_actual_file_is_rejected_before_record_access(self):
        b,_=fixture()
        for n in [0,24,4096,RECORD_BASE+65]:
            with self.assertRaises(reader.InvalidProfile):reader.read_mmap(self.file(b[:n]))
    def test_wrong_version_and_hostile_magic_are_rejected(self):
        for off in [0,8,12]:
            b,_=fixture();b[off]^=255
            with self.assertRaises(reader.InvalidProfile):reader.read_mmap(self.file(b))
    def test_payload_length_does_not_authorize_arbitrary_reads(self):
        for n in [65473,2**64-1]:
            b,_=fixture();struct.pack_into('<Q',b,RECORD_BASE+24,n)
            self.assertEqual(reader.read_mmap(self.file(b))['records'],[])
    def test_hostile_payload_is_bounded_and_not_decoded_recursively(self):
        b,_=fixture();body=b'['*10000+b'0'+b']'*10000;struct.pack_into('<Q',b,RECORD_BASE+24,len(body));b[RECORD_BASE+64:RECORD_BASE+64+len(body)]=body
        self.assertEqual(reader.read_mmap(self.file(b))['records'],[])
    def test_zero_denominator_is_missing_instead_of_infinity(self):
        _,r=fixture();r['counters']['display_new_frames']=0;self.assertIsNone(reader.ratios(r)['rmcontrol_per_display_new'])
    def test_missing_frame_or_call_source_is_unknown(self):
        self.assertIsNone(reader.ratios({'counters':{}})['rmcontrol_per_display_new'])
    def test_jsonl_truncated_tail_preserves_complete_records(self):
        _,r=fixture();p=Path(self.tmp.name)/'prof.jsonl';p.write_text(json.dumps(r)+'\n{"schema":')
        self.assertEqual(reader.read_jsonl(p),[r])
    def test_jsonl_oversized_line_is_rejected_boundedly(self):
        p=Path(self.tmp.name)/'prof.jsonl';p.write_bytes(b'x'*65537+b'\n')
        with self.assertRaises(reader.InvalidProfile):reader.read_jsonl(p)
    def test_summary_uses_ratio_of_sums_not_mean_of_ratios(self):
        _,a=fixture();_,b=fixture();b['counters']['rmcontrol_seen']=10;b['counters']['display_new_frames']=8
        self.assertEqual(reader.summarize([a,b])['ratios']['rmcontrol_per_display_new'],2)
    def test_unknown_interval_is_not_counted_as_zero_exposure(self):
        _,a=fixture();_,b=fixture();b['counters']['display_new_frames']=None
        s=reader.summarize([a,b]);self.assertEqual(s['ratios']['rmcontrol_per_display_new'],5);self.assertEqual(s['ratio_covered_records'],1)
if __name__=='__main__':unittest.main()
