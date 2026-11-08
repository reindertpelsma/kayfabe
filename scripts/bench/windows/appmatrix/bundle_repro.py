# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Level 1: exact payload inventory; level 2: diagnostic container equality.

Object integrity is checked against an image's OWN receipt, never against the
other build's container digest. No vendor input can opt out of byte comparison.
"""
import copy
import hashlib
import json
from pathlib import Path
import re

PROGRAMS=frozenset(('cup2.exe','cup3.exe','cup8.exe','kf_cuda_control.exe',
                    'kf_dxprobe.exe','kf_glgears.exe','kf_overlayprobe.exe'))

def canonical(data):
    return json.dumps(data,sort_keys=True,separators=(',',':'),ensure_ascii=True).encode('ascii')

def policy(path,toolchain):
    programs=toolchain.get('programs',{})
    if not isinstance(programs,dict) or set(programs)-PROGRAMS:raise ValueError('unknown reproducibility program')
    name=path.removeprefix('tools/')
    result=copy.deepcopy(programs.get(name,dict(mode='sha256'))) if path=='tools/'+name and name in PROGRAMS else dict(mode='sha256')
    if result.get('mode') not in ('sha256','inventory-only'):raise ValueError('invalid program comparison mode')
    if result['mode']=='inventory-only':
        if not isinstance(result.get('reason'),str) or not result['reason'].strip() or not isinstance(result.get('functional_checks'),list) or not result['functional_checks'] or not all(isinstance(x,str) and re.fullmatch(r'[a-z0-9_]+',x)for x in result['functional_checks']):
            raise ValueError('inventory-only needs reason and named functional checks')
    return result

def inventory(receipt):
    files=receipt['files']
    if not isinstance(files,dict) or not files:raise ValueError('empty reproduction inventory')
    out={}
    for path,record in sorted(files.items()):
        if not re.fullmatch(r'[0-9a-f]{64}',record.get('sha256','')) or type(record.get('size'))is not int or record['size']<0:
            raise ValueError('invalid per-file reproduction pin')
        expected=policy(path,receipt.get('toolchain',{}))
        actual=record.get('reproducibility',dict(mode='sha256'))
        if actual!=expected:raise ValueError('per-file comparison differs from pinned program policy: '+path)
        if actual['mode']=='inventory-only':
            out[path]=dict(reproducibility=actual)
        else:out[path]=dict(sha256=record['sha256'],size=record['size'],reproducibility=actual)
    return out

def inventory_hash(receipt):return hashlib.sha256(canonical(inventory(receipt))).hexdigest()

def metadata_diff(expected,actual,prefix=''):
    if isinstance(expected,dict)and isinstance(actual,dict):
        rows=[]
        for key in sorted(set(expected)|set(actual)):
            rows.extend(metadata_diff(expected.get(key),actual.get(key),prefix+'.'+key if prefix else key))
        return rows
    if isinstance(expected,list)and isinstance(actual,list):
        rows=[]
        for i in range(max(len(expected),len(actual))):
            rows.extend(metadata_diff(expected[i]if i<len(expected)else None,actual[i]if i<len(actual)else None,prefix+'['+str(i)+']'))
        return rows
    if expected==actual:return []
    return [dict(field=prefix,expected=expected,actual=actual)]

def compare(expected,actual):
    first,second=inventory(expected),inventory(actual)
    added=sorted(set(second)-set(first));removed=sorted(set(first)-set(second))
    changed=[p for p in sorted(set(first)&set(second))if first[p]!=second[p]]
    different=bool(added or removed or changed)
    container_equal=expected['image']['sha256']==actual['image']['sha256'] and expected['image']['size']==actual['image']['size']
    diff=metadata_diff(expected.get('container_metadata',{}),actual.get('container_metadata',{}))
    if not container_equal and not diff:diff=[dict(field='container.sha256',expected=expected['image']['sha256'],actual=actual['image']['sha256'])]
    return dict(level1='different'if different else'equal',level2='equal'if container_equal else'different',
                added=added,removed=removed,changed=changed,container_diff=diff,
                inventory_only=[p for p,v in second.items()if v['reproducibility']['mode']=='inventory-only'])

def read_candidate(iso,receipt):
    """Verify stored-object integrity against its own receipt, then read payload."""
    import bundle_iso
    path=Path(iso);h=hashlib.sha256()
    with path.open('rb')as stream:
        while block:=stream.read(1024**2):h.update(block)
    if path.stat().st_size!=receipt['image']['size'] or h.hexdigest()!=receipt['image']['sha256']:
        raise ValueError('INTEGRITY_FAIL image differs from its own receipt')
    actual=copy.deepcopy(receipt)
    with bundle_iso.Reader(path,allow_any_label=True)as image:
        records=image.records()
        if set(records)!=set(receipt['files']):raise ValueError('LEVEL1_FAIL candidate file list differs from its own manifest')
        for name,pin in records.items():
            if any(pin[k]!=receipt['files'][name][k]for k in ('sha256','size')):
                raise ValueError('LEVEL1_FAIL candidate file differs from its own manifest: '+name)
        actual['container_metadata']=image.metadata()
    return actual

def report(result):
    print('REPRODUCTION_LEVEL1 '+result['level1'].upper())
    if result['level1']!='equal':print('LEVEL1_DIFF '+json.dumps({k:result[k]for k in ('added','removed','changed')},sort_keys=True))
    if result['level2']!='equal':print('WARNING LEVEL2_CONTAINER_DIFF '+json.dumps(result['container_diff'],sort_keys=True))
    if result['inventory_only']:print('INVENTORY_ONLY functional checks required: '+','.join(result['inventory_only']))
