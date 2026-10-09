import json,struct,collections
def run(path):
    cmds=collections.Counter(); ev=0; t0=None; t1=None; evt=[]; reqt=[]
    for line in open(path,errors="replace"):
        try: r=json.loads(line)
        except Exception: continue
        if r.get("kind")!="record": continue
        p=bytes.fromhex(r["payload_hex"]); i=p.find(b"VRPC")
        if i<4: continue
        hv,sig,ln,fn,res,resp,sq,sp=struct.unpack_from("<8I",p,i-4)
        t=r["qpc"]/1e9; t0=t if t0 is None else t0; t1=t
        b=p[i-4+32:]
        if fn==76 and r["direction"]==0 and len(b)>=12:
            c=struct.unpack_from("<3I",b,0)[2]; cmds[c]+=1
            if c==0xa06c0105: reqt.append(t-t0)
        if fn==0x1003 and r["direction"]==1 and len(b)>=12 and struct.unpack_from("<3I",b,0)[2]==139:
            ev+=1; evt.append(t-t0)
    return cmds,ev,t1-t0,reqt,evt
for label,path in (("HW boot3","/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/gsp.jsonl"),("KAYFABE run104","/var/lib/kf-windows-20261005/boundary-kayfabe-104/gsp.jsonl")):
    c,ev,span,reqt,evt=run(path)
    print("==",label,"span %.1f s"%span)
    print("  NVA06C_CTRL_CMD_PREEMPT requests:",c[0xa06c0105]," FIFO_CHANNEL_PREEMPTIVE_REMOVAL:",c[0x2080110a]," PREEMPT_COMPLETE posts:",ev)
    print("  preempt request times (s):",[round(x,1) for x in reqt][:30])
    print("  complete post times (s):  ",[round(x,1) for x in evt][:30])
