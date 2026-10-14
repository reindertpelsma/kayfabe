#!/usr/bin/env python3
"""analyze.py RUNDIR [LOCK_THRESHOLD] -- one-page summary of an irq-flood boot (run on the bench host).

Reads RUNDIR = /var/lib/kf-windows-20261005/boundary-kayfabe-N: shots-*/s-HHMMSS.mmm.ppm (non-black fraction as
nonblack.py: every 997th byte after the PPM header, share above 16), trace.log (MSI per UTC second, as tl.py),
qemu.log (flood banner and last status segment, teardown lines), marker.txt, flip-*.out (QGA probes).
Prints; nothing is written. Run with python3 -I.
"""
import collections
import glob
import os
import re
import sys

d = sys.argv[1].rstrip("/")
lock_thr = float(sys.argv[2]) if len(sys.argv) > 2 else 0.1


def nonblack(p):
    b = open(p, "rb").read()
    i = 0
    for _ in range(3):
        i = b.index(b"\n", i) + 1
    px = b[i::997]
    return sum(1 for x in px if x > 16) / max(1, len(px))


print("== run", os.path.basename(d))
print("-- marker.txt")
try:
    print(open(d + "/marker.txt", errors="replace").read().strip())
except OSError as e:
    print("(none)", e)

print("-- screenshots (UTC HHMMSS.mmm : non-black fraction); lock screen = fraction >", lock_thr)
rows = []
for p in sorted(glob.glob(d + "/shots-*/s-*.ppm")):
    try:
        rows.append((os.path.basename(p)[2:-4], nonblack(p)))
    except (ValueError, OSError):
        pass
if rows:
    print("first shot", rows[0][0], "last shot", rows[-1][0], "n =", len(rows))
    lock = [r for r in rows if r[1] > lock_thr]
    anyc = [r for r in rows if r[1] > 0.005]
    print("lock frames:", len(lock), "first", lock[0][0] if lock else None, "last", lock[-1][0] if lock else None)
    print("any non-black (>0.005):", len(anyc), "first", anyc[0][0] if anyc else None, "last", anyc[-1][0] if anyc else None)
    print(" ".join("%s=%.3f" % r for r in rows if r[1] > 0.005))

print("-- MSI per UTC second (trace.log)")
R = re.compile(r"^(\S+) vfio_msi_interrupt")
per = collections.Counter()
try:
    for l in open(d + "/trace.log", errors="replace"):
        m = R.match(l)
        if m:
            per[m.group(1)[11:19]] += 1
    ks = sorted(per)
    print("total", sum(per.values()), "seconds with MSI", len(ks))
    print(" ".join("%s=%d" % (k, per[k]) for k in ks))
except OSError as e:
    print("(no trace.log)", e)

print("-- qemu.log")
q = open(d + "/qemu.log", errors="replace").read().splitlines()
for l in q:
    if "PERTURBING DIAGNOSTIC ON" in l:
        print("banner:", l[:300])
        break
last = [l for l in q if "PERTURBING DIAGNOSTIC ON: irq-flood" in l and l.startswith("kf3: family")]
if last:
    m = re.search(r"PERTURBING DIAGNOSTIC ON: irq-flood .*?raised\[[^\]]*\]", last[-1])
    print("last status segment:", m.group(0) if m else last[-1][-300:])
else:
    print("no status segment (flood off or no status line printed)")
for pat in ("UnloadingGuestDriver", "birth REFUSED", "stall=", "PT-STALL", "ChannelFreed { kind: Core"):
    ls = [l for l in q if pat in l]
    print("%-28s %d %s" % (pat, len(ls), ls[0][:160] if ls else ""))
print("-- QGA probes")
for p in sorted(glob.glob(d + "/flip-*.out")):
    t = open(p, errors="replace").read().strip().splitlines()
    print(os.path.basename(p), "lines", len(t), "|", " / ".join(x[:100] for x in t[-3:]))
