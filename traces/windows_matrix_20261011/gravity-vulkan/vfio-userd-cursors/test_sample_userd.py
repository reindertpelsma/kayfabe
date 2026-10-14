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


class Lifetimes(unittest.TestCase):
    def request(self):
        b=bytearray(144)
        struct.pack_into('<7I',b,80,1,2,3,0x81,0x40000000,0,0)
        struct.pack_into('<Q',b,112,4096)
        struct.pack_into('<I',b,120,1)
        struct.pack_into('<I',b,128,1<<16)
        struct.pack_into('<Q',b,136,0x100000)
        return dict(rpc_function=4,direction='request',rpc_sequence=0,rpc_status='0x0',payload_hex=b.hex())
    def reply(self,status='0x0'):
        return dict(rpc_function=4,direction='reply',rpc_sequence=0,rpc_status=status,payload_hex='')
    def registry(self):
        c=m.Candidates();c.boundary(1,False);return c
    def free(self,obj):
        b=bytearray(96);struct.pack_into('<4I',b,80,1,0,obj,0)
        return dict(rpc_function=10,direction='request',rpc_sequence=0,payload_hex=b.hex())
    def test_success_required_and_ambiguous_empty_replies_never_admit(self):
        c=self.registry();c.observe(self.request(),1);self.assertFalse(c.live)
        c.observe(self.reply('0x56'),1);self.assertFalse(c.live)
        c.observe(self.request(),1);c.observe(self.request(),1)
        c.observe(self.reply(),1);c.observe(self.reply(),1);self.assertFalse(c.live)
        c.observe(self.request(),1);c.observe(self.reply(),1);self.assertEqual(len(c.live),1)
    def test_client_device_and_object_free_invalidate_children_and_pending(self):
        for obj in [1,2,3,0]:
            c=self.registry();c.observe(self.request(),1);c.observe(self.reply(),1)
            c.observe(self.free(obj),1);self.assertFalse(c.live)
            c.observe(self.request(),1);c.observe(self.free(obj),1)
            c.observe(self.reply(),1);self.assertFalse(c.live)
    def test_gap_generation_and_handle_reuse_do_not_preserve_identity(self):
        c=self.registry();c.observe(self.request(),1)
        with self.assertRaises(ValueError): c.boundary(1,True)
        c.observe(self.reply(),1);self.assertFalse(c.live)
        c.observe(self.request(),1);c.observe(self.reply(),1);old=next(iter(c.live.values()))['allocation_serial']
        c.observe(self.free(3),1);c.observe(self.request(),1);c.observe(self.reply(),1)
        self.assertGreater(next(iter(c.live.values()))['allocation_serial'],old)
        c.boundary(2,False);self.assertFalse(c.live)
        with self.assertRaises(ValueError):c.boundary(1,False)

if __name__=='__main__': unittest.main()
