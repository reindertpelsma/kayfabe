import re,collections
RR=re.compile(r"region0\+0x([0-9a-f]+), (?:0x[0-9a-f]+, )?\d\)(?: = 0x([0-9a-f]+))?")
def run(path,label,t0,t1):
    vals=collections.defaultdict(collections.Counter); nmsi=0
    for l in open(path,errors="replace"):
        t=l[11:19]
        if not (t0<=t<=t1): continue
        if "vfio_msi_interrupt" in l: nmsi+=1; continue
        if "vfio_region_read" not in l: continue
        m=RR.search(l)
        if not m: continue
        o=int(m.group(1),16)
        if 0xb81000<=o<=0xb8101c or 0xb81600<=o<=0xb81608: vals[o][m.group(2)]+=1
    print("==",label,t0,t1,"msi",nmsi)
    for o in sorted(vals): print(" ",hex(o),vals[o].most_common(5))
run("/var/lib/kf-windows-20261005/boundary-kayfabe-105/trace.log","KAYFABE run105 quiet phase after busy (no lock screen)","09:03:13","09:03:15")
run("/var/lib/kf-windows-20261005/boundary-kayfabe-105/trace.log","KAYFABE run105 busy second","09:03:07","09:03:07")
run("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log","HW idle","20:10:55","20:10:57")
run("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log","HW busy","20:10:52","20:10:53")
