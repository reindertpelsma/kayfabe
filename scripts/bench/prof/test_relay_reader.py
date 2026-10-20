# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Live private-ring observations must survive before any summary exists."""
import importlib.util
from pathlib import Path
import struct,tempfile,unittest
sp=importlib.util.spec_from_file_location('reader',Path(__file__).with_name('reader.py'));r=importlib.util.module_from_spec(sp);sp.loader.exec_module(r)
BASE=12587008
class RelayReaderTests(unittest.TestCase):
    def profile(self,values):
        b=bytearray(r.FILE_BYTES);b[:8]=b'KFPROF01';struct.pack_into('<IIQ',b,8,1,0x01020304,r.FILE_BYTES)
        struct.pack_into('<32Q',b,BASE,*values);return b
    def read(self,b):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'prof.mmap';p.write_bytes(b);return r.read_mmap(p)
    def values(self):
        v=[0]*32;v[0]=2;v[1]=1;v[2]=7;v[3]=0x123;v[5]=100;v[6]=7;v[7]=511;v[8]=3;v[9]=19;v[10]=1;v[11]=22;v[16]=80;v[24]=1;v[25]=512;v[26]=100;v[28]=70;return v
    def test_private_ring_is_read_without_an_aggregator_record(self):
        data=self.read(self.profile(self.values()));self.assertEqual(data['records'],[])
        q=data['relays'][0];self.assertEqual((q['private_get'],q['private_put'],q['reference'],q['private_depth']),(511,3,19,4));self.assertEqual(q['wait'],{'kind':1,'target':22,'birth_ns':70})
    def test_private_depth_handles_a_different_capacity_and_wrap(self):
        v=self.values();v[7]=7;v[8]=1;v[25]=8;self.assertEqual(self.read(self.profile(v))['relays'][0]['private_depth'],2)
    def test_unknown_private_words_do_not_become_zero_progress(self):
        v=self.values();v[6]=0;q=self.read(self.profile(v))['relays'][0]
        self.assertIsNone(q['private_get']);self.assertIsNone(q['private_put']);self.assertIsNone(q['reference']);self.assertIsNone(q['private_depth'])
    def test_missing_put_does_not_invent_zero_depth(self):
        v=self.values();v[6]=1;q=self.read(self.profile(v))['relays'][0]
        self.assertIsNone(q['private_put']);self.assertIsNone(q['private_depth'])
    def test_put_beyond_capacity_is_not_a_wrapped_backlog(self):
        v=self.values();v[8]=2**64-1;q=self.read(self.profile(v))['relays'][0];self.assertIsNone(q['private_depth'])
    def test_odd_relay_record_is_not_a_completed_observation(self):
        v=self.values();v[0]=3;self.assertEqual(self.read(self.profile(v))['relays'],[])
    def test_odd_relay_record_with_valid_hardware_values_is_still_rejected(self):
        v=self.values();v[0]=1;v[7]=12;self.assertFalse(self.read(self.profile(v))['relays'])
    def test_invalid_private_indices_cannot_wrap_into_a_valid_depth(self):
        for get,put in [(512,1),(0,512),(2**64-1,0)]:
            v=self.values();v[7]=get;v[8]=put;self.assertIsNone(self.read(self.profile(v))['relays'][0]['private_depth'])
    def test_declared_source_count_cannot_expand_reader_work(self):
        b=self.profile(self.values());struct.pack_into('<Q',b,200,2**64-1)
        self.assertEqual(len(self.read(b)['relays']),1)
    def test_guest_depth_and_change_time_are_separate_from_private_ring(self):
        v=self.values();v[6]|=24;v[17]=7;v[18]=1;v[19]=8;v[31]=60
        q=self.read(self.profile(v))['relays'][0];self.assertEqual(q['guest_depth'],2);self.assertEqual(q['guest_get_changed_ns'],60);self.assertEqual(q['private_depth'],4)
    def test_guest_depth_wrap_uses_guest_capacity_even_when_private_capacity_differs(self):
        v=self.values();v[6]|=24;v[17]=511;v[18]=3;v[19]=512;v[25]=16
        self.assertEqual(self.read(self.profile(v))['relays'][0]['guest_depth'],4)
    def test_missing_guest_get_never_invents_depth_or_change_age(self):
        v=self.values();v[6]|=16;v[18]=1;v[19]=8
        q=self.read(self.profile(v))['relays'][0];self.assertIsNone(q['guest_depth']);self.assertIsNone(q['guest_get_changed_ns'])
if __name__=='__main__':unittest.main()
