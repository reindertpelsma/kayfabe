# mdthreads.py <dump> [nthreads] : per-thread CPU time, RIP and stack return-address scan (module+offset only)
import struct, sys
d = open(sys.argv[1], "rb").read(); top = int(sys.argv[2]) if len(sys.argv) > 2 else 8
sig, ver, ns, dirrva = struct.unpack_from("<IIII", d, 0)
st = {}
for i in range(ns):
    t, sz, rva = struct.unpack_from("<III", d, dirrva + 12 * i); st.setdefault(t, []).append((sz, rva))
mods = []
sz, rva = st[4][0]; n = struct.unpack_from("<I", d, rva)[0]
for i in range(n):
    o = rva + 4 + 108 * i
    base, size, _, ts, namerva = struct.unpack_from("<QIIII", d, o)
    l = struct.unpack_from("<I", d, namerva)[0]; mods.append((base, size, d[namerva + 4:namerva + 4 + l].decode("utf-16le").split("\\")[-1]))
def modof(a):
    for b, s, nm in mods:
        if b <= a < b + s: return "%s+%#x" % (nm, a - b)
    return None
info = {}
if 17 in st:
    sz, rva = st[17][0]; hs, es, ne = struct.unpack_from("<III", d, rva)
    for i in range(ne):
        tid, fl, err, ex, ct, et, kt, ut, sa, af = struct.unpack_from("<IIIIQQQQQQ", d, rva + hs + es * i)
        info[tid] = (kt / 1e7, ut / 1e7, modof(sa))
sz, rva = st[3][0]; n = struct.unpack_from("<I", d, rva)[0]
th = []
for i in range(n):
    o = rva + 4 + 48 * i
    tid, susp, pc, pr, teb, sstart, ssz, srva, csz, crva = struct.unpack_from("<IIIIQQIIII", d, o)
    rip, rsp = struct.unpack_from("<Q", d, crva + 0xF8)[0], struct.unpack_from("<Q", d, crva + 0x98)[0]
    th.append((tid, sstart, ssz, srva, rip, rsp))
th.sort(key=lambda t: -(sum(info.get(t[0], (0, 0, None))[:2])))
for tid, sstart, ssz, srva, rip, rsp in th[:top]:
    k, u, sa = info.get(tid, (0, 0, None))
    print("tid %d kernel=%.1fs user=%.1fs start=%s rip=%s" % (tid, k, u, sa, modof(rip) or hex(rip)))
    hits = []
    off = max(0, rsp - sstart)
    for j in range(off, ssz - 7, 8):
        v = struct.unpack_from("<Q", d, srva + j)[0]; m = modof(v)
        if m and not m.startswith(("jvm.dll",)) or (m and len(hits) < 3):
            hits.append(m)
        if len(hits) >= 14: break
    print("   ", " | ".join(hits))
