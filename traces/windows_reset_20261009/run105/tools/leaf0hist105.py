import re,collections,sys
RR=re.compile(r"region0\+0x([0-9a-f]+), (?:0x[0-9a-f]+, )?\d\)(?: = 0x([0-9a-f]+))?")
for label,path in (("run104 (relay on)","/var/lib/kf-windows-20261005/boundary-kayfabe-104/trace.log"),("run105 (relay off)","/var/lib/kf-windows-20261005/boundary-kayfabe-105/trace.log")):
    c=collections.Counter(); n=0; msi=0; r611ec0=0
    for l in open(path,errors="replace"):
        if " vfio_msi_interrupt " in l: msi+=1; continue
        if "vfio_region_read" not in l: continue
        m=RR.search(l)
        if not m: continue
        o=int(m.group(1),16)
        if o==0xb81000: c[m.group(2)]+=1; n+=1
        elif o==0x611ec0: r611ec0+=1
    tot=sum(c.values())
    bit1=sum(v for k,v in c.items() if int(k,16)&2)
    bit2=sum(v for k,v in c.items() if int(k,16)&4)
    print(label,"msi",msi,"LEAF(0) reads",tot,"with bit1 (CE2, vec 1)",bit1,"with bit2 (CE3, vec 2)",bit2,"top",c.most_common(6),"reads of 0x611ec0",r611ec0)
