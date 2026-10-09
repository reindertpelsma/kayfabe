import re,collections
R=re.compile(r"^(\S+) (vfio_region_(?:read|write)|vfio_msi_interrupt)\s+\((?:0000:01:00.0:region0\+0x([0-9a-f]+)|[^)]*)")
rows=collections.defaultdict(lambda: collections.Counter())
last=None
for l in open("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log",errors="replace"):
    m=R.match(l)
    if not m: continue
    t=m.group(1)[11:19]; ev=m.group(2)
    if ev=="vfio_msi_interrupt": rows[t]["msi"]+=1; continue
    k="r" if ev.endswith("read") else "w"
    o=int(m.group(3),16)
    reg="disp" if 0x610000<=o<0x700000 else "pmc/top" if o<0x2000 else "fifo" if 0x2000<=o<0x3000 else "pbdma/rl" if 0x40000<=o<0x80000 else "gsp/falcon" if 0x110000<=o<0x120000 or 0x800000<=o<0x900000 else "pcie/other"
    rows[t][k+":"+reg]+=1
for t in sorted(rows):
    if True: print(t,dict(rows[t]))
