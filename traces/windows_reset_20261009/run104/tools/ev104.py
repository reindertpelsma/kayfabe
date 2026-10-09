import json,struct,collections
def run(path):
    c=collections.Counter()
    for line in open(path,errors="replace"):
        try: r=json.loads(line)
        except Exception: continue
        if r.get("kind")!="record": continue
        p=bytes.fromhex(r["payload_hex"]); i=p.find(b"VRPC")
        if i<4: continue
        hv,sig,ln,fn,res,resp,sq,sp=struct.unpack_from("<8I",p,i-4)
        c[(r["direction"],fn)]+=1
    return c
H=run("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/gsp.jsonl")
K=run("/var/lib/kf-windows-20261005/boundary-kayfabe-104/gsp.jsonl")
print("GSP->guest records (direction 1) by rpc function; fn >= 0x1000 are events")
keys=sorted({k for k in H if k[0]==1}|{k for k in K if k[0]==1}, key=lambda k:(k[1]>=0x1000,k[1]))
for k in keys:
    if k[1]>=0x1000 or H[k]!=K[k]:
        print("  fn %5d (0x%04x)  hw %6d  kf %6d"%(k[1],k[1],H[k],K[k]))
