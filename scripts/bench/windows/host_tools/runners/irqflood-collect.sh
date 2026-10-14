#!/bin/bash
# collect.sh N — small evidence files for run N into /var/lib/kf-windows-20261005/irqflood-evidence/runN
N=$1; W=/var/lib/kf-windows-20261005; R=$W/boundary-kayfabe-$N; O=$W/irqflood-evidence/run$N
mkdir -p $O
python3 -I $W/irqflood-analyze.py $R > $O/analysis.txt 2>&1
python3 -I $W/irqflood-tl.py $R/trace.log > $O/per-second.txt 2>&1
cp $R/marker.txt $R/command.json $O/ 2>/dev/null
cp $W/wr-run$N.log $O/wr-run$N.log 2>/dev/null
for f in $R/flip-*.out; do [ -f $f ] && cp $f $O/; done
# qemu.log: flood banner, every distinct status segment every ~60 s, teardown lines, stall snapshots
grep -a "★★ PERTURBING" $R/qemu.log | head -1 > $O/qemu-flood-lines.txt
grep -a "kf3: family" $R/qemu.log | grep -a -o "PERTURBING DIAGNOSTIC ON: irq-flood.*raised\[[^]]*\]" | awk 'NR%15==1' | cut -c1-400 >> $O/qemu-flood-lines.txt
grep -a "kf3: family" $R/qemu.log | grep -a -o "PERTURBING DIAGNOSTIC ON: irq-flood.*raised\[[^]]*\]" | tail -1 | cut -c1-400 >> $O/qemu-flood-lines.txt
grep -a "UnloadingGuestDriver\|birth REFUSED\|PT-SNAP BEGIN stall\|ChannelFreed { kind: Core\|BORN Passthrough" $R/qemu.log | head -40 | cut -c1-260 > $O/qemu-teardown-lines.txt
grep -a "^kf3: family" $R/qemu.log | tail -1 | grep -o "disp\[[^]]*\]" > $O/last-disp-status.txt
# one lock-screen frame and the last frame, as PNG
python3 -I - $R $O <<'PY'
import sys, glob, zlib, struct
R, O = sys.argv[1], sys.argv[2]
ps = sorted(glob.glob(R + "/shots-*/s-*.ppm"))
def ppm(p):
    b = open(p, "rb").read(); i = 0; hdr = []
    for _ in range(3):
        j = b.index(b"\n", i); hdr.append(b[i:j]); i = j + 1
    w, h = map(int, hdr[1].split()); return w, h, b[i:]
def png(w, h, rgb, out, scale=4):
    # downscale by taking every scale-th pixel and row
    W2, H2 = w // scale, h // scale
    raw = bytearray()
    for y in range(H2):
        raw.append(0)
        row = rgb[(y * scale) * w * 3:(y * scale + 1) * w * 3]
        for x in range(W2):
            raw += row[x * scale * 3:x * scale * 3 + 3]
    def ch(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    open(out, "wb").write(b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", struct.pack(">IIBBBBB", W2, H2, 8, 2, 0, 0, 0)) + ch(b"IDAT", zlib.compress(bytes(raw), 9)) + ch(b"IEND", b""))
def frac(p):
    w, h, d = ppm(p); px = d[::997]; return sum(1 for x in px if x > 16) / max(1, len(px))
if ps:
    lock = [p for p in ps if frac(p) > 0.1]
    pick = [("first-lock", lock[0])] if lock else []
    pick += [("last", ps[-1])]
    if len(lock) > 2: pick.append(("mid-lock", lock[len(lock) // 2]))
    for name, p in pick:
        w, h, d = ppm(p); png(w, h, d, "%s/frame-%s.png" % (O, name))
PY
ls -la $O | tail -n +2
