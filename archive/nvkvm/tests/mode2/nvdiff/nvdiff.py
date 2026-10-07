#!/usr/bin/env python3
"""
nvdiff.py — decode + differ for nvdiff_shim.c JSONL captures.

This is the *analysis half* of the host<->guest differential oracle. It never
re-runs anything; it only reads captures, so any future question can be asked of
an old capture.

Sub-commands
  summary  A.jsonl                 per-opcode census of one capture
  show     A.jsonl [--grep RE]     human-readable decode of records
  replies  A.jsonl                 per-control: does RM actually WRITE a reply body?
  diff     A.jsonl B.jsonl         align the two streams and name the FIRST divergence

Divergence kinds reported by `diff`:
  MISSING   a call present in A (reference) and absent in B
  EXTRA     a call present in B and absent in A
  SIZE      same call, different declared parameter size  (the class the owner
            reports was "often the issue" in the C campaign)
  STATUS    same call, different rc / NV status
  VALUE     same call, same size, differing bytes that canonicalisation cannot
            explain as a handle or an address

Canonicalisation (the noise filter). Two runs of the SAME program on the SAME
host still differ in: RM object handles (allocated in sequence, but the client
base varies), userspace pointers (ASLR), device virtual addresses, and clock
values. We do NOT blanket-mask those. We build, per capture, a first-appearance
symbol table of every handle value and every pointer value seen in a DECODED
field, and then a differing byte-run is "explained" only if the corresponding
values in A and B carry the SAME symbol. Anything else stays UNEXPLAINED and is
reported. `--noise` prints the residual so a noise floor is always stated.

Deliberately NOT done: no sampling, no capping of the record stream, and a
truncated parameter buffer is reported as `trunc`, never treated as short.
"""

import argparse
import io
import difflib
import json
import re
import struct
import sys
from collections import Counter, OrderedDict

# ---------------------------------------------------------------- escape names
# research_clones/ogkm-580.159.04/src/nvidia/arch/nvalloc/unix/include/nv_escape.h
ESC = {
    0x27: "RM_ALLOC_MEMORY", 0x28: "RM_ALLOC_OBJECT", 0x29: "RM_FREE",
    0x2A: "RM_CONTROL", 0x2B: "RM_ALLOC", 0x32: "RM_CONFIG_GET",
    0x33: "RM_CONFIG_SET", 0x34: "RM_DUP_OBJECT", 0x35: "RM_SHARE",
    0x37: "RM_CONFIG_GET_EX", 0x38: "RM_CONFIG_SET_EX", 0x39: "RM_I2C_ACCESS",
    0x41: "RM_IDLE_CHANNELS", 0x4A: "RM_VID_HEAP_CONTROL",
    0x4D: "RM_ACCESS_REGISTRY", 0x4E: "RM_MAP_MEMORY", 0x4F: "RM_UNMAP_MEMORY",
    0x52: "RM_GET_EVENT_DATA", 0x54: "RM_ALLOC_CONTEXT_DMA2",
    0x56: "RM_ADD_VBLANK_CALLBACK", 0x57: "RM_MAP_MEMORY_DMA",
    0x58: "RM_UNMAP_MEMORY_DMA", 0x59: "RM_BIND_CONTEXT_DMA",
    0x5C: "RM_EXPORT_OBJECT_TO_FD", 0x5D: "RM_IMPORT_OBJECT_FROM_FD",
    0x5E: "RM_UPDATE_DEVICE_MAPPING_INFO", 0x5F: "RM_LOCKLESS_DIAGNOSTIC",
    # kernel-open/common/inc/nv-ioctl-numbers.h  (NV_IOCTL_BASE = 200)
    0xC8: "CARD_INFO", 0xC9: "REGISTER_FD", 0xCA: "ALLOC_OS_EVENT_OLD",
    0xCE: "ALLOC_OS_EVENT", 0xCF: "FREE_OS_EVENT", 0xD2: "CHECK_VERSION_STR",
    0xD6: "SYS_PARAMS", 0xD8: "NUMA_INFO", 0xD9: "SET_NUMA_STATUS",
    0xDA: "EXPORT_TO_DMABUF_FD", 0xDB: "WAIT_OPEN_COMPLETE",
}

U32 = "I"
U64 = "Q"
S32 = "i"

