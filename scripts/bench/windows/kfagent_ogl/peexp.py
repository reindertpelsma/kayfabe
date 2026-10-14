# peexp.py <dll> <rva-hex>... : nearest export at or below each RVA
import struct, sys
d = open(sys.argv[1], "rb").read()
pe = struct.unpack_from("<I", d, 0x3c)[0]; nsec = struct.unpack_from("<H", d, pe + 6)[0]; optsz = struct.unpack_from("<H", d, pe + 20)[0]
opt = pe + 24; magic = struct.unpack_from("<H", d, opt)[0]; ddoff = opt + (112 if magic == 0x20b else 96)
exp_rva, exp_sz = struct.unpack_from("<II", d, ddoff)
secs = []
for i in range(nsec):
    o = opt + optsz + 40 * i; vs, va, rs, rp = struct.unpack_from("<IIII", d, o + 8); secs.append((va, max(vs, rs), rp))
def off(r):
    for va, sz, rp in secs:
        if va <= r < va + sz: return r - va + rp
nf, nn, af, anm, ao = struct.unpack_from("<IIIII", d, off(exp_rva) + 20)
funcs = [struct.unpack_from("<I", d, off(af) + 4 * i)[0] for i in range(nf)]
names = {}
for i in range(nn):
    nr = struct.unpack_from("<I", d, off(anm) + 4 * i)[0]; o = struct.unpack_from("<H", d, off(ao) + 2 * i)[0]
    s = d[off(nr):d.index(b"\0", off(nr))].decode(); names.setdefault(funcs[o], s)
ex = sorted((r, n) for r, n in names.items())
for a in sys.argv[2:]:
    r = int(a, 16); best = max((e for e in ex if e[0] <= r), default=None)
    print(a, "->", "%s+%#x" % (best[1], r - best[0]) if best else "?")
