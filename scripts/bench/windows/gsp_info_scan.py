#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# usage: gsp_info_scan.py GSP_JSONL  -- list the FB/BUS/GPU_GET_INFO_V2 index lists (request and reply) in a VFIO observer capture.
# Layout: RPC header at the "VRPC" signature - 4 (32 bytes); fn 76 body = hClient hObject cmd status paramsSize flags + 16 bytes, params at +40.
import json,struct,sys
CMDS={0x20801303:'FB_GET_INFO_V2',0x20801823:'BUS_GET_INFO_V2',0x20800102:'GPU_GET_INFO_V2'}
path=sys.argv[1]
seq=0
for line in open(path):
    r=json.loads(line)
    if r.get('kind')!='record': continue
    p=bytes.fromhex(r['payload_hex'])
    i=p.find(b'VRPC')
    if i<4: continue
    hv,sig,ln,fn,res,resp,sq,spare=struct.unpack_from('<8I',p,i-4)
    if fn!=76: continue
    body=p[i-4+32:]
    if len(body)<44: continue
    hc,ho,cmd,st,psz,fl,rf=struct.unpack_from('<7I',body,0)
    if cmd not in CMDS: continue
    params=body[40:]
    n=struct.unpack_from('<I',params,0)[0]
    ent=[struct.unpack_from('<II',params,4+8*k) for k in range(min(n,0x80)) if 4+8*k+8<=len(params)]
    print(r['direction'],r['queue_sequence'],CMDS[cmd],'status=%x'%st,'n=%d'%n,' '.join('%x:%x'%e for e in ent))