# name, offset, format. Layouts from ogkm-580.159.04 src/common/sdk/nvidia/inc/nvos.h
LAYOUT = {
    0x29: [("hRoot", 0, U32, "h"), ("hObjectParent", 4, U32, "h"),
           ("hObjectOld", 8, U32, "h"), ("status", 12, U32, "s")],           # NVOS00 :166
    0x2A: [("hClient", 0, U32, "h"), ("hObject", 4, U32, "h"),
           ("cmd", 8, U32, "c"), ("flags", 12, U32, ""),
           ("params", 16, U64, "p"), ("paramsSize", 24, U32, "z"),
           ("status", 28, U32, "s")],                                        # NVOS54 :2238
    0x34: [("hClient", 0, U32, "h"), ("hParent", 4, U32, "h"),
           ("hObject", 8, U32, "h"), ("hClientSrc", 12, U32, "h"),
           ("hObjectSrc", 16, U32, "h"), ("flags", 20, U32, ""),
           ("status", 24, U32, "s")],                                        # NVOS55 :2273
    0x4E: [("hClient", 0, U32, "h"), ("hDevice", 4, U32, "h"),
           ("hMemory", 8, U32, "h"), ("offset", 16, U64, ""),
           ("length", 24, U64, ""), ("pLinearAddress", 32, U64, "p"),
           ("status", 40, U32, "s"), ("flags", 44, U32, "")],                # NVOS33 :1854
    0x27: [("hRoot", 0, U32, "h"), ("hObjectParent", 4, U32, "h"),
           ("hObjectNew", 8, U32, "h"), ("hClass", 12, U32, "k"),
           ("flags", 16, U32, ""), ("pMemory", 24, U64, "p"),
           ("limit", 32, U64, ""), ("status", 40, U32, "s")],                # NVOS02 :293
    0x4A: [("hRoot", 0, U32, "h"), ("hObjectParent", 4, U32, "h"),
           ("function", 8, U32, "f"), ("hVASpace", 12, U32, "h"),
           ("ivcHeapNumber", 16, "h", ""), ("status", 20, U32, "s"),
           ("total", 24, U64, ""), ("free", 32, U64, "")],                   # NVOS32 :662
}
# NVOS21 (32-byte ioctl) vs NVOS64 (48-byte ioctl) for RM_ALLOC -- nvos.h:471 / :488
ALLOC21 = [("hRoot", 0, U32, "h"), ("hObjectParent", 4, U32, "h"),
           ("hObjectNew", 8, U32, "h"), ("hClass", 12, U32, "k"),
           ("pAllocParms", 16, U64, "p"), ("paramsSize", 24, U32, "z"),
           ("status", 28, U32, "s")]
ALLOC64 = [("hRoot", 0, U32, "h"), ("hObjectParent", 4, U32, "h"),
           ("hObjectNew", 8, U32, "h"), ("hClass", 12, U32, "k"),
           ("pAllocParms", 16, U64, "p"), ("pRightsRequested", 24, U64, "p"),
           ("paramsSize", 32, U32, "z"), ("flags", 36, U32, ""),
           ("status", 40, U32, "s")]


def unhex(s):
    return bytes.fromhex(s) if s else b""


def layout_for(rec):
    if rec.get("dev", "").startswith("nvidia-uvm"):
        return None            # UVM params are per-ioctl structs; compared as bytes
    nr = rec.get("nr")
    if nr == 0x2B:
        return ALLOC64 if rec.get("iocsize", 0) >= 48 else ALLOC21
    return LAYOUT.get(nr)


