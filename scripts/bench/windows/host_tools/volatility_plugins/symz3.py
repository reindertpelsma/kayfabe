import json,lzma,glob,bisect,re,sys
def load(f,op=open):
    d=json.load(op(f)); S=sorted((v["address"],k) for k,v in d["symbols"].items() if "address" in v); return [a for a,_ in S],S
nt=load(glob.glob("venv/lib/python3.14/site-packages/volatility3/symbols/windows/ntkrnlmp.pdb/*.json.xz")[0],lzma.open)
mods={"ntoskrnl.exe":nt,"dxgkrnl.sys":load("../re/pdb/dxgkrnl.json"),"dxgmms2.sys":load("../re/pdb/dxgmms2.json"),"win32kbase.sys":load("../re/pdb/win32kbase.json")}
def nm(m,r):
    if m not in mods: return "%s+0x%x"%(m,r)
    A,S=mods[m]; i=bisect.bisect_right(A,r)-1
    return "%s!%s+0x%x"%(m.split(".")[0],S[i][1][:90],r-S[i][0]) if i>=0 else "%s+0x%x"%(m,r)
pat=sys.argv[2] if len(sys.argv)>2 else "dxgkrnl|dxgmms2"
for ln in open(sys.argv[1]):
    p=ln.rstrip("\n").split("\t")
    if len(p)<7 or not re.search(pat,p[6]): continue
    print("== pid",p[0],p[1],"tid",p[2],"state",p[3],"wr",p[4],"waited",p[5])
    for x in p[6].split(" < "):
        m=re.match(r"(\S+?)\+0x([0-9a-f]+)$",x)
        print("    ",nm(m.group(1),int(m.group(2),16)) if m else x)
