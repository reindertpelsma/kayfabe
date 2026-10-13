import json,struct,sys,collections

def read(path):
 out=collections.defaultdict(list)
 for line in open(path):
  try:r=json.loads(line)
  except ValueError:continue
  if r.get('kind')!='record' or r.get('direction')!=1 or r.get('rpc_function')!=76:continue
  p=bytes.fromhex(r['payload_hex']);o=p.find(b'VRPC')-4
  if o<0:continue
  ln=struct.unpack_from('<I',p,o+8)[0];b=p[o+32:o+ln]
  if len(b)<40:continue
  hc,ho,cmd,st,sz=struct.unpack_from('<5I',b)
  if not out[cmd]:out[cmd].append((st,sz,b[40:]))
 return out
a,b=map(read,sys.argv[1:])
for c in sorted(a.keys()&b.keys()):
 x,y=a[c][0],b[c][0]
 if c>>16==0x2080 and 0xa00<=c&0xffff<=0xaff and x!=y:
  changed=[i for i,(v,w) in enumerate(zip(x[2],y[2])) if v!=w]
  print(hex(c),'status',hex(x[0]),hex(y[0]),'size',x[1],y[1],'differentbytes',len(changed),'first',changed[:10])
