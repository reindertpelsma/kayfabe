import re,sys
log=sys.argv[1];va=int(sys.argv[2],16)
pat=re.compile(r"maplog t=([0-9.]+) walk#(\d+) (VasKey\(\d+\)) root (0x[0-9a-f]+) (MAP|UNMAP) va=(0x[0-9a-f]+) len=(0x[0-9a-f]+) at=(0x[0-9a-f]+) ap=(\S+) kind=(0x[0-9a-f]+) ack=(\S+)")
n=0
with open(log,"rb") as f:
  for i,raw in enumerate(f,1):
    if b"maplog" not in raw and b"RC-SEEN" not in raw: continue
    l=raw.decode("utf-8","replace")
    if "RC-SEEN" in l: print(i,l.strip()[:300]); continue
    m=pat.search(l)
    if not m: continue
    a=int(m[6],16);ln=int(m[7],16)
    if a<=va<a+ln:
      n+=1; print(i,m[1],"walk",m[2],m[3],m[4],m[5],m[6],m[7],"at",m[8],"ap",m[9],"kind",m[10],"ack",m[11])
print("matches",n)
