#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded read-only kf-prof reader. All times are host CLOCK_MONOTONIC ns."""
import argparse
import json
import math
import os
from pathlib import Path
import struct
import time

FILE_BYTES=16777216
HEADER_BYTES=4096
THREAD_BASE=4096
THREAD_BYTES=196608
RECORD_BASE=13635584
RECORD_BYTES=65536
RECORDS=47
MAX_PAYLOAD=65472
MAX_JSONL_BYTES=64*1024*1024
class InvalidProfile(ValueError):
    """Malformed, unsupported, incomplete or excessively large input."""

def _json(body):
    # Reject excessive nesting before invoking the recursive standard decoder.
    depth=0;quoted=False;escaped=False
    for c in body:
        if quoted:
            if escaped:escaped=False
            elif c==92:escaped=True
            elif c==34:quoted=False
        elif c==34:quoted=True
        elif c in (91,123):
            depth+=1
            if depth>16:raise InvalidProfile('JSON nesting exceeds 16')
        elif c in (93,125):depth-=1
    try:
        row=json.loads(body,parse_constant=lambda _: (_ for _ in ()).throw(InvalidProfile('nonfinite value')))
    except (ValueError,UnicodeError,RecursionError) as e:raise InvalidProfile('invalid JSON') from e
    if not isinstance(row,dict) or row.get('schema')!=1:raise InvalidProfile('unsupported record schema')
    def uint(v):return type(v) is int and 0<=v<2**64
    if not uint(row.get('start_ns')) or not uint(row.get('end_ns')) or row['end_ns']<row['start_ns']:raise InvalidProfile('invalid interval')
    threads=row.get('threads',[]);counters=row.get('counters',{});ledger=row.get('ledger',{})
    if not isinstance(threads,list) or len(threads)>64 or not all(isinstance(t,dict) for t in threads):raise InvalidProfile('invalid thread table')
    if not isinstance(counters,dict) or not all(v is None or uint(v) for v in counters.values()):raise InvalidProfile('invalid counters')
    if not isinstance(ledger,dict) or not isinstance(ledger.get('oldest',[]),list) or len(ledger.get('oldest',[]))>16:raise InvalidProfile('invalid ledger')
    for t in threads:
        if any(k in t and not uint(t[k]) for k in ['slot','tid','state','site','arg','since_ns','dropped','role']):raise InvalidProfile('invalid thread fields')
    for e in ledger.get('oldest',[]):
        if not isinstance(e,dict) or any(k in e and not uint(e[k]) for k in ['id','kind','birth_ns','age_ns','generation','thread']):raise InvalidProfile('invalid obligation')
    return row

def _relay(index,v):
    known=lambda mask,at:v[at] if v[6]&mask else None
    get,put,entries=known(1,7),known(2,8),v[25]
    depth=(put+entries-get)%entries if get is not None and put is not None and entries>0 and get<entries and put<entries else None
    gg,gp,ge=known(8,17),known(16,18),v[19]
    gd=(gp+ge-gg)%ge if gg is not None and gp is not None and ge>0 and gg<ge and gp<ge else None
    return dict(guest_depth=gd,guest_get_changed_ns=v[31] if gg is not None else None,source_slot=index,generation=v[2],key=v[3],token=v[4],observed_ns=v[5],private_get=get,private_put=put,reference=known(4,9),private_depth=depth,wait=dict(kind=v[10],target=v[11],birth_ns=v[28]),doorbell_count=known(32,12),last_doorbell_observed_ns=v[13],wake_reason=v[14],wake_observed_ns=v[15],private_get_changed_ns=v[16],guest_get=known(8,17),guest_put=known(16,18),guest_entries=v[19],guest_observed_ns=v[20],issued=v[21],fetched=v[22],markers_issued=v[23],active=bool(v[24]),private_entries=entries,private_observed_ns=v[26],private_put_observed_ns=v[27],birth_ns=v[29])

