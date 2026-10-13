import sys,json,struct
for path in sys.argv[1:]:
 print(path)
 for line in open(path):
  try:r=json.loads(line)
  except ValueError:continue
  if r.get('kind')!='record' or r.get('direction')!=1 or r.get('rpc_function')!=76:continue
  p=bytes.fromhex(r['payload_hex']);o=p.find(b'VRPC')-4
  if o<0:continue
  b=p[o+32:];c=struct.unpack_from('<I',b,8)[0]
  if c in [0x20800a1f,0x20801227]: print(hex(c),b[40:63].hex())
