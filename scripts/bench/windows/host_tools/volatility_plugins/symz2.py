import json,lzma,glob,bisect,re,sys
f=glob.glob("venv/lib/python3.14/site-packages/volatility3/symbols/windows/ntkrnlmp.pdb/*.json.xz")[0]
d=json.load(lzma.open(f)); S=sorted((v["address"],k) for k,v in d["symbols"].items() if "address" in v); A=[a for a,_ in S]
def nm(r):
    i=bisect.bisect_right(A,r)-1
    return "%s+0x%x"%(S[i][1],r-S[i][0]) if i>=0 else hex(r)
fn,pid,tid=sys.argv[1],sys.argv[2],sys.argv[3]
found=False
for ln in open(fn):
    p=ln.rstrip("\n").split("\t")
    if len(p)>=5 and p[0]==pid and p[2]==tid:
        found=True
        fr=[x for x in p[4].split(" < ") if not x.startswith("BAD") and not x.startswith("?")]
        print("  OWNER",p[0],p[1],"tid",p[2],"wr",p[3]); 
        for x in fr[:22]:
            m=re.match(r"ntoskrnl.exe\+0x([0-9a-f]+)$",x); print("     ",nm(int(m.group(1),16)) if m else x)
if not found: print("  OWNER",pid,tid,"is NOT in the waiting-thread list (running/ready/terminated)")
