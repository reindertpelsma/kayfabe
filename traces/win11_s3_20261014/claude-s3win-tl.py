import re,sys
rows=[]
for l in open(sys.argv[1]):
    m=re.match(r"\s*([\d.]+):\s+kvm:kvm_pio: pio_(read|write) at (0x[0-9a-f]+) size (\d) count \d+ val (0x[0-9a-f]+)",l)
    if m: rows.append((float(m[1]),m[2],int(m[3],16),int(m[4]),int(m[5],16)))
t0=rows[0][0]; cf8=0; out=[]
def add(t,s):
    if out and out[-1][1]==s: out[-1][2]+=1
    else: out.append([t,s,1])
pt=0
for t,rw,p,sz,v in rows:
    if p==0xb008: pt+=1; continue
    if pt: add(t,"PMTMR x%d"%pt) if False else None
    if p==0xcf8 and rw=="write": cf8=v; continue
    if p==0xcf8: continue
    if 0xcfc<=p<=0xcff:
        if cf8&0x80000000:
            b=(cf8>>16)&0xff;d=(cf8>>11)&31;f=(cf8>>8)&7;r=(cf8&0xfc)+(p-0xcfc)
            add(t,"PCI %s %02x:%02x.%d reg %02x = %x"%(rw,b,d,f,r,v)); 
        continue
    nm={0x514:"fwcfg-dma",0x518:"fwcfg-dma",0x510:"fwcfg",0x511:"fwcfg",0x64:"kbc",0x60:"kbc",0x70:"cmos",0x71:"cmos",0xaf00:"pcihp",0xaf04:"pcihp",0xaf08:"pcihp",0xb000:"pm1",0xb002:"pm1",0xb004:"pm1cnt"}.get(p,hex(p))
    add(t,"%s %s %x"%(nm,rw,v) if nm not in("fwcfg-dma","kbc") else nm)
print("pm timer reads:",pt)
for t,s,n in out[:int(sys.argv[2]) if len(sys.argv)>2 else 80]: print("%.3f %s%s"%(t-t0,s," x%d"%n if n>1 else ""))