def decode(rec):
    """Attach .esc, .fields_pre, .fields_post, .op to an ioctl record."""
    if rec.get("t") != "ioctl":
        rec["op"] = "%s:%s:len=%s" % (rec.get("t"), rec.get("dev"), rec.get("len"))
        return rec
    nr = rec["nr"]
    if rec.get("uvm"):
        rec["esc"] = "UVM_" + rec["uvm"]
    elif rec.get("dev", "").startswith("nvidia-uvm"):
        rec["esc"] = "uvm_nr_0x%02x" % nr
    else:
        rec["esc"] = ESC.get(nr, "nr_0x%02x" % nr)
    lay = layout_for(rec)
    for side in ("pre", "post"):
        buf = unhex(rec.get("h" + side, ""))
        f = OrderedDict()
        if lay:
            for name, off, fmt, kind in lay:
                sz = struct.calcsize("<" + fmt)
                if off + sz <= len(buf):
                    f[name] = struct.unpack_from("<" + fmt, buf, off)[0]
        else:
            for i in range(0, len(buf) - 3, 4):
                f["w%d" % (i // 4)] = struct.unpack_from("<I", buf, i)[0]
        rec["f_" + side] = f
    disc = ""
    fp = rec["f_pre"]
    if nr == 0x2A and "cmd" in fp:
        disc = ":cmd=0x%08x" % fp["cmd"]
    elif nr in (0x2B, 0x27) and "hClass" in fp:
        disc = ":cls=0x%04x" % fp["hClass"]
    elif nr == 0x4A and "function" in fp:
        disc = ":fn=%d" % fp["function"]
    rec["op"] = "%s:%s%s" % (rec["dev"], rec["esc"], disc)
    return rec


def _open_text(path):
    """Read .jsonl, .jsonl.gz or .jsonl.zst -- committed references are zstd."""
    if path.endswith(".gz"):
        import gzip
        return gzip.open(path, "rt")
    if path.endswith(".zst"):
        try:
            import zstandard  # optional
            return io.TextIOWrapper(zstandard.ZstdDecompressor().stream_reader(open(path, "rb")))
        except ImportError:
            import subprocess
            p = subprocess.run(["zstd", "-dc", path], stdout=subprocess.PIPE, check=True)
            return io.StringIO(p.stdout.decode())
    return open(path)


def load(path):
    out = []
    with _open_text(path) as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                out.append(decode(json.loads(line)))
            except json.JSONDecodeError:
                sys.stderr.write("WARN: unparsable record dropped\n")
    return out


# ------------------------------------------------------------ canonicalisation
class Symbols(object):
    """First-appearance symbol tables for handles and pointers of ONE capture."""

    def __init__(self, recs):
        self.h = OrderedDict()   # handle value -> symbol
        self.p = OrderedDict()   # pointer value -> symbol
        for r in recs:
            if r.get("t") != "ioctl":
                continue
            lay = layout_for(r)
            if not lay:
                continue
            for side in ("f_pre", "f_post"):
                for name, off, fmt, kind in lay:
                    v = r[side].get(name)
                    if v is None or v == 0:
                        continue
                    if kind == "h":
                        self.h.setdefault(v, "H%d" % len(self.h))
                    elif kind == "p":
                        self.p.setdefault(v, "P%d" % len(self.p))
            if r.get("pptr"):
                v = int(r["pptr"], 16)
                if v:
                    self.p.setdefault(v, "P%d" % len(self.p))

    def hsym(self, v):
        return self.h.get(v)

    def psym(self, v):
        return self.p.get(v)


def looks_like_ptr(v):
    return 0x1000 <= v < (1 << 48)


def explain(off, a, b, sa, sb, span):
    """Classify one differing byte-run. Returns (kind, detail).

    ★ The candidate windows are NOT assumed aligned. Several NVIDIA parameter
    structs are packed, so a differing pointer shows up as a 4-byte run at an odd
    offset (its high half); an alignment-only rule scores that as UNEXPLAINED and
    the noise floor never converges. We therefore test every 8-byte and 4-byte
    window that COVERS the differing run.
    """
    end = off + span
    for w, fmt, sym, kind in ((8, "<Q", "p", "ptr"), (4, "<I", "h", "handle")):
        lo = max(0, end - w)
        for s in range(lo, off + 1):
            if s + w > len(a) or s + w > len(b):
                continue
            va = struct.unpack_from(fmt, a, s)[0]
            vb = struct.unpack_from(fmt, b, s)[0]
            if va == vb:
                continue
            if sym == "p":
                pa, pb = sa.psym(va), sb.psym(vb)
                if pa is not None and pa == pb:
                    return "ptr", "%s (0x%x vs 0x%x)" % (pa, va, vb)
            else:
                ha, hb = sa.hsym(va), sb.hsym(vb)
                if ha is not None and ha == hb:
                    return "handle", "%s (0x%x vs 0x%x)" % (ha, va, vb)
    # last resort: two plausible 64-bit userspace/device addresses in the same slot
    for s in range(max(0, end - 8), off + 1):
        if s + 8 > len(a) or s + 8 > len(b):
            continue
        va = struct.unpack_from("<Q", a, s)[0]
        vb = struct.unpack_from("<Q", b, s)[0]
        if va != vb and looks_like_ptr(va) and looks_like_ptr(vb):
            return "addrlike", "0x%x vs 0x%x @%d" % (va, vb, s)
    return "UNEXPLAINED", "%s vs %s" % (a[off:off + span].hex(), b[off:off + span].hex())


def runs(a, b):
    """Yield (offset, length) of maximal differing byte runs over min length."""
    n = min(len(a), len(b))
    i = 0
    while i < n:
        if a[i] != b[i]:
            j = i
            while j < n and a[j] != b[j]:
                j += 1
            yield i, j - i
            i = j
        else:
            i += 1


def field_at(rec, off):
    lay = layout_for(rec)
    if not lay:
        return None
    best = None
    for name, o, fmt, kind in lay:
        sz = struct.calcsize("<" + fmt)
        if o <= off < o + sz:
            best = name
    return best


def cmp_buf(ra, rb, key, sa, sb, label, out):
    a, b = unhex(ra.get(key, "")), unhex(rb.get(key, ""))
    if len(a) != len(b):
        out.append(("SIZE", "%s length %d vs %d" % (label, len(a), len(b))))
        return
    for off, span in runs(a, b):
        kind, detail = explain(off, a, b, sa, sb, span)
        fname = field_at(ra, off) if label.startswith("hdr") else None
        where = "%s@0x%x%s" % (label, off, ("(%s)" % fname) if fname else "")
        out.append((kind, "%s %s" % (where, detail)))


def diff(A, B, args):
    sa, sb = Symbols(A), Symbols(B)
    ka = [r["op"] for r in A]
    kb = [r["op"] for r in B]
    sm = difflib.SequenceMatcher(a=ka, b=kb, autojunk=False)
    divergences = []
    noise = Counter()
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            for k in range(i2 - i1):
                ra, rb = A[i1 + k], B[j1 + k]
                if ra.get("t") != "ioctl":
                    continue
                local = []
                if ra.get("psize") != rb.get("psize"):
                    local.append(("SIZE", "declared paramsSize %s vs %s"
                                  % (ra.get("psize"), rb.get("psize"))))
                if ra.get("rc") != rb.get("rc"):
                    local.append(("STATUS", "rc %s vs %s" % (ra.get("rc"), rb.get("rc"))))
                spre = ra["f_post"].get("status")
                spost = rb["f_post"].get("status")
                if spre != spost:
                    local.append(("STATUS", "NV status 0x%x vs 0x%x" % (spre or 0, spost or 0)))
                cmp_buf(ra, rb, "hpre", sa, sb, "hdr_pre", local)
                cmp_buf(ra, rb, "hpost", sa, sb, "hdr_post", local)
                if not args.no_params:
                    cmp_buf(ra, rb, "ppre", sa, sb, "params_pre", local)
                    cmp_buf(ra, rb, "ppost", sa, sb, "params_post", local)
                for kind, detail in local:
                    noise[kind] += 1
                    if kind in ("UNEXPLAINED", "SIZE", "STATUS") or args.strict:
                        divergences.append((i1 + k, j1 + k, ra["op"], kind, detail))
        elif tag == "delete":
            for k in range(i1, i2):
                noise["MISSING"] += 1
                divergences.append((k, j1, A[k]["op"], "MISSING", "absent in B"))
        elif tag == "insert":
            for k in range(j1, j2):
                noise["EXTRA"] += 1
                divergences.append((i1, k, B[k]["op"], "EXTRA", "absent in A"))
        elif tag == "replace":
            for k in range(i1, i2):
                noise["MISSING"] += 1
                divergences.append((k, j1, A[k]["op"], "MISSING", "replaced region"))
            for k in range(j1, j2):
                noise["EXTRA"] += 1
                divergences.append((i1, k, B[k]["op"], "EXTRA", "replaced region"))

    divergences.sort(key=lambda d: (d[0], d[1]))
    print("records: A=%d B=%d   opcode-aligned ratio=%.6f" % (len(A), len(B), sm.ratio()))
    print("symbols: A handles=%d ptrs=%d   B handles=%d ptrs=%d"
          % (len(sa.h), len(sa.p), len(sb.h), len(sb.p)))
    print("classification census: %s" % dict(noise))
    if not divergences:
        print("RESULT: NO DIVERGENCE (after canonicalisation)")
        return 0
    print("RESULT: %d divergence(s). FIRST %d:" % (len(divergences), args.n))
    for d in divergences[:args.n]:
        ia, ib, op, kind, detail = d
        print("  A[%d] B[%d] %-52s %-12s %s" % (ia, ib, op, kind, detail))
    return 1


def summary(recs):
    c = Counter(r["op"] for r in recs)
    fails = [r for r in recs if r.get("t") == "ioctl"
             and (r.get("rc", 0) != 0 or r.get("f_post", {}).get("status", 0) not in (0, None))]
    print("records=%d  distinct ops=%d  nonzero rc/status=%d" % (len(recs), len(c), len(fails)))
    trunc = [r for r in recs if r.get("trunc")]
    if trunc:
        print("!! %d records TRUNCATED (raise NVDIFF_MAXBUF): %s"
              % (len(trunc), Counter(r["op"] for r in trunc).most_common(5)))
    for op, n in c.most_common():
        print("  %6d  %s" % (n, op))
    if fails:
        print("-- nonzero status --")
        for r in fails[:60]:
            print("  i=%d %-50s rc=%s status=0x%x" % (
                r["i"], r["op"], r.get("rc"), r.get("f_post", {}).get("status", 0) or 0))


def replies(recs):
    """Which controls actually WRITE a reply body, and how many bytes move.

    ★ This is the query that answers the C oracle's FIFTH LIMIT directly. The
    captured C control table has 11 of 56 rows at dlen=0 -- reply body never
    captured -- and every one checked against real hardware was CONTRADICTED
    (CLAUDE.md). Here the host is asked the same question with the body present
    on BOTH sides of the call, so "wrote nothing" is a MEASUREMENT and not a
    missing capture: `changed=0` with `psize=N` means RM was handed N bytes and
    returned them unaltered; `psize=0` means there was no body to write.
    ⊘ An empty capture is evidence of nothing. An unchanged captured body is
    evidence of something.
    """
    agg = OrderedDict()
    for r in recs:
        if r.get("t") != "ioctl" or r.get("nr") not in (0x2A, 0x2B):
            continue
        if "ppre" not in r:
            continue
        a, b = unhex(r["ppre"]), unhex(r["ppost"])
        n = min(len(a), len(b))
        changed = sum(1 for i in range(n) if a[i] != b[i])
        first = next((i for i in range(n) if a[i] != b[i]), None)
        key = r["op"]
        e = agg.setdefault(key, {"calls": 0, "psize": r.get("psize"), "got": len(a),
                                 "changed": 0, "first": first, "trunc": 0, "bad": 0})
        e["calls"] += 1
        e["changed"] = max(e["changed"], changed)
        if r.get("trunc"):
            e["trunc"] += 1
        if r.get("rc", 0) != 0 or (r.get("f_post", {}).get("status") or 0) != 0:
            e["bad"] += 1
        if first is not None and (e["first"] is None or first < e["first"]):
            e["first"] = first
    print("%-46s %5s %7s %6s %8s %7s %s" %
          ("op", "calls", "psize", "got", "changed", "first", "flags"))
    for op, e in agg.items():
        flags = []
        if e["trunc"]:
            flags.append("TRUNC=%d(UNMEASURED TAIL)" % e["trunc"])
        if e["bad"]:
            flags.append("nonzero-status=%d" % e["bad"])
        if e["psize"] and e["got"] < e["psize"]:
            flags.append("SHORT-READ")
        if e["psize"] and not e["changed"] and not e["trunc"]:
            flags.append("pure-IN (RM wrote nothing)")
        print("%-46s %5d %7s %6d %8d %7s %s" %
              (op, e["calls"], e["psize"], e["got"], e["changed"],
               "-" if e["first"] is None else e["first"], " ".join(flags)))


def show(recs, pat, limit):
    rx = re.compile(pat) if pat else None
    n = 0
    for r in recs:
        if rx and not rx.search(r["op"]):
            continue
        n += 1
        if n > limit:
            break
        if r.get("t") != "ioctl":
            print("[%d] %s" % (r["i"], r["op"]))
            continue
        print("[%d] %s rc=%s psize=%s pgot=%s" % (r["i"], r["op"], r["rc"],
                                                  r.get("psize"), r.get("pgot")))
        for side in ("pre", "post"):
            f = r["f_" + side]
            print("     %-4s %s" % (side, " ".join("%s=0x%x" % (k, v) for k, v in f.items())))
        if r.get("ppre"):
            print("     ppre  %s" % r["ppre"][:256])
            print("     ppost %s" % r["ppost"][:256])


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("summary"); s.add_argument("a")
    s = sub.add_parser("show"); s.add_argument("a"); s.add_argument("--grep")
    s.add_argument("--limit", type=int, default=40)
    s = sub.add_parser("replies"); s.add_argument("a")
    s = sub.add_parser("diff"); s.add_argument("a"); s.add_argument("b")
    s.add_argument("-n", type=int, default=25)
    s.add_argument("--strict", action="store_true",
                   help="also report handle/addr-explained differences")
    s.add_argument("--no-params", action="store_true",
                   help="headers only; skip out-of-line parameter bodies")
    args = ap.parse_args()

    if args.cmd == "summary":
        summary(load(args.a)); return 0
    if args.cmd == "show":
        show(load(args.a), args.grep, args.limit); return 0
    if args.cmd == "replies":
        replies(load(args.a)); return 0
    return diff(load(args.a), load(args.b), args)


if __name__ == "__main__":
    sys.exit(main())
