import json,lzma,glob,bisect,re,sys
f=glob.glob("venv/lib/python3.14/site-packages/volatility3/symbols/windows/ntkrnlmp.pdb/*.json.xz")[0]
d=json.load(lzma.open(f))
S=sorted((v["address"],k) for k,v in d["symbols"].items() if "address" in v)
A=[a for a,_ in S]
def nm(rva):
    i=bisect.bisect_right(A,rva)-1
    return "%s+0x%x"%(S[i][1],rva-S[i][0]) if i>=0 else hex(rva)
pat=sys.argv[1]
for ln in open("unwind223.txt"):
    p=ln.rstrip("\n").split("\t")
    if len(p)<5 or not re.search(pat,p[4]): continue
    fr=[x for x in p[4].split(" < ") if not x.startswith("BAD") and not x.startswith("?")]
    out=[]
    for x in fr:
        m=re.match(r"ntoskrnl.exe\+0x([0-9a-f]+)$",x)
        out.append(nm(int(m.group(1),16)) if m else x)
    print(p[0],p[1],"tid",p[2],"wr",p[3]); print("   "+"\n   ".join(out[:26]))
