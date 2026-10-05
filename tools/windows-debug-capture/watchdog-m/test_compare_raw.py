#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Synthetic rejection tests; contains no Windows or NVIDIA bytes."""
import importlib.util
from pathlib import Path
import struct

spec=importlib.util.spec_from_file_location('raw',Path(__file__).with_name('compare-raw.py'))
r=importlib.util.module_from_spec(spec);spec.loader.exec_module(r)
pe=dict(entry=0x200,size=0x4000,checksum=42,timestamp=123)
base=0xffff800000100000

def fixture():
    data=bytearray(16384);data[:8]=b'PAGEDU64'
    text='nvlddmkm.sys\0'.encode('utf-16le');name=0x1004;record=0x200
    struct.pack_into('<I',data,name-4,len(text)//2-1);data[name:name+len(text)]=text
    struct.pack_into('<I',data,record,name-4)
    struct.pack_into('<QQI',data,record+0x38,base,base+pe['entry'],pe['size'])
    struct.pack_into('<HH',data,record+0x60,len(text)-2,len(text)-2)
    struct.pack_into('<I',data,record+0x80,pe['checksum']);struct.pack_into('<I',data,record+0x88,pe['timestamp'])
    return data

def refuses(data):
    try:r.module(data,pe)
    except ValueError:return
    raise AssertionError('bad module identity accepted')

assert r.module(fixture(),pe)==(base,base+pe['size'],0x200,0x1004)
for offset in (0x238,0x240,0x248,0x260,0x280,0x288,0x1000):
    data=fixture();data[offset]^=1;refuses(data)
data=fixture();data[0x400:0x490]=data[0x200:0x290];refuses(data)
data=fixture();data.extend('nvlddmkm.sys\0'.encode('utf-16le'));refuses(data)
refuses(fixture()[:0x280])
class NeverDecode:
    def decode_nvcd(self,*args):raise AssertionError('invalid envelope reached NVCD decoder')
schema={'constants':{'NVCD_SIGNATURE':0x4443564e}}
for data in (b'not-a-dump',fixture(),fixture()+b'NVCD'+b'NVCD'):
    try:r.decode(data,schema,NeverDecode())
    except ValueError:pass
    else:raise AssertionError('invalid dump/NVCD envelope accepted')
print('Raw dump profile bounds, identity, ambiguity and NVCD envelope rejection tests passed')
