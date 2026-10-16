import json,re,sys
from pathlib import Path
rows=[]
for line in Path(sys.argv[1]).read_text(errors='replace').splitlines():
 if 'kf-gsp: reply-body ' not in line or ' function=0x4c ' not in line or ' selector=0x20800a89 ' not in line:continue
 # Public header status and declared length only. No input fields/address bytes exported.
 encoded=re.search(r' body=([0-9a-f]+)$',line)
 row={'rpc_result':re.search(r' rpc_result=(0x[0-9a-f]+)',line)[1], 'truncated':re.search(r' truncated=(\d+)',line)[1]}
 if encoded:
  body=bytes.fromhex(encoded[1]);row['prefix_bytes']=len(body)
  if len(body)>=20:
   row['control_status']=int.from_bytes(body[12:16],'little');row['params_size']=int.from_bytes(body[16:20],'little')
 rows.append(row)
assert len(rows)<=512
print(json.dumps({'a89_posted_statuses':rows,'limit':'Status and declared size only; all input buffer/address bytes omitted. Guest CPU polling behavior UNVERIFIED.'},indent=2))
