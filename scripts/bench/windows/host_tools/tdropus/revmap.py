#!/usr/bin/env python3
"""revmap.py QPID CR3 PA [LOWMEM] — every kernel VA that maps guest-physical page PA, by walking the guest's
4-level page tables read from QEMU's shared guest-RAM memfd (no VM stop). Diagnosis tool, host-only.
Guest RAM layout (pc machine): GPA [0, LOWMEM) at memfd offset GPA; GPA [4G, ...) at offset GPA - 4G + LOWMEM."""
import os, struct, sys

def ram_fd(qpid):
    best = None
    for fd in os.listdir(f"/proc/{qpid}/fd"):
        try:
            t = os.readlink(f"/proc/{qpid}/fd/{fd}")
        except OSError:
            continue
        if t.startswith("/memfd:") and "kf3-" not in t and "display" not in t:
            p = f"/proc/{qpid}/fd/{fd}"
            sz = os.stat(p).st_size
            if best is None or sz > best[1]:
                best = (p, sz)
    if best is None:
        sys.exit("no guest-RAM memfd")
    return os.open(best[0], os.O_RDONLY)

def main():
    qpid, cr3, pa = int(sys.argv[1]), int(sys.argv[2], 0), int(sys.argv[3], 0)
    lowmem = int(sys.argv[4], 0) if len(sys.argv) > 4 else 0xC0000000
    fd = ram_fd(qpid)
    M = 0x000FFFFFFFFFF000
    def off(g):
        return g if g < lowmem else (g - (1 << 32) + lowmem if g >= (1 << 32) else None)
    def table(g):
        o = off(g)
        if o is None:
            return None
        b = os.pread(fd, 4096, o)
        return struct.unpack("<512Q", b) if len(b) == 4096 else None
    target = pa & ~0xFFF
    root = cr3 & M
    l4 = table(root)
    hits = []
    def canon(v):
        return v | 0xFFFF000000000000 if v & (1 << 47) else v
    for i4 in range(256, 512):
        e4 = l4[i4]
        if not e4 & 1 or (e4 & M) == root:
            continue  # absent, or the self-map
        l3 = table(e4 & M)
        if l3 is None:
            continue
        for i3, e3 in enumerate(l3):
            if not e3 & 1:
                continue
            va3 = (i4 << 39) | (i3 << 30)
            if e3 & 0x80:
                b = e3 & 0x000FFFFFC0000000
                if b <= target < b + (1 << 30):
                    hits.append(canon(va3 + (target - b)))
                continue
            l2 = table(e3 & M)
            if l2 is None:
                continue
            for i2, e2 in enumerate(l2):
                if not e2 & 1:
                    continue
                va2 = va3 | (i2 << 21)
                if e2 & 0x80:
                    b = e2 & 0x000FFFFFFFE00000
                    if b <= target < b + (1 << 21):
                        hits.append(canon(va2 + (target - b)))
                    continue
                l1 = table(e2 & M)
                if l1 is None:
                    continue
                for i1, e1 in enumerate(l1):
                    if e1 & 1 and (e1 & M) == target:
                        hits.append(canon(va2 | (i1 << 12)))
    for h in hits:
        print(f"{h:#x}")

if __name__ == "__main__":
    main()
