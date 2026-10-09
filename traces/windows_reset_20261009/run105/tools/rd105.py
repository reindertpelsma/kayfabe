import re,collections
RR=re.compile(r"region0\+0x([0-9a-f]+), (?:0x[0-9a-f]+, )?\d\)(?: = 0x([0-9a-f]+))?")
W=(0x611d80,0x611d84,0x611d00,0x611d04,0x611d40,0x611c00,0x611c04)
def run(path,label):
    print("==",label)
    vals=collections.defaultdict(collections.Counter); first={}
    for l in open(path,errors="replace"):
        if "vfio_region_read" not in l: continue
        m=RR.search(l)
        if not m: continue
        o=int(m.group(1),16)
        if o in W or (0x611d00<=o<0x611e00):
            vals[o][m.group(2)]+=1; first.setdefault(o,l[11:23])
    for o in sorted(vals): print(" ",hex(o),"first@",first[o],vals[o].most_common(5))
run("/var/lib/kf-windows-20261005/boundary-kayfabe-105/trace.log","KAYFABE run105 reads")
run("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log","HW boot3 reads")
