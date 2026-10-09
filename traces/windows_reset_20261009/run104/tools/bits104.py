import re,collections
RR=re.compile(r"region0\+0x([0-9a-f]+), (?:0x[0-9a-f]+, )?\d\)(?: = 0x([0-9a-f]+))?")
def run(path):
    cnt=collections.defaultdict(collections.Counter); n=collections.Counter(); first={}
    for l in open(path,errors="replace"):
        if "vfio_region_read" not in l: continue
        m=RR.search(l)
        if not m: continue
        o=int(m.group(1),16)
        if 0xb81000<=o<=0xb8101c:
            leaf=(o-0xb81000)//4; v=int(m.group(2),16); n[leaf]+=1
            for b in range(32):
                if v>>b&1:
                    cnt[leaf][b]+=1; first.setdefault((leaf,b),l[11:23])
    return cnt,n,first
H,Hn,Hf=run("/var/lib/kf-windows-20261005/vfio-dvi-20261008/boot3/trace.log")
K,Kn,Kf=run("/var/lib/kf-windows-20261005/boundary-kayfabe-104/trace.log")
print("leaf reads  hw:",dict(Hn)," kf:",dict(Kn))
print("vector = leaf*32+bit; share = fraction of that leaf reads with the bit set")
for leaf in range(8):
    bits=sorted(set(H[leaf])|set(K[leaf]))
    for b in bits:
        h=H[leaf][b]; k=K[leaf][b]
        tag="HW-ONLY" if h and not k else ("KF-ONLY" if k and not h else "both")
        print(" leaf%d bit%2d vec%3d  hw %5d/%-6d (%.2f)  kf %5d/%-6d (%.2f)  %s"%(leaf,b,leaf*32+b,h,Hn[leaf],h/max(1,Hn[leaf]),k,Kn[leaf],k/max(1,Kn[leaf]),tag))
