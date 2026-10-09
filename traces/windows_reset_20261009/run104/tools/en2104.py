import re
RR=re.compile(r"region0\+0x([0-9a-f]+), 0x([0-9a-f]+), \d\)")
def run(path,label):
    en=[0]*8; ever=[0]*8
    for l in open(path,errors="replace"):
        if "vfio_region_write" not in l: continue
        m=RR.search(l)
        if not m: continue
        o=int(m.group(1),16); v=int(m.group(2),16)
        if 0xb81200<=o<=0xb8121c:
            i=(o-0xb81200)//4; en[i]|=v; ever[i]|=v
        elif 0xb81400<=o<=0xb8141c:
            i=(o-0xb81400)//4; en[i]&=~v
    print("==",label)
    for i in range(8):
        if ever[i]:
            on=[i*32+b for b in range(32) if en[i]>>b&1]; ev=[i*32+b for b in range(32) if ever[i]>>b&1]
            print("  leaf",i,"enabled at end:",on,"| ever enabled:",ev)
run("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log","HW boot3 (guest enable writes)")
run("/var/lib/kf-windows-20261005/boundary-kayfabe-104/trace.log","KAYFABE run104 (guest enable writes)")