def read_mmap(path):
    # pread handles concurrent truncation as a short read, never a SIGBUS.
    fd=os.open(path,os.O_RDONLY)
    try:
        if os.fstat(fd).st_size!=FILE_BYTES:raise InvalidProfile('wrong actual file length')
        h=os.pread(fd,HEADER_BYTES,0)
        if len(h)!=HEADER_BYTES or h[:8]!=b'KFPROF01':raise InvalidProfile('invalid header')
        version,endian,declared=struct.unpack_from('<IIQ',h,8)
        if version!=1 or endian!=0x01020304:raise InvalidProfile('unsupported version/endianness')
        if declared!=FILE_BYTES:raise InvalidProfile('invalid declared geometry')
        thread_count=struct.unpack_from('<Q',h,168)[0]
        if thread_count>64:raise InvalidProfile('invalid thread count')
        rows=[];threads=[];relays=[]
        for i in range(RECORDS):
            off=RECORD_BASE+i*RECORD_BYTES
            prefix=os.pread(fd,64,off)
            if len(prefix)!=64:continue
            seq,start,end,n,index,dropped,flags,_=struct.unpack('<8Q',prefix)
            if seq==0 or seq&1 or n>MAX_PAYLOAD:continue
            body=os.pread(fd,n,off+64)
            after=os.pread(fd,8,off)
            if len(body)!=n or len(after)!=8 or struct.unpack('<Q',after)[0]!=seq:continue
            try:row=_json(body)
            except InvalidProfile:continue
            if row.get('start_ns')!=start or row.get('end_ns')!=end:continue
            rows.append(row)
        for i in range(64):
            off=THREAD_BASE+i*THREAD_BYTES;body=os.pread(fd,112,off);after=os.pread(fd,8,off)
            if len(body)!=112 or len(after)!=8:continue
            seq,tid,role,state,arg,since,site,dropped,_,_,_,_,born,retired=struct.unpack('<14Q',body)
            if seq==0 or seq&1 or struct.unpack('<Q',after)[0]!=seq:continue
            threads.append(dict(slot=i,tid=tid,role=role,state=state,arg=arg,since_ns=since,site=site,dropped=dropped,birth_ns=born,retired_ns=retired,active=retired==0))
        sources=os.pread(fd,1048576,12587008)
        if len(sources)==1048576:
            for i in range(4096):
                v=struct.unpack_from('<32Q',sources,i*256)
                if v[0]==0 or v[0]&1 or v[1]!=1:continue
                after=os.pread(fd,8,12587008+i*256)
                if len(after)!=8 or struct.unpack('<Q',after)[0]!=v[0]:continue
                relays.append(_relay(i,v))
        rows.sort(key=lambda r:r['end_ns'])
        return dict(header={'last_update_ns':struct.unpack_from('<Q',h,56)[0],'clock_mode':'diagnostic_os_clock' if struct.unpack_from('<Q',h,40)[0]&1 else 'broker_monotonic' if struct.unpack_from('<Q',h,40)[0]&4 else 'calibrated_tsc'},records=rows,threads=threads,relays=relays)
    finally:os.close(fd)

def read_jsonl(path):
    rows=[];consumed=0
    with open(path,'rb') as f:
        while True:
            line=f.readline(MAX_PAYLOAD+2)
            if not line:break
            consumed+=len(line)
            if consumed>MAX_JSONL_BYTES or len(line)>MAX_PAYLOAD+1:raise InvalidProfile('input limit exceeded')
            if not line.endswith(b'\n'):break
            try:rows.append(_json(line))
            except InvalidProfile:continue
    return rows

def _count(c,key):
    n=c.get(key)
    return n if type(n) is int and 0<=n<2**64 else None

def ratios(row):
    c=row.get('counters',{});out={}
    for numerator,label in [('rmcontrol_seen','rmcontrol'),('host_rm_calls','host_rm')]:
        for denominator,dlabel in [('display_new_frames','display_new'),('broker_presented_new_frames','broker_new')]:
            n,d=_count(c,numerator),_count(c,denominator)
            out[f'{label}_per_{dlabel}']=n/d if n is not None and d else None
    return out

def pacing(row):
    c=row.get('counters',{});g=row.get('pacing',{});b=row.get('broker',{})
    c=c if isinstance(c,dict) else {}
    g=g if isinstance(g,dict) else {}
    b=b if isinstance(b,dict) else {}
    elapsed=row.get('end_ns',0)-row.get('start_ns',0);frames=_count(c,'broker_presented_new_frames')
    repeats=_count(b,'repeated')
    def low(key):
        gap=_count(g,key)
        return 1e9/gap if gap is not None and gap>0 else None
    return dict(start_ns=row.get('start_ns'),end_ns=row.get('end_ns'),new_fps=frames*1e9/elapsed if frames is not None and elapsed>0 else None,repeated_per_second=repeats*1e9/elapsed if frames is not None and repeats is not None and elapsed>0 else None,low_1pct_fps=low('p99_gap_ns'),low_01pct_fps=low('p999_gap_ns'),basis=g.get('basis','UNKNOWN'))

