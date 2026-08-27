#!/usr/bin/env python3
"""mode2_diag report generator.

Joins two logs from a Mode-2 cuCtxCreate (or any) run into a clean diagnostic report:
  - GUEST dmesg with the instrumented open nvidia.ko:
        NVKVMIOC cmd=0x.. inner=0x.. arg=<ptr> size=<n>     (one per RM ioctl entry)
        NVKVMCO  to=<ptr> n=<bytes> src0=<first8>           (one per copy_to_user out)
  - QEMU log (/tmp/m0_qemu.log) M4 RPC lines giving each control's reqPsize
        (= the paramsSize libcuda allocated = its buffer size).

Output: per-ioctl sections (decoded control name, its copyouts), and a RED-FLAG list of
copyouts that exceed the owning control's reqPsize (a provable userspace-buffer overrun ==
the cuCtxCreate rbp-clobber class).

Usage: report.py <guest_dmesg.txt> <qemu.log>
"""
import sys, re, collections

# minimal control-name map (extend freely)
CTRL = {
 0x20803002:"NVLINK_GET_NVLINK_STATUS", 0x20800a22:"INTERNAL_STATIC_GET_GR_*",
 0x20800a40:"INTERNAL_STATIC_KMIGMGR_*", 0x20800a2a:"INTERNAL_STATIC_KGR_GET_INFO",
 0x20801201:"GR_GET_INFO", 0x20801112:"FIFO_GET_DEVICE_INFO_TABLE",
 0x2080012b:"GPU_PROMOTE_CTX", 0x20800a5c:"INTERNAL_INTR_GET_KERNEL_TABLE",
 0x208001b0:"GET_CONSTRUCTED_FALCON_INFO", 0x2080012f:"GR_GET_CTX_BUFFER_SIZE",
 0x906f0101:"GET_CLASS_ENGINEID", 0x0080170d:"FIFO_GET_CHANNELLIST",
}
def cname(c): return CTRL.get(c, "?")

def parse_guest(path):
    """Return ordered list of events: ('ioc',cmd,inner,arg,size) | ('co',to,n,src0)."""
    evs=[]
    for ln in open(path, errors='replace'):
        m=re.search(r'NVKVMIOC cmd=0x([0-9a-f]+) inner=0x([0-9a-f]+) arg=([0-9a-fx]+) size=(\d+)', ln)
        if m: evs.append(('ioc',int(m.group(1),16),int(m.group(2),16),int(m.group(3),16),int(m.group(4)))); continue
        m=re.search(r'NVKVMCO to=([0-9a-f]+) n=(\d+) src0=([0-9a-f]+)', ln)
        if m: evs.append(('co',int(m.group(1),16),int(m.group(2)),int(m.group(3),16)))
    return evs

def parse_qemu_reqpsize(path):
    """control cmd -> max reqPsize seen (libcuda's buffer size)."""
    d={}
    for ln in open(path, errors='replace'):
        m=re.search(r'M4: RPC fn=76 cmd=0x([0-9a-f]+) reqPsize=(\d+)', ln)
        if m:
            c=int(m.group(1),16); p=int(m.group(2)); d[c]=max(d.get(c,0),p)
    return d

def main():
    g=parse_guest(sys.argv[1]); req=parse_qemu_reqpsize(sys.argv[2]) if len(sys.argv)>2 else {}
    # attribute each copyout to the most recent ioctl
    cur=None; groups=[]; flags=[]
    for e in g:
        if e[0]=='ioc':
            cur={'cmd':e[1],'inner':e[2],'arg':e[3],'size':e[4],'cos':[]}; groups.append(cur)
        elif e[0]=='co' and cur is not None:
            to,n,src0=e[1],e[2],e[3]; cur['cos'].append((to,n,src0))
            buf=req.get(cur['inner'],None)
            # overrun heuristic: copyout into the OUTER arg buffer bigger than reqPsize,
            # OR any copyout whose size exceeds the control's declared reqPsize while landing
            # near the arg (same 64KB stack window) -> exceeds the allocation.
            inarg = cur['arg'] <= to < cur['arg']+max(cur['size'],n)
            if buf and n>buf and (inarg or abs(to-cur['arg'])<0x10000):
                flags.append((cur['inner'],cur['arg'],to,n,buf))
    print("=== RED FLAGS: copyout > control reqPsize (buffer overrun) ===")
    if not flags: print("  (none by reqPsize heuristic — overrun is likely an EMBEDDED inner buffer;")
    if not flags: print("   see the large-copyout list below and compare inner ptr vs arg)")
    for inner,arg,to,n,buf in flags:
        print(f"  ctrl 0x{inner:08x} {cname(inner):28s} arg=0x{arg:x} -> copyout to=0x{to:x} n={n} > reqPsize {buf}  *** OVERRUN ***")
    print("\n=== largest copyouts (size | owning ctrl | dst-vs-arg) ===")
    allco=[]
    for grp in groups:
        for (to,n,src0) in grp['cos']:
            allco.append((n,grp['inner'],grp['arg'],to,src0))
    for n,inner,arg,to,src0 in sorted(allco,reverse=True)[:20]:
        rel = to-arg
        print(f"  n={n:6d} ctrl=0x{inner:08x} {cname(inner):26s} arg=0x{arg:x} to=0x{to:x} (to-arg={rel:+#x}) src0=0x{src0:x}")
    print(f"\n=== summary: {len(groups)} ioctls, {sum(len(x['cos']) for x in groups)} copyouts ===")

main()
