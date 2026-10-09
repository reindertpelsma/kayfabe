import re
RR=re.compile(r"region0\+0x([0-9a-f]+), (?:0x([0-9a-f]+), )?\d\)(?: = 0x([0-9a-f]+))?")
def run(path,label,tmin=None,tmax=None):
    print("==",label); n=0
    last=None
    for l in open(path,errors="replace"):
        t=l[11:23]
        if "vfio_region_write" not in l: continue
        m=RR.search(l)
        if not m: continue
        o=int(m.group(1),16)
        if 0x611d00<=o<0x611e00 or 0x611f00<=o<0x612000 or o in (0x611ec0,):
            n+=1
            if n<=40: print(" ",t,hex(o),"<-",m.group(2))
    print(" total writes in 0x611d00-0x611dff/0x611f00-:",n)
run("/var/lib/kf-windows-20261005/boundary-kayfabe-105/trace.log","KAYFABE run105")
run("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log","HW boot3")
