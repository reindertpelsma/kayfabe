#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import re,collections,json,statistics,hashlib,pathlib
import argparse
parser=argparse.ArgumentParser(description='Mine retained kvm_mmio records, without running a VM')
parser.add_argument('trace',type=pathlib.Path);parser.add_argument('output',type=pathlib.Path)
args=parser.parse_args();p=args.trace; events=[]
for l in p.open():
 m=re.match(r'M (\d+) (\d+) (\d+) ([0-9a-f]+) ([0-9a-f]+)$',l.strip())
 if m:
  t,tid,kind,gpa,value=m.groups(); off=int(gpa,16)&0xffffff
  if 0xb81000<=off<0xb82000 or off==0xbb0090: events.append((int(t),int(tid),int(kind),off,int(value,16)))
events.sort(); cycles=[];active={}
for e in events:
 if e[3]==0xb81408:
  if active.get(e[1]): cycles.append(active[e[1]])
  active[e[1]]=[]
 if e[1] in active and e[3]!=0xbb0090: active[e[1]].append(e)
cycles.extend(c for c in active.values() if c)
cycles.sort(key=lambda c:c[0][0])
shapes=collections.Counter(tuple((x[3],x[4],x[2]) for x in c) for c in cycles)
counts=collections.Counter((e[3],e[2]) for e in events); classes=collections.Counter()
for t,tid,kind,off,val in events:
 cls='doorbell' if off==0xbb0090 else 'mask' if off in [0xb81608,0xb81610] or 0xb81200<=off<0xb81600 else 'ack' if 0xb81000<=off<0xb81040 else 'trigger' if off==0xb81640 else 'other'
 classes[cls]+=1
spacing=[(b[0]-a[0])/1000 for c in cycles for a,b in zip(c,c[1:]) if a[1]==b[1]]
report={'source':'P1 p1on1-direct-final-3_trace/trace.txt','sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'mmio_records':len(events),'types':dict(collections.Counter(str(x[2]) for x in events)),'classes':dict(classes),'mask_fraction_of_interrupt_writes':classes['mask']/(len(events)-classes['doorbell']),'cycles':len(cycles),'offset_counts':[{ 'off':hex(o),'type':k,'count':n} for (o,k),n in sorted(counts.items())], 'top_shapes':[{'count':n,'ordered':[(hex(o),hex(v),'W' if k==2 else 'R' if k==1 else 'UNSAT') for o,v,k in shape]} for shape,n in shapes.most_common(6)],'successive_write_spacing_us':{'n':len(spacing),'median':statistics.median(spacing),'min':min(spacing),'max':max(spacing)},'limits':['whole capture including background/warmup; per-TID windows delimited by EN_CLEAR(2), not guest ISR entry; last window per TID may be truncated','type=2 is write; no read record does NOT mean no guest RAM-backed reads','no coherent pending/enable snapshots: cannot count masked then enabled edges','spacing includes guest instructions, interrupt/DPC gaps, scheduling and tracing overhead; not pure VM-exit latency']}
args.output.write_text(json.dumps(report,indent=2)+'\n'); print(json.dumps(report,indent=2))
