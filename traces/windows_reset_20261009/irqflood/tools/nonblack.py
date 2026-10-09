import sys,os,glob
d=sys.argv[1]
for p in sorted(glob.glob(d+"/s-*.ppm")):
    b=open(p,"rb").read()
    # skip the PPM header (3 newline-terminated lines)
    i=0
    for _ in range(3): i=b.index(b"\n",i)+1
    px=b[i::997]
    nz=sum(1 for x in px if x>16)
    print(os.path.basename(p)[2:-4], round(nz/max(1,len(px)),3))