def summarize(rows):
    covered=[]
    for r in rows:
        c=r.get('counters',{})
        if _count(c,'rmcontrol_seen') is not None and _count(c,'display_new_frames') is not None:covered.append(r)
    sums={}
    for r in covered:
        for k,v in r.get('counters',{}).items():
            if _count(r['counters'],k) is not None:sums[k]=sums.get(k,0)+v
    return dict(pacing_per_second=[pacing(r) for r in rows if 'pacing' in r],records=len(rows),ratio_covered_records=len(covered),counters=sums,ratios=ratios({'counters':sums}))

def load(path):
    p=Path(path)
    if p.is_dir():p=p/'prof.mmap'
    with open(p,'rb') as f:magic=f.read(8)
    if p.suffix=='.mmap' or magic==b'KFPROF01':return read_mmap(p)
    rows=read_jsonl(p);return dict(records=rows,threads=rows[-1].get('threads',[]) if rows else [],header={})

def main():
    p=argparse.ArgumentParser(description=__doc__);sub=p.add_subparsers(dest='command',required=True)
    for name in ['tail','summary','window','hangs','relays']:
        a=sub.add_parser(name);a.add_argument('file')
        if name=='tail':a.add_argument('--once',action='store_true')
        if name=='window':a.add_argument('t0',type=int);a.add_argument('t1',type=int)
        if name=='summary':a.add_argument('range',nargs='?')
        if name=='hangs':a.add_argument('--stuck-ms',type=float,default=100)
    m=sub.add_parser('merge');m.add_argument('host');m.add_argument('broker');m.add_argument('--guest');m.add_argument('--calibration')
    args=p.parse_args()
    try:
        if args.command=='merge':
            from merge import merge_files
            print(json.dumps(merge_files(args.host,args.broker,args.guest,args.calibration),allow_nan=False));return
        while True:
            data=load(args.file);rows=data['records']
            if args.command=='tail':
                r=rows[-1] if rows else {};c=r.get('counters',{});v=ratios(r)['rmcontrol_per_display_new']
                print(f"mono_ns={r.get('end_ns','UNKNOWN')} last_update_ns={data['header'].get('last_update_ns','UNKNOWN')} RmControl/frame[display_new]={v if v is not None else 'N/A/UNKNOWN'} calls={c.get('rmcontrol_seen','UNKNOWN')} frames={c.get('display_new_frames','UNKNOWN')}",flush=True)
                if 'pacing' in r:print(json.dumps(pacing(r),allow_nan=False),flush=True)
                for q in data.get('relays',[])[:64]:
                    now=time.monotonic_ns();get_age=(now-q['private_get_changed_ns'])/1e6 if q['private_get'] is not None else None
                    print(f"relay={q['key']:x}/{q['generation']} private_GET/PUT/REF={q['private_get']}/{q['private_put']}/{q['reference']} depth={q['private_depth']} Wait={q['wait']} get_change_age_ms={get_age} doorbells={q['doorbell_count']} doorbell_seen_ns={q['last_doorbell_observed_ns']} wake={q['wake_reason']} sample_ns={q['private_observed_ns']}",flush=True)
                if args.once:break
                time.sleep(1);continue
            if args.command=='relays':
                out=data.get('relays',[]);observed_now=time.monotonic_ns()
                for q in out:
                    q['observation_age_ms']=max(0,observed_now-q['observed_ns'])/1e6
                    for domain in ['guest','private']:
                        changed=q.get(domain+'_get_changed_ns')
                        q[domain+'_ms_since_last_observed_get_change']=max(0,observed_now-changed)/1e6 if changed is not None and (domain!='private' or q.get('private_get') is not None) else None
            elif args.command=='window':rows=[r for r in rows if r.get('start_ns',0)<args.t1 and r.get('end_ns',0)>args.t0];out=rows
            elif args.command=='hangs':
                last=rows[-1] if rows else {};now=last.get('end_ns',data['header'].get('last_update_ns',0))
                out=dict(last_update_ns=now,stale_age_ns=max(0,time.monotonic_ns()-now),threads=[t for t in data['threads'] if t.get('active',True) and t.get('state') not in (0,17) and now-t.get('since_ns',now)>args.stuck_ms*1e6],ledger=[e for e in last.get('ledger',{}).get('oldest',[]) if e.get('overdue')])
            else:
                if args.range:
                    lo,hi=map(int,args.range.split('..'));rows=[r for r in rows if r.get('start_ns',0)<hi and r.get('end_ns',0)>lo]
                out=summarize(rows)
            print(json.dumps(out,allow_nan=False));break
    except (OSError,InvalidProfile,ValueError) as e:p.exit(2,f'kf-prof: {e}\n')
if __name__=='__main__':main()
