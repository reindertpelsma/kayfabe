#!/usr/bin/env python3
"""Decode GSP_RM_ALLOC (fn 0x67) and FREE (fn 0xa) elements out of an rpctrace,
reporting the *inner* status field — the one RM reads — per hClass.

decode_rpctrace.py --controls already does this for GSP_RM_CONTROL (fn 0x4c).
It does NOT do it for alloc/free, and alloc/free is where our refusals live
(rpcRmApiAlloc_GSP / rpcRmApiFree_GSP).
"""
import collections
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import decode_rpctrace as D

ELEM = D.ELEM_HDR_SIZE      # 48
RPCH = D.RPC_HDR_SIZE       # 32

ALLOC_FMT = "<4I3I4x"       # hClient hParent hObject hClass status paramsSize flags + reserved[4]
ALLOC_SIZE = struct.calcsize(ALLOC_FMT)     # 32
FREE_FMT = "<4I"            # hRoot hObjectParent hObjectOld status
FREE_SIZE = struct.calcsize(FREE_FMT)


def main(path):
    blob = open(path, "rb").read()
    hdr, recs = D.verify_and_parse(blob)
    allocs, frees = [], []
    for r in recs:
        off = ELEM + RPCH
        b = r["body"]
        if r["rpc_fn"] == 0x67 and len(b) >= off + ALLOC_SIZE:
            hc, hp, ho, hcls, st, psz, flags = struct.unpack_from(ALLOC_FMT, b, off)
            allocs.append(dict(seq=r["seq"], dir=r["dir"], hClass=hcls, status=st,
                               paramsSize=psz, present=len(b) - off - ALLOC_SIZE,
                               hClient=hc, hObject=ho))
        elif r["rpc_fn"] == 0xa and len(b) >= off + FREE_SIZE:
            hr, hpp, hoo, st = struct.unpack_from(FREE_FMT, b, off)
            frees.append(dict(seq=r["seq"], dir=r["dir"], status=st,
                              hRoot=hr, hObject=hoo))

    for name, rows, key in (("GSP_RM_ALLOC (fn 0x67)", allocs, "hClass"),
                            ("FREE (fn 0xa)", frees, None)):
        reps = [x for x in rows if x["dir"] == D.DIR_REP]
        reqs = [x for x in rows if x["dir"] == D.DIR_REQ]
        print("\n=== %s: %d elements (%d req / %d rep) ===" % (name, len(rows), len(reqs), len(reps)))
        nonok = [x for x in reps if x["status"] != 0]
        print("    replies with NON-ZERO inner status: %d" % len(nonok))
        for x in nonok:
            print("      seq %d status=0x%x %s" % (x["seq"], x["status"],
                  ("hClass=0x%08x" % x["hClass"]) if key else ("hObject=0x%08x" % x["hObject"])))
        if key:
            by = collections.defaultdict(lambda: dict(n=0, st=set(), psz=set(), present=set()))
            for x in reps:
                e = by[x["hClass"]]
                e["n"] += 1
                e["st"].add(x["status"])
                e["psz"].add(x["paramsSize"])
                e["present"].add(x["present"])
            print("    %-12s %4s  %-16s %-14s %s" % ("hClass", "n", "reply paramsSize", "bytes present", "status"))
            for k in sorted(by):
                e = by[k]
                print("    0x%08x  %4d  %-16s %-14s %s" % (
                    k, e["n"],
                    ",".join(str(s) for s in sorted(e["psz"])),
                    ",".join(str(s) for s in sorted(e["present"])),
                    ",".join("0x%x" % s for s in sorted(e["st"]))))


if __name__ == "__main__":
    main(sys.argv[1])
