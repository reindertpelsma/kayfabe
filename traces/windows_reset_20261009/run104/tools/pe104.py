import json,struct,collections
def run(path):
    cls={}; posts=[]; allocs=collections.Counter()
    for line in open(path,errors="replace"):
        try: r=json.loads(line)
        except Exception: continue
        if r.get("kind")!="record": continue
        p=bytes.fromhex(r["payload_hex"]); i=p.find(b"VRPC")
        if i<4: continue
        hv,sig,ln,fn,res,resp,sq,sp=struct.unpack_from("<8I",p,i-4)
        b=p[i-4+32:]
        if fn==103 and r["direction"]==0 and len(b)>=32:
            hc,hp,ho,c=struct.unpack_from("<4I",b,0); cls[(hc,ho)]=c
            if c in (0x78,0x79,0x7e) and len(b)>=32+16:
                allocs[(cls.get((hc,hp),0),struct.unpack_from("<I",b,32+12)[0],c)]+=1
        if fn==0x1003 and r["direction"]==1 and len(b)>=12:
            hc,he,ni=struct.unpack_from("<3I",b,0); posts.append((hc,he,ni))
    return cls,posts,allocs
for label,path in (("HW boot3","/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/gsp.jsonl"),("KAYFABE run104","/var/lib/kf-windows-20261005/boundary-kayfabe-104/gsp.jsonl")):
    cls,posts,allocs=run(path)
    print("==",label,"POST_EVENT:",len(posts))
    c=collections.Counter((hex(hc),hex(he),ni) for hc,he,ni in posts)
    for (hc,he,ni),n in c.most_common(12): print("   client",hc,"event",he,"notifyIndex",ni,"x",n)
    print("   registered OS/callback events by (parent class, notifyIndex, event class):")
    for k,n in sorted(allocs.items(), key=lambda x:-x[1])[:14]: print("     parent_cls=%#x notifyIndex=%d class=%#x  x%d"%(k+(n,)))
