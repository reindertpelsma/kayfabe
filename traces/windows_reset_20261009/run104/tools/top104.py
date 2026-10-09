import re,collections
RR=re.compile(r"region0\+0x([0-9a-f]+), (?:0x[0-9a-f]+, )?\d\)(?: = 0x([0-9a-f]+))?")
def run(path,t0,t1,label):
    off=collections.Counter(); msi=collections.Counter(); vals=collections.defaultdict(collections.Counter)
    for l in open(path,errors="replace"):
        t=l[11:19]
        if not (t0<=t<=t1): continue
        if " vfio_msi_interrupt " in l or " vfio_msi_interrupt  " in l:
            msi[l.split("vfio_msi_interrupt",1)[1].strip()[:60]]+=1; continue
        if "vfio_region_" not in l: continue
        k="r" if "vfio_region_read" in l else "w"
        m=RR.search(l)
        if not m: continue
        o=int(m.group(1),16); off[(k,o)]+=1
        if m.group(2): vals[o][m.group(2)]+=1
    print("==",label,t0,t1)
    print("msi by arg:",msi.most_common(6))
    for (k,o),c in off.most_common(14): print(" ",k,hex(o),c,"vals:",vals[o].most_common(3) if k=="r" else "")
run("/var/lib/kf-windows-20261005/boundary-kayfabe-104/trace.log","08:10:43","08:10:45","KAYFABE run104 lock screen (3 s)")
run("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log","20:10:55","20:10:57","HW boot3 idle (3 s)")
