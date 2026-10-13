#!/usr/bin/env python3
"""Compare control inventories (ids, counts, SIZES only) of census TSVs. Usage: compare_inventory.py REF.tsv REAL.tsv [C.tsv ...]"""
import csv, sys
def load(p):
    d={}
    for r in csv.DictReader(open(p),delimiter='\t'):
        d[r['cmd']]=r
    return d
def sizes(s):  # "12:10,16:2" -> set of sizes
    return set(x.split(':')[0] for x in s.split(',') if x and x!='-')
ref=load(sys.argv[1]); real=load(sys.argv[2])
extra={}
for p in sys.argv[3:]:
    extra[p]=load(p)
print('ref distinct controls',len(ref),' real (union of given captures) distinct controls',len(real))
common=sorted(set(ref)&set(real)); only_real=sorted(set(real)-set(ref)); only_ref=sorted(set(ref)-set(real))
print('common',len(common),'only_real',len(only_real),'only_ref',len(only_ref))
cls=lambda r:r['ogkm']
def cnt(ids,d):
    c={}
    for i in ids: c[cls(d[i])]=c.get(cls(d[i]),0)+1
    return c
print('ref by ogkm class',cnt(ref,ref)); print('real by ogkm class',cnt(real,real))
print('common by class',cnt(common,ref))
print('only_real by class',cnt(only_real,real)); print('only_ref by class',cnt(only_ref,ref))
wo_ref=[i for i in ref if cls(ref[i])=='absent_all_tags']; wo_real=[i for i in real if cls(real[i])=='absent_all_tags']
print('absent_all_tags ("Windows-only" rows): ref',len(wo_ref),' present in real',len([i for i in wo_ref if i in real]),' absent in real',len([i for i in wo_ref if i not in real]),' real-only',len([i for i in wo_real if i not in ref]))
req_diff=[];rep_diff=[]
for i in common:
    if sizes(ref[i]['req_sizes'])!=sizes(real[i]['req_sizes']): req_diff.append(i)
    if sizes(ref[i]['reply_sizes'])!=sizes(real[i]['reply_sizes']): rep_diff.append(i)
print('common ids whose request size SET differs',len(req_diff),' reply size set differs',len(rep_diff))
sub_req=[i for i in req_diff if sizes(real[i]['req_sizes'])<=sizes(ref[i]['req_sizes'])]
sub_rep=[i for i in rep_diff if sizes(real[i]['reply_sizes'])<=sizes(ref[i]['reply_sizes'])]
print('  of which real sizes are a subset of ref sizes: req',len(sub_req),'reply',len(sub_rep))
# tsv
w=csv.writer(open('inventory_comparison.tsv','w'),delimiter='\t',lineterminator='\n')
w.writerow(['cmd','ogkm','in_ref','ref_requests','ref_req_sizes','ref_reply_sizes','in_real','real_requests','real_req_sizes','real_reply_sizes','size_sets_equal'])
for i in sorted(set(ref)|set(real)):
    a=ref.get(i); b=real.get(i)
    eq=''
    if a and b: eq='yes' if sizes(a['req_sizes'])==sizes(b['req_sizes']) and sizes(a['reply_sizes'])==sizes(b['reply_sizes']) else 'no'
    w.writerow([i,(a or b)['ogkm'],int(bool(a)),a['requests'] if a else '',a['req_sizes'] if a else '',a['reply_sizes'] if a else '',int(bool(b)),b['requests'] if b else '',b['req_sizes'] if b else '',b['reply_sizes'] if b else '',eq])
print('only_real ids',only_real)
for p,d in extra.items():
    print(p,'distinct',len(d),'in ref',len(set(d)&set(ref)),'not in ref',len(set(d)-set(ref)),'windows-only(absent_all_tags) rows present',len([i for i in d if cls(d[i])=='absent_all_tags']))
