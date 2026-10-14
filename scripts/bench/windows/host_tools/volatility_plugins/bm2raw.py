# Diagnosis helper (host only): Windows summary/kernel bitmap dump -> sparse raw physical image for Volatility.
import struct, sys
src, dst = sys.argv[1], sys.argv[2]
f = open(src, "rb")
h = f.read(0x2000)
assert h[:8] == b"PAGEDU64", h[:8]
f.seek(0x2000)
bh = f.read(0x38)
sig, valid = struct.unpack_from("<4s4s", bh, 0); first, present, bsize = struct.unpack_from("<QQQ", bh, 0x20); pages = present; hsize = first - 0x2000
print("sig", sig, valid, "first", hex(first), "present", present, "bitmap bits", bsize)
nb = (bsize + 7) // 8
bm = f.read(nb)
out = open(dst, "wb")
out.truncate(bsize * 4096)
f.seek(first)
n = 0
for i in range(nb):
    b = bm[i]
    if not b: continue
    for bit in range(8):
        if b >> bit & 1:
            data = f.read(4096)
            if len(data) < 4096: break
            out.seek((i * 8 + bit) * 4096); out.write(data); n += 1
print("pages written", n)
