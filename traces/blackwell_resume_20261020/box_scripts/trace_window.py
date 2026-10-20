"""Rental-side: BAR0 trace records between the doorbell write of the last non-teardown RPC and the
teardown start RPC (aligned by counting 0x110c00 doorbell writes in both logs). Bounded output."""
import re,sys,collections
qlog,tlog,out=sys.argv[1:4]
L=open(qlog,'rb').read().decode('utf-8','replace').splitlines()
TR=re.compile(r'kf-rm: rpc-trace fn=(\d+) \w+ seq=\d+ ?(?:cmd=(0x[0-9a-f]+)|class=(0x[0-9a-f]+))?')
DB=re.compile(r'w#\d+ bar0 @0x110c00 ')
rows=[];db=0
for l in L:
  if DB.search(l): db+=1
  m=TR.search(l)
  if m: rows.append((m[1],m[2] or m[3] or '-',db))
def tdlike(r): return r[0]=='10' or r[1]=='0x50700117'
td=None;run=0
for i,r in enumerate(rows):
  run=run+1 if tdlike(r) else 0
  if run==20: td=i-19; break
a=rows[td-1][2]; b=rows[td][2]   # doorbell count at last pre-teardown RPC and at teardown start
W=re.compile(r'vfio_region_write \(\S+:region0\+0x110c00,')
T=open(tlog,'rb').read().decode('utf-8','replace').splitlines()
n=0;win=[]
for l in T:
  if W.search(l): n+=1
  if a-1<=n<=b: win.append(l)
cnt=collections.Counter()
for l in win:
  m=re.search(r'(read|write) \(\S+:region0\+0x([0-9a-f]+)',l)
  if m: cnt[(m[1],m[2])]+=1
with open(out,'w') as f:
  f.write(f'teardown_index={td} last_pre_teardown={rows[td-1][:2]} doorbells a={a} b={b} trace_lines={len(T)} window_lines={len(win)}\n')
  for (k,o),c in cnt.most_common(60): f.write(f'COUNT {k} 0x{o} {c}\n')
  for l in win[:1500]: f.write(l[:200]+'\n')
print(open(out).read()[:400])
