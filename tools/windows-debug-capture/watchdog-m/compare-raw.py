#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Bounded private raw-dump comparison for the validated L/M 580.88 profile.

No universal Windows dump-layout claim. Discover NVCD and the module name afresh;
require unique structure/PE matches and exact L agreement with previous KD output.
Emit only hashes, file offsets, scalar status, and module-relative assertion RVAs.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import struct

DRIVER_SHA='31c79cce80b573e21647ace8d1a2b65697f992e7f86196f89e30af477b1bd47c'
DECODER_SHA='a9c7b3f5abea4f27fceeb2b48574d4c17e8e9bb397127acade48d65ae50a10a7'
SCHEMA_SHA='47aae92da4e4872693b399f75ceff2c66ff01b0a519d8532c07eee61f609fc42'

def read(path, limit, digest=None):
    with path.open('rb') as source: data=source.read(limit+1)
    if len(data)>limit: raise ValueError('input exceeds bound')
    if digest and hashlib.sha256(data).hexdigest()!=digest: raise ValueError('pinned source/input hash mismatch')
    return data

def hits(data, pattern):
    result=[];start=0
    while (start:=data.find(pattern,start))>=0:
        result.append(start);start+=len(pattern)
        if len(result)>1024: raise ValueError('candidate count exceeds bound')
    return result

def pe_profile(driver):
    if driver[:2]!=b'MZ': raise ValueError('not PE')
    pe=struct.unpack_from('<I',driver,60)[0];opt=pe+24
    if driver[pe:pe+4]!=b'PE\0\0' or struct.unpack_from('<H',driver,opt)[0]!=0x20b: raise ValueError('not PE64')
    return dict(entry=struct.unpack_from('<I',driver,opt+16)[0],
                size=struct.unpack_from('<I',driver,opt+56)[0],
                checksum=struct.unpack_from('<I',driver,opt+64)[0],
                timestamp=struct.unpack_from('<I',driver,pe+8)[0])

def module(data, pe):
    # Bounded recognizer for these PAGE/DU64 triage dumps: name reference followed
    # by the saved loader entry. Offsets are checked against L's independent KD
    # range and four pinned retail PE identity fields; never used in product code.
    name='nvlddmkm.sys\0'.encode('utf-16le')
    names=hits(data,name)
    if len(names)!=1 or names[0]<4: raise ValueError('not one exact module name')
    name_at=names[0];name_offset=name_at-4
    if struct.unpack_from('<I',data,name_offset)[0]!=len(name)//2-1: raise ValueError('invalid counted module name')
    candidates=[]
    for at in hits(data,struct.pack('<I',name_offset)):
        if at%8 or at+0x90>len(data) or struct.unpack_from('<I',data,at+4)[0]: continue
        base,entry=struct.unpack_from('<QQ',data,at+0x38)
        size=struct.unpack_from('<I',data,at+0x48)[0]
        name_len,name_max=struct.unpack_from('<HH',data,at+0x60)
        checksum=struct.unpack_from('<I',data,at+0x80)[0]
        timestamp=struct.unpack_from('<I',data,at+0x88)[0]
        if (base & 0xffff or base>>48!=0xffff or base>2**64-1-pe['size'] or
            entry-base!=pe['entry'] or size!=pe['size'] or checksum!=pe['checksum'] or
            timestamp!=pe['timestamp'] or name_len!=len(name)-2 or name_max!=len(name)-2): continue
        candidates.append((base,base+size,at,name_at))
    if len(candidates)!=1: raise ValueError('not one structurally and PE-matched module entry')
    return candidates[0]

def decode(data, schema, decoder):
    if data[:8]!=b'PAGEDU64' or len(data)<8192: raise ValueError('not supported PAGE/DU64 dump')
    offsets=hits(data,struct.pack('<I',schema['constants']['NVCD_SIGNATURE']))
    if len(offsets)!=1: raise ValueError('not one NVCD signature')
    return decoder.decode_nvcd(data,schema,offsets[0])

