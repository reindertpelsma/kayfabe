#!/usr/bin/env python3
"""pngdiff.py capA.png capB.png capC.png — nvkvm-pv's client-window differential
(tests/perf/verify_client_window_differential.sh, the embedded python, verbatim decoder and the same
per-channel >8 threshold on a 2-pixel sampling grid). Prints AB_DIFF <n> and BC_DIFF <n>:
A (no client) vs B (client) non-zero ⇒ the client's window contributed pixels;
B vs C (1 s later) non-zero ⇒ the client is rendering new frames, not one stale buffer."""
import os, struct, sys, zlib

def load(p):
    d = open(p, 'rb').read()
    pos, idat, w, h, ctype = 8, b'', 0, 0, 0
    while pos < len(d):
        ln = struct.unpack('>I', d[pos:pos+4])[0]; typ = d[pos+4:pos+8]
        if typ == b'IHDR': w, h, _, ctype = struct.unpack('>IIBB', d[pos+8:pos+18])
        elif typ == b'IDAT': idat += d[pos+8:pos+8+ln]
        pos += 12 + ln
    raw = zlib.decompress(idat)
    nch = {0: 1, 2: 3, 4: 2, 6: 4}[ctype]; stride = w * nch
    out = bytearray(); prev = bytearray(stride); i = 0
    for y in range(h):
        f = raw[i]; i += 1
        line = bytearray(raw[i:i+stride]); i += stride
        for x in range(stride):
            a = line[x-nch] if x >= nch else 0
            b = prev[x]; c = prev[x-nch] if x >= nch else 0
            if f == 1: line[x] = (line[x] + a) & 255
            elif f == 2: line[x] = (line[x] + b) & 255
            elif f == 3: line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                pp = a + b - c; pa, pb, pc = abs(pp - a), abs(pp - b), abs(pp - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        out += line; prev = line
    return w, h, nch, bytes(out)

def diff(p, q, tag):
    if not (os.path.exists(p) and os.path.exists(q)):
        print(f"{tag} MISSING"); return
    w, h, n, A = load(p); _, _, _, B = load(q)
    if len(A) != len(B):
        print(f"{tag} SIZE_MISMATCH"); return
    nd = 0; stride = w * n
    for y in range(0, h, 2):
        row = y * stride
        for x in range(0, w, 2):
            o = row + x * n
            if abs(A[o]-B[o]) > 8 or abs(A[o+1]-B[o+1]) > 8 or abs(A[o+2]-B[o+2]) > 8:
                nd += 1
    print(f"{tag} {nd}")
    print(f"# {tag}: {nd}/{(w//2)*(h//2)} sampled px differ")

if __name__ == "__main__":   # (imgnoise.py imports load())
    diff(sys.argv[1], sys.argv[2], "AB_DIFF")
    diff(sys.argv[2], sys.argv[3], "BC_DIFF")
