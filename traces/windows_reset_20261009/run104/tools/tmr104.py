import re,collections
RR=re.compile(r"region0\+0x([0-9a-f]+), (?:0x([0-9a-f]+), )?\d\)(?: = 0x([0-9a-f]+))?")
def run(path,label):
    c=collections.Counter(); sample={}
    for l in open(path,errors="replace"):
        k="r" if "vfio_region_read" in l else "w" if "vfio_region_write" in l else None
        if not k: continue
        m=RR.search(l)
        if not m: continue
        o=int(m.group(1),16)
        if 0x9000<=o<0xa000 or 0x1400<=o<0x1500 and False:
            c[(k,hex(o))]+=1
            sample.setdefault((k,hex(o)),(m.group(2) or m.group(3)))
    print("==",label)
    for (k,o),n in sorted(c.items(), key=lambda x:-x[1])[:12]: print("  ",k,o,n,"first value",sample[(k,o)])
run("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log","HW boot3")
run("/var/lib/kf-windows-20261005/boundary-kayfabe-104/trace.log","KAYFABE run104")