def assertions(decoded, base, end):
    result=[]
    def rva(value): return hex(value-base) if base<=value<end else 'outside-nvlddmkm'
    def walk(value, depth=0):
        if depth>64: raise ValueError('decoded traversal exceeds depth')
        if isinstance(value,dict):
            for key,child in value.items():
                if key=='journal_assert':
                    for a in child:
                        result.append(dict(hint_rva=[rva(x) for x in a.get('breakpoint_addr_hint',[])],
                            stack_rvas=[rva(x) for x in a.get('call_stack',[])],level=a.get('level',[])))
                        if len(result)>4096: raise ValueError('assertion count bound')
                walk(child,depth+1)
        elif isinstance(value,list):
            for child in value: walk(child,depth+1)
    walk(decoded);return result

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('decoder','schema','driver','l_dump','l_kd','l_decoded','m_dump'):
        p.add_argument('--'+name.replace('_','-'),type=Path,required=True)
    a=p.parse_args()
    read(a.decoder,1024*1024,DECODER_SHA)
    spec=importlib.util.spec_from_file_location('trusted_nvcd',a.decoder)
    decoder=importlib.util.module_from_spec(spec);spec.loader.exec_module(decoder)
    schema=json.loads(read(a.schema,1024*1024,SCHEMA_SHA))
    pe=pe_profile(read(a.driver,128*1024*1024,DRIVER_SHA))
    dumps=[read(a.l_dump,8*1024*1024),read(a.m_dump,8*1024*1024)]
    entries=[module(data,pe) for data in dumps]
    decoded=[decode(data,schema,decoder) for data in dumps]
    previous=json.loads(read(a.l_decoded,8*1024*1024))
    if decoded[0]['decoded']!=previous['decoded']: raise ValueError('L raw decode differs from independent KD enumtag decode')
    kd=read(a.l_kd,8*1024*1024).decode('utf-8-sig')
    matches=re.findall(r'^([0-9a-fA-F`]{16,17})\s+([0-9a-fA-F`]{16,17})\s+nvlddmkm\s',kd,re.M)
    if len(matches)!=1 or tuple(int(x.replace('`',''),16) for x in matches[0])!=entries[0][:2]: raise ValueError('L module range differs from KD')
    rows=[assertions(d['decoded'],*e[:2]) for d,e in zip(decoded,entries)]
    equal_prefix=0
    for left,right in zip(*rows):
        if left!=right:break
        equal_prefix+=1
    output=dict(profile='Windows580.88-PAGE-DU64-L-M-20261005',driver_sha256=DRIVER_SHA,
                l_raw_matches_kd_journal=True,l_module_range_matches_kd=True,
                identical_assertion_prefix=equal_prefix, differences=[])
    for label,data,entry,journal,records in zip(('L','M'),dumps,entries,decoded,rows):
        # WinDumpHeader64 offsets compiled from pinned QEMU10.2.4 public header;
        # parameter4 is a kernel pointer and deliberately omitted from output.
        code=struct.unpack_from('<I',data,56)[0]
        params=struct.unpack_from('<3Q',data,64)
        output[label]=dict(dump_sha256=hashlib.sha256(data).hexdigest(),dump_bytes=len(data),
            module_record_file_offset=hex(entry[2]),module_name_file_offset=hex(entry[3]),
            pe_identity={k:hex(v) for k,v in pe.items()},bugcheck_code=hex(code),
            bugcheck_parameters_1_to_3=[hex(x) for x in params],nvcd=journal['nvcd'],
            assertions=len(records),trailing_assertion_chain=records[17:])
    for i,(left,right) in enumerate(zip(*rows)):
        if left!=right: output['differences'].append(dict(ordinal=i,L=left,M=right))
    output['limits']=['No independent successful KD analysis for M.',
      'Module recognizer is a checked profile, not a general documented Windows dump reader.',
      'One missing outer NVCD byte prevents whole-NVCD checksum verification.',
      'No live descriptor locals; assertion level is not NV_STATUS.']
    print(json.dumps(output,indent=2))

if __name__=='__main__':main()
