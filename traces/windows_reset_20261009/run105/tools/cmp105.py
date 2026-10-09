import re,collections
R=re.compile(r"^(\S+) vfio_region_(read|write)\s+\(0000:01:00.0:region0\+0x([0-9a-f]+), (?:0x([0-9a-f]+), )?(\d)\)(?: = 0x([0-9a-f]+))?")
def load(p):
    ev=[]
    for l in open(p,errors="replace"):
        m=R.match(l)
        if m:
            k=m.group(2); v=int(m.group(4) if k=="write" else m.group(6),16)
            ev.append((m.group(1),k,int(m.group(3),16),v))
    return ev
hw=load("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log")
kf=load("/var/lib/kf-windows-20261005/boundary-kayfabe-105/trace.log")
def first(ev):
    d=collections.OrderedDict()
    for t,k,o,v in ev:
        if (k,o) not in d: d[(k,o)]=[t,v,0,set()]
        d[(k,o)][2]+=1; d[(k,o)][3].add(v)
    return d
fh,fk=first(hw),first(kf)
for k in ("read","write"):
    h={o for (kk,o) in fh if kk==k}; f={o for (kk,o) in fk if kk==k}
    print(k,"offsets hw",len(h),"kf",len(f),"both",len(h&f),"hw-only",len(h-f),"kf-only",len(f-h))
rh={o for (k,o) in fh if k=="read"}; rk={o for (k,o) in fk if k=="read"}
def bucket(o): return hex(o>>16<<16)
print("HW-ONLY read 64K regions:",collections.Counter(bucket(o) for o in rh-rk).most_common(10))
print("KF-ONLY read 64K regions:",collections.Counter(bucket(o) for o in rk-rh).most_common(10))
diff=[(fk[("read",o)][0],o,fh[("read",o)][1],fk[("read",o)][1],sorted(fh[("read",o)][3])[:3],sorted(fk[("read",o)][3])[:3]) for o in rh&rk if fh[("read",o)][3]!=fk[("read",o)][3]]
diff.sort()
print("common read offsets whose SET of returned values differs:",len(diff),"of",len(rh&rk))
for t,o,a,b,sa,sb in diff[:30]: print(t[11:23],hex(o),"hw first=",hex(a),"kf first=",hex(b),"| hwset",[hex(x) for x in sa],"kfset",[hex(x) for x in sb])
