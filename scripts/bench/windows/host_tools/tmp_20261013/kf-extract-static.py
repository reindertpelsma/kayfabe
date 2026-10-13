import json,struct,sys
for line in open(sys.argv[1]):
 try: r=json.loads(line)
 except ValueError: continue
 if r.get('kind')!='record' or r.get('direction')!=1 or r.get('rpc_function')!=65: continue
 p=bytes.fromhex(r['payload_hex']); o=p.find(b'VRPC')-4
 if o<0: continue
 ln=struct.unpack_from('<I',p,o+8)[0]
 print(p[o+32:o+ln].hex());break
else: sys.exit('no fn65 reply')
