#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded offline merge. A diag measurement is input, never presumed available."""
import csv,json
from pathlib import Path
import reader
MAX_NS=2**64
FIELDS=('qpc','guest_boot_id','event_id','pid','tid')
def uint(v):return type(v) is int and 0<=v<MAX_NS
def calibrate_ping_pong(samples,*,valid_from_ns,valid_until_ns,drift_ppm):
    if not isinstance(samples,list) or not 1<=len(samples)<=64:raise ValueError('need 1..64 diag samples')
    if not all(uint(v) for v in [valid_from_ns,valid_until_ns,drift_ppm]) or valid_until_ns<valid_from_ns or valid_until_ns-valid_from_ns>60_000_000_000 or drift_ppm>10000:raise ValueError('invalid calibration validity/drift')
    keys=('host_send_ns','host_recv_ns','guest_qpc','frequency','guest_boot_id')
    clean=[]
    for sample in samples:
        if not isinstance(sample,dict) or not all(uint(sample.get(k)) for k in keys):raise ValueError('invalid diag sample')
        row={k:sample[k] for k in keys}
        if row['host_recv_ns']<row['host_send_ns'] or row['host_recv_ns']-row['host_send_ns']>60_000_000_000 or not 0<row['frequency']<=10**12:raise ValueError('invalid diag bracket/frequency')
        if clean and (row['guest_boot_id']!=clean[0]['guest_boot_id'] or row['frequency']!=clean[0]['frequency']):raise ValueError('mixed guest clock lifetimes')
        clean.append(row)
    selected=min(clean,key=lambda r:r['host_recv_ns']-r['host_send_ns'])
    mid=(selected['host_send_ns']+selected['host_recv_ns'])//2
    if not valid_from_ns<=mid<=valid_until_ns:raise ValueError('anchor outside validity')
    return dict(method='diag_ping_pong',host_ns0=mid,qpc0=selected['guest_qpc'],frequency=selected['frequency'],guest_boot_id=selected['guest_boot_id'],error_ns=(selected['host_recv_ns']-selected['host_send_ns']+1)//2,valid_from_ns=valid_from_ns,valid_until_ns=valid_until_ns,drift_ppm=drift_ppm,sample_count=len(clean),samples=clean)
def normalize_calibration(c):
    if not isinstance(c,dict) or c.get('method')!='diag_ping_pong':raise ValueError('unsupported calibration method')
    fresh=calibrate_ping_pong(c.get('samples'),valid_from_ns=c.get('valid_from_ns'),valid_until_ns=c.get('valid_until_ns'),drift_ppm=c.get('drift_ppm'))
    if any(c.get(k)!=fresh[k] for k in ['host_ns0','qpc0','frequency','guest_boot_id','error_ns']):raise ValueError('calibration disagrees with measured bracket')
    return fresh
def align_guest(row,calibration):
    out={k:row[k] for k in FIELDS if k in row and uint(row[k])}
    if not all(k in out for k in ['qpc','guest_boot_id']):raise ValueError('guest event needs bounded qpc/boot identity')
    out.update(alignment='UNALIGNED',ts_ns=None,clock_error_ns=None)
    if calibration is None:return out
    c=normalize_calibration(calibration)
    if not isinstance(c,dict) or c.get('method')!='diag_ping_pong' or not all(uint(c.get(k)) for k in ['host_ns0','qpc0','frequency','guest_boot_id','error_ns','valid_from_ns','valid_until_ns','drift_ppm']) or not 0<c['frequency']<=10**12 or c['drift_ppm']>10000:raise ValueError('invalid diag calibration')
    delta=(out['qpc']-c['qpc0'])*1_000_000_000//c['frequency'];stamp=c['host_ns0']+delta
    if out['guest_boot_id']!=c['guest_boot_id'] or not c['valid_from_ns']<=stamp<=c['valid_until_ns'] or not uint(stamp):return out
    error=c['error_ns']+(abs(delta)*c['drift_ppm']+999999)//1000000
    if not uint(error):return out
    out.update(alignment='ALIGNED',ts_ns=stamp,clock_error_ns=error,clock_lower_ns=max(0,stamp-error),clock_upper_ns=min(MAX_NS-1,stamp+error))
    return out

def read_guest_csv(path):
    csv.field_size_limit(4096);used=0
    with Path(path).open(encoding='utf-8',newline='') as stream:
        def lines():
            nonlocal used
            while True:
                line=stream.readline(4097)
                if not line:break
                used+=len(line)
                if len(line)>4096 or used>reader.MAX_JSONL_BYTES:raise ValueError('guest CSV input bound')
                yield line
        try:
            out=[]
            for row in csv.DictReader(lines()):
                if len(out)>=100000 or len(row)>32:raise ValueError('guest CSV row bound')
                values={}
                for key in FIELDS:
                    value=row.get(key)
                    if value is not None and value!='':
                        n=int(value)
                        if not uint(n):raise ValueError('guest integer out of bounds')
                        values[key]=n
                if 'qpc' not in values or 'guest_boot_id' not in values:raise ValueError('missing guest clock identity')
                out.append(values)
            return out
        except (csv.Error,UnicodeError) as e:raise ValueError('invalid guest CSV') from e

def merge_records(host,broker,guest,calibration):
    calibration=normalize_calibration(calibration) if calibration is not None else None
    records=[]
    for source,rows in [('kayfabe',host),('broker',broker)]:
        for row in rows:
            checked=reader._json(json.dumps(row,allow_nan=False).encode())
            records.append(dict(checked,source=source))
    records.sort(key=lambda r:r['start_ns'])
    return dict(schema=1,clock='CLOCK_MONOTONIC',cross_source_identity='UNVERIFIED',records=records,guest=[align_guest(row,calibration)for row in guest],calibration=calibration)
def merge_files(host,broker,guest=None,calibration=None):
    c=None
    if calibration:
        p=Path(calibration)
        if p.stat().st_size>65536:raise ValueError('calibration input bound')
        try:c=normalize_calibration(json.loads(p.read_text()))
        except (ValueError,RecursionError) as e:raise ValueError('invalid measured calibration') from e
    return merge_records(reader.load(host)['records'],reader.load(broker)['records'],read_guest_csv(guest)if guest else [],c)
