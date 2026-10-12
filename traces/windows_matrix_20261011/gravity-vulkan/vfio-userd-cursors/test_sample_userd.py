#!/usr/bin/env python3
"""GPU-free checks for reference RAM boundaries and public page-list decoding."""
import importlib.util
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location('sample_userd', Path(__file__).with_name('sample_userd.py'))
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
SIZE = 8*1024**3
TREE = '''0000000000000000-00000000bfffffff (prio 0, ram): alias ram-below-4g @ram0 0000000000000000-00000000bfffffff
0000000100000000-000000023fffffff (prio 0, ram): alias ram-above-4g @ram0 00000000c0000000-00000001ffffffff'''

class Bounds(unittest.TestCase):
    def test_ram_aliases_do_not_resolve_mmio_or_partial_span(self):
        ranges = m.aliases(TREE, SIZE)
        self.assertEqual(m.resolve(ranges, 0x100000000, 4096, SIZE), 0xc0000000)
        self.assertEqual(m.resolve(ranges, 0x23ffff000, 4096, SIZE), SIZE-4096)
        for address,size in [(0xc0000000,4096),(0xfffff000,8192),(0x23ffff000,8192),(2**64-8,16)]:
            with self.assertRaises(ValueError): m.resolve(ranges,address,size,SIZE)
        with self.assertRaises(ValueError): m.aliases(TREE.replace('@ram0','@vfio-bar'),SIZE)
        with self.assertRaises(ValueError): m.aliases(TREE.replace('00000001ffffffff','0000000200000000'),SIZE)

    def test_only_public_one_page_system_descriptor_is_accepted(self):
        payload = bytearray(144)
        struct.pack_into('<7I',payload,80,1,2,3,0x81,0x40000000,0,0)
        struct.pack_into('<Q',payload,112,4096)
        struct.pack_into('<I',payload,120,1)
        struct.pack_into('<I',payload,128,1<<16)
        struct.pack_into('<Q',payload,136,0x23974a)
        def request(): return dict(rpc_function=4,direction='request',payload_hex=payload.hex())
        self.assertEqual(m.memory_request(request())['gpa'],0x23974a000)
        for offset,value in [(92,0x82),(100,4),(120,2),(128,(1<<16)|1)]:
            old = bytes(payload[offset:offset+4]); struct.pack_into('<I',payload,offset,value)
            self.assertIsNone(m.memory_request(request())); payload[offset:offset+4]=old
        struct.pack_into('<Q',payload,136,1<<52)
        with self.assertRaises(ValueError): m.memory_request(request())

    def test_only_fixed_userd_fields_are_retained(self):
        page=bytearray(b'\xaa'*4096)
        struct.pack_into('<I',page,0x600+0x48,7)
        struct.pack_into('<2I',page,0x600+0x88,8,9)
        fields=m.fields(page)
        self.assertEqual(fields[3],dict(slot=0x600,reference=7,get=8,put=9))
        self.assertEqual(set(fields[3]),{'slot','reference','get','put'})
        with self.assertRaises(ValueError): m.fields(page[:-1])

if __name__=='__main__': unittest.main()
