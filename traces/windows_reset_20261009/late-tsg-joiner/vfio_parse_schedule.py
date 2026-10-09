import json,struct,sys,collections
CH={0xa06f,0xb06f,0xc06f,0xc36f,0xc46f,0xc56f,0xc86f,0xc96f,0xca6f,0xcb6f,0xcc6f,0x906f,0x826f}
TSG=0xa06c
ev=[]
for l in open(sys.argv[1]):
    r=json.loads(l)
    if r['kind']!='record' or r['direction']!=0: continue
    b=bytes.fromhex(r['payload_hex'])
    i=b.find(b'VRPC')
    if i<4: continue
    h=i-4
    ver,sig,length,fn,res,resp,seq,u=struct.unpack_from('<8I',b,h)
    body=b[h+32:]
    ev.append((r['qpc'],fn,body,seq))
ev.sort(key=lambda e:e[0])
out=[]
for q,fn,body,seq in ev:
    if fn==103 and len(body)>=28:
        hc,hp,ho,cls,st,ps,fl=struct.unpack_from('<7I',body,0)
        if cls==TSG or cls in CH:
            out.append((q,'ALLOC',hex(cls),'client',hex(hc),'parent',hex(hp),'h',hex(ho)))
    elif fn==76 and len(body)>=20:
        hc,ho,cmd,st,ps=struct.unpack_from('<5I',body,0)
        if (cmd>>16) in (0xa06c,0xa06f,0xb06f,0xc36f,0xc56f,0xc86f,0xc96f,0x906f) :
            extra=''
            if cmd in (0xa06c0101,0xa06f0103) and ps>=3:
                extra=' params='+body[40:40+ps].hex()
            out.append((q,'CTRL',hex(cmd),'client',hex(hc),'obj',hex(ho),'ps',ps,extra))
    elif fn==103:
        pass
t0=out[0][0]
for o in out:
    print(f"{(o[0]-t0)/1e9:10.4f}",*o[1:])
