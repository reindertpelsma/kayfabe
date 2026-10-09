# usage: python3 -I kernel_table_dump.py GSP.jsonl [...]   (the INTERNAL_INTR_GET_KERNEL_TABLE reply, row by row)
import json, struct, sys


def dec(path):
    out = []
    for line in open(path, errors="replace"):
        try:
            r = json.loads(line)
        except Exception:
            continue
        if r.get("kind") != "record":
            continue
        p = bytes.fromhex(r["payload_hex"])
        i = p.find(b"VRPC")
        if i < 4:
            continue
        hv, sig, ln, fn, res, resp, sq, sp = struct.unpack_from("<8I", p, i - 4)
        if fn != 76:
            continue
        b = p[i - 4 + 32:]
        if len(b) < 20:
            continue
        hc, ho, cmd = struct.unpack_from("<3I", b, 0)
        if cmd != 0x20800a5c or r["direction"] != 1:
            continue
        psz = struct.unpack_from("<I", b, 16)[0]
        out.append((hc, ho, psz, b[40:40 + psz]))
    return out


for path in sys.argv[1:]:
    rs = dec(path)
    print("==", path, "replies:", len(rs))
    if not rs:
        continue
    hc, ho, psz, pl = rs[0]
    n = struct.unpack_from("<I", pl, 0)[0]
    print(" psz", psz, "tableLen", n)
    for k in range(min(n, 128)):
        o = 4 + k * 16
        if o + 16 > len(pl):
            break
        eng, = struct.unpack_from("<H", pl, o)
        pmc, vs, vn = struct.unpack_from("<3I", pl, o + 4)
        print("  eng=%4d pmc=%#010x stall=%-6d nonstall=%-6d" % (eng, pmc, vs if vs != 0xffffffff else -1, vn if vn != 0xffffffff else -1))
