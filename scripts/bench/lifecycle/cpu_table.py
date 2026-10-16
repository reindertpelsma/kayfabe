import re, sys, glob, os
EV="/data/claude-perf2/ev/heaven"
print("%-16s %8s %8s %8s %8s %9s  %s" % ("run", "w0 u+s", "w1 u+s", "vCPUs", "disp", "inv_csw_w", "(seconds over the whole run ~320 s)"))
for r in sys.argv[1:]:
    p=f"{EV}/{r}/threads_cpu.txt"
    if not os.path.exists(p): continue
    w={}; vc=0.0; disp=0.0; inv=0
    for l in open(p):
        f=l.split()
        if len(f)<6 or not f[0].isdigit(): continue
        c=f[1]; t=float(f[2])+float(f[3])
        if c.startswith("kf3-worker"): w[c]=t; inv+=int(f[5])
        elif c.startswith("qemu-system") and t>20: vc+=t
        elif c.startswith("kf3-display"): disp=t
    print("%-16s %8.0f %8.0f %8.0f %8.0f %9d" % (r, w.get("kf3-worker0",0), w.get("kf3-worker1",0), vc, disp, inv))
