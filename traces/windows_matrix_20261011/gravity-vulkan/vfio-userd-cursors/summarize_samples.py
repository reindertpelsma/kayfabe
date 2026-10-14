#!/usr/bin/env python3
"""Validate sample footer/hash and report narrow registration/cursor windows."""
import argparse
import collections
import hashlib
import json
from pathlib import Path


def summarize(path, allow_incomplete=False):
    digest=hashlib.sha256(); samples=[]; regs=[]; memories=[]; footer=None; counts=collections.Counter()
    with path.open() as stream:
        for seq,line in enumerate(stream):
            if len(line)>8192 or seq>100000 or footer is not None:
                raise ValueError('sample line/count/framing bound')
            row=json.loads(line)
            if row['kind']=='footer':
                if row.get('file_export_complete') is not True or row.get('records')!=seq or row.get('sha256')!=digest.hexdigest():
                    raise ValueError('sample footer/hash mismatch')
                footer=row; continue
            if row['sequence']!=seq or row.get('complete',False) is not False:
                raise ValueError('sample sequence/completeness mismatch')
            digest.update(line.encode()); counts[row['kind']]+=1
            if seq==0 and (row['kind']!='header' or row['schema']!='kayfabe-native-userd-samples/1'):
                raise ValueError('unsupported sample schema')
            if row['kind']=='registration': regs.append(row)
            if row['kind']=='memory-candidate': memories.append(row)
            if row['kind']=='sample': samples.append(row)
    if footer is None:
        if not allow_incomplete: raise ValueError('missing sample footer')
        footer=dict(file_export_complete=False, records=sum(counts.values()), independently_computed_sha256=digest.hexdigest(), reason='missing-collector-footer')
    summaries=[]
    for reg in regs:
        candidates=[m for m in memories if m.get('request_index',m['record_index'])<reg['record_index'] and m['generation']==reg['generation'] and m['client']==reg['client'] and m['handle']==reg['userd']]
        if not candidates: raise ValueError('registration has no observed memory candidate')
        memory=candidates[-1]
        def same_candidate(c):
            return all(c[k] == memory[k] for k in ['generation','client','handle','allocation_serial'])
        page=[s for s in samples if s['offset']==memory['offset'] and any(same_candidate(c) for c in s['candidates'])]
        target=reg['slot']; observed=reg['observed_ns']
        def value(s):
            first=next(f for f in s['fields'] if f['slot']==target)
            second=next(f for f in s['second'] if f['slot']==target)
            return dict(sequence=s['sequence'],relative_us=(s['sampled_ns']-observed)/1000,
                        target=first,second=second,target_stable=first==second)
        before=[s for s in page if s['sampled_ns']<observed]
        after=[s for s in page if s['sampled_ns']>=observed]
        window=[value(s) for s in page if abs(s['sampled_ns']-observed)<=10_000_000]
        later=[value(s) for s in after if s['fields'][target//512]!=s['second'][target//512] or s['fields'][target//512]['get'] or s['fields'][target//512]['put']]
        summaries.append(dict(registration=reg,memory_candidate=memory,
            last_before=value(before[-1]) if before else None,
            first_after=value(after[0]) if after else None,
            first_nonzero_after=later[0] if later else None,window_10ms=window,
            note='Native sampled fields only; live writer delay, non-atomic CPU samples and no executed-trigger proof.'))
    return dict(schema='kayfabe-native-userd-cursor-summary/1',complete=False,
        counts=dict(counts),footer=footer,registrations=summaries,
        warning='Observed initial cursor values are not a universal migration rule or ownership/lifetime proof.')

if __name__=='__main__':
    ap=argparse.ArgumentParser(description=__doc__); ap.add_argument('samples',type=Path)
    ap.add_argument('--allow-incomplete',action='store_true',help='explicit partial evidence only; never synthesize a collector footer')
    args=ap.parse_args(); print(json.dumps(summarize(args.samples,args.allow_incomplete),indent=2))
