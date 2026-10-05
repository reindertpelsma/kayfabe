#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Labelled incremental raw-journal comparisons retaining the independent L control.

The private manifest lists 2..4 ordered {label, revision, dump} cases. All use the
same checked Windows580.88 profile. No target access, driver execution or dump writes.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import struct

RAW_SHA='10e794b09ea0def90482033cffb8d2688c0b251d0482055c86336ae7ba06381f'

def load(path,name,sha):
    with path.open('rb') as f:data=f.read(1024*1024+1)
    if len(data)>1024*1024 or hashlib.sha256(data).hexdigest()!=sha:
        raise ValueError('pinned dependency mismatch: '+name)
    spec=importlib.util.spec_from_file_location(name,path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

def delta(previous,current):
    prefix=0
    for a,b in zip(previous,current):
        if a!=b:break
        prefix+=1
    suffix=0
    while suffix<min(len(previous),len(current))-prefix and previous[-suffix-1]==current[-suffix-1]:suffix+=1
    return dict(previous_assertions=len(previous),current_assertions=len(current),
                identical_prefix=prefix,identical_suffix_after_prefix=suffix,
                previous_changed=previous[prefix:len(previous)-suffix if suffix else None],
                current_changed=current[prefix:len(current)-suffix if suffix else None])

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('manifest','decoder','schema','driver','reference_kd','reference_decoded'):
        p.add_argument('--'+name.replace('_','-'),type=Path,required=True)
    a=p.parse_args();raw=load(Path(__file__).with_name('compare-raw.py'),'raw_profile',RAW_SHA)
    cases=json.loads(raw.read(a.manifest,65536))
    if not isinstance(cases,list) or not 2<=len(cases)<=4:raise ValueError('expected 2..4 cases')
    labels=set()
    for row in cases:
        if not isinstance(row,dict) or set(row)!={'label','revision','dump'}:raise ValueError('manifest fields')
        label=row['label']
        if not isinstance(label,str) or not re.fullmatch(r'[A-Za-z][A-Za-z0-9_-]{0,31}',label) or label in labels:raise ValueError('invalid/duplicate label')
        labels.add(label)
        if not isinstance(row['revision'],str) or not re.fullmatch(r'[0-9a-f]{8,40}',row['revision']):raise ValueError('invalid revision')
        if not isinstance(row['dump'],str):raise ValueError('invalid private dump path')
    decoder=load(a.decoder,'nvcd_decoder',raw.DECODER_SHA)
    schema=json.loads(raw.read(a.schema,1024*1024,raw.SCHEMA_SHA))
    pe=raw.pe_profile(raw.read(a.driver,128*1024*1024,raw.DRIVER_SHA))
    rows=[];summaries=[]
    for i,case in enumerate(cases):
        data=raw.read(Path(case['dump']),8*1024*1024)
        entry=raw.module(data,pe);journal=raw.decode(data,schema,decoder)
        if i==0:
            independent=json.loads(raw.read(a.reference_decoded,8*1024*1024))
            if journal['decoded']!=independent['decoded']:raise ValueError('reference raw journal disagrees with independent KD decode')
            kd=raw.read(a.reference_kd,8*1024*1024).decode('utf-8-sig')
            matches=re.findall(r'^([0-9a-fA-F`]{16,17})\s+([0-9a-fA-F`]{16,17})\s+nvlddmkm\s',kd,re.M)
            if len(matches)!=1 or tuple(int(x.replace('`',''),16) for x in matches[0])!=entry[:2]:raise ValueError('reference module range disagrees with KD')
        records=raw.assertions(journal['decoded'],*entry[:2]);rows.append(records)
        summaries.append(dict(label=case['label'],caller_supplied_revision=case['revision'],
            dump_bytes=len(data),dump_sha256=hashlib.sha256(data).hexdigest(),
            nvcd=journal['nvcd'],assertion_count=len(records),
            module_record_file_offset=hex(entry[2]),module_name_file_offset=hex(entry[3]),
            pe_identity={k:hex(v) for k,v in pe.items()},
            bugcheck_code=hex(struct.unpack_from('<I',data,56)[0]),
            bugcheck_parameters_1_to_3=[hex(x) for x in struct.unpack_from('<3Q',data,64)]))
    comparisons=[]
    for i in range(1,len(cases)):
        comparisons.append(dict(previous=cases[i-1]['label'],current=cases[i]['label'],**delta(rows[i-1],rows[i])))
    print(json.dumps(dict(schema='kayfabe-labelled-watchdog-series/1',driver_sha256=raw.DRIVER_SHA,
        reference_label=cases[0]['label'],reference_raw_matches_independent_kd=True,
        cases=summaries,incremental_comparisons=comparisons,
        limits=['Saved complete protobuf records only; inspect each outer NVCD integrity result.',
                'No live descriptor locals; no assertion-level interpretation as NV_STATUS.',
                'Labels/revisions supplied by caller; retain separate run metadata.',
                'Module recognition is a checked580.88 triage profile, not a general Windows ABI.']),indent=2))

if __name__=='__main__':main()
