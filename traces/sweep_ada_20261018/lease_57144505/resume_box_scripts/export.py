"""Rental-side bounded export of one arm's QEMU log: RPC metadata, alloc headers, kf lines at abort. No bodies."""
import re,sys,json,hashlib,subprocess,collections
from pathlib import Path
log=Path(sys.argv[1]); out=Path(sys.argv[2]); out.mkdir(parents=True,exist_ok=True)
data=log.read_bytes(); assert len(data)<512*1024*1024
lines=data.decode('utf-8','replace').splitlines()
TR=re.compile(r'kf-rm: rpc-trace fn=(\d+) ([A-Za-z0-9_]+(?:\(\d+\))?) seq=(\d+) ?(?:cmd=(0x[0-9a-f]+)|class=(0x[0-9a-f]+))?.*? result=(\S+)')
rows=[];idx=[]
for i,l in enumerate(lines):
  m=TR.search(l)
  if m:
    rows.append((len(rows),m[1],m[2],m[3],m[4] or m[5] or '-',m[6])); idx.append(i)
with (out/'api.tsv').open('w') as f:
  f.write('ordinal\tfunction\tkind\trpc_sequence\tcontrol_or_class\ttrace_result\n')
  for r in rows: f.write('\t'.join(map(str,r))+'\n')
unl=[r[0] for r in rows if r[1]=='47']
# teardown start: first run of >=20 consecutive Free (fn 10)
td=None;run=0
def tdlike(r): return r[1]=='10' or r[4]=='0x50700117'
for r in rows:
  run=run+1 if tdlike(r) else 0
  if run==20: td=r[0]-19; break
# kf lines (no rpc-trace/reply-body/hex) around teardown start .. unload
def keep(l): return ('kf' in l[:40]) and 'reply-body' not in l and 'rpc-trace' not in l and len(l)<=400
ctx=[]
if td is not None:
  lo=idx[max(0,td-15)]; hi=idx[unl[0]] if unl else idx[-1]
  ctx=[f'{i}: {lines[i]}' for i in range(lo,min(hi+40,len(lines))) if keep(lines[i]) or 'rpc-trace' in lines[i]]
  ctx=[re.sub(r'(client|object|parent|handle)=0x[0-9a-f]+',r'\1=H',c) for c in ctx][:600]
(out/'abort_context.txt').write_text('\n'.join(ctx)+'\n')
cnt=collections.Counter()
for l in lines:
  if keep(l):
    k=re.sub(r'0x[0-9a-f]+|\d+','N',l)[:120]; cnt[k]+=1
(out/'kf_line_census.txt').write_text(''.join(f'{n}\t{k}\n' for k,n in cnt.most_common(80)))
nonok=[r for r in rows if r[5] not in ('0x0','none')]
subprocess.run(['python3','-I','/root/s2/alloc_headers.py',str(log),str(out/'allocations.jsonl')],stdout=subprocess.DEVNULL,check=False)
al=[json.loads(x) for x in (out/'allocations.jsonl').read_text().splitlines()] if (out/'allocations.jsonl').exists() else []
summ={'log_sha256':hashlib.sha256(data).hexdigest(),'rpcs':len(rows),'unload_ordinals':unl,'teardown_start_ordinal':td,
 'last_nonfree_before_teardown':[r for r in rows if td is not None and r[0]<td and not tdlike(r)][-3:],
 'policy_nonok':nonok,'alloc_posted_nonzero_body_status':[a for a in al if a.get('body_status') not in (None,'0x0') or a.get('envelope_result')!='0x0'],
 'cfb0_posted':[a for a in al if a.get('class_selector')=='0xcfb0'],'ca7a_posted':[a for a in al if a.get('class_selector')=='0xca7a'],'cbcb_posted':[a for a in al if a.get('class_selector')=='0xcbcb']}
(out/'summary.json').write_text(json.dumps(summ,indent=1)+'\n'); print(json.dumps({k:summ[k] for k in ('rpcs','unload_ordinals','teardown_start_ordinal','last_nonfree_before_teardown')}))
