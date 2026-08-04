#!/usr/bin/env python3
"""ctrl_payload_pairs.py — pair every GSP_RM_CONTROL request with its reply and
report what the PAYLOAD BYTES say about direction and about replayability.

Task #180, `replay-conformance`.  This is the *measured* half of the DATA-vs-ACT
classification; the static half is `scripts/rm_ctrl_index.py` + reading `ogkm`.

Two questions it answers, and one it deliberately does not.

1. **out_vs_in** — is the reply payload byte-identical to the request payload?
   `DIFF` ⇒ the reply carries bytes the request did not: the control HAS `[out]`
   fields, so it is not a pure ACT.  That direction is sound.
   ⊘ `SAME` is NOT evidence of ACT.  A pure-`[out]` field whose value happens to
   be the zero the caller passed in reads as `SAME`: measured here on
   `0x0073010c` `SYSTEM_GET_ACTIVE`, a documented `[out] displayId` that answers
   0 ("no display is active") for every head.  `SAME` means "this call adds
   nothing", never "this control returns nothing".

2. **replayable** — for each DISTINCT request payload, is the reply always the
   same bytes?  `rpctrace_ga106_boot1` contains TWO complete, independent GSP
   bring-ups (`decode_rpctrace.split_sessions`), so a control answered
   identically for identical arguments in both is self-contained *as far as this
   capture can show*; one that varies references per-boot state (handles,
   addresses, counters) and cannot be served from a fixed table.
   ⊘ `n=1` proves nothing either way and is reported as `ONCE`.

⊘ What it cannot answer: whether a control that returns data ALSO does something.
That is MIXED, and only the driver source decides it.

  ctrl_payload_pairs.py TRACE.bin [--ids IDFILE] [--dump 0xCMD]
"""
import argparse, collections, importlib.util, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("dr", os.path.join(HERE, "decode_rpctrace.py"))
dr = importlib.util.module_from_spec(spec); spec.loader.exec_module(dr)


def pair(trace):
    h, recs = dr.verify_and_parse(open(trace, "rb").read())
    sess = dr.split_sessions(recs)
    sid = {}
    for i, s in enumerate(sess):
        for r in s:
            sid[id(r)] = i
    ctrls = dr.decode_controls(recs, {})
    out, pend = [], None
    for c in ctrls:
        if c["dir"] == dr.DIR_REQ:
            pend = c
        else:
            out.append((pend, c))
            pend = None
    for r in recs:
        pass
    # attach session index by seq
    bounds = [(s[0]["seq"], s[-1]["seq"]) for s in sess]
    def sess_of(seq):
        for i, (a, b) in enumerate(bounds):
            if a <= seq <= b:
                return i
        return -1
    return [(a, b, sess_of(b["seq"])) for a, b in out], len(sess)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("trace")
    ap.add_argument("--ids", help="file of 0x… ids, one per line; default all")
    ap.add_argument("--dump", type=lambda s: int(s, 16), help="hexdump every pair for one cmd")
    a = ap.parse_args()

    pairs, nsess = pair(a.trace)
    by = collections.defaultdict(list)
    for req, rep, s in pairs:
        by[rep["cmd"]].append((req, rep, s))

    if a.dump is not None:
        for req, rep, s in by.get(a.dump, []):
            ra = bytes(req["params"]) if req else b""
            rb = bytes(rep["params"])
            print("seq%-6d sess%d obj=0x%08x status=0x%-3x" % (rep["seq"], s, rep["h_object"], rep["status"]))
            print("   REQ %s" % ra.hex())
            print("   REP %s" % rb.hex())
        return

    ids = [int(l, 16) for l in open(a.ids)] if a.ids else sorted(by)
    print("# trace=%s sessions=%d  (each session is a COMPLETE GSP bring-up)" % (a.trace, nsess))
    print("%-12s %4s %7s %-9s %-12s %-8s %s" %
          ("cmd", "n", "psize", "out_vs_in", "replayable", "sessions", "status"))
    for cid in ids:
        lst = by.get(cid, [])
        if not lst:
            print("%-12s  (not demanded)" % ("0x%08x" % cid))
            continue
        diffs = set()
        keyed = collections.defaultdict(set)
        for req, rep, s in lst:
            ra = bytes(req["params"]) if req else None
            rb = bytes(rep["params"])
            if ra is not None:
                diffs.add("SAME" if ra == rb else "DIFF")
                keyed[ra].add((rb, rep["status"]))
        if len(lst) == 1:
            rep_v = "ONCE"
        elif all(len(v) == 1 for v in keyed.values()):
            rep_v = "STABLE" if len(keyed) == 1 else "STABLE/KEYED"
        else:
            rep_v = "VARIES"
        st = ",".join("0x%x" % x for x in sorted(set(r["status"] for _, r, _ in lst)))
        ss = ",".join(str(x) for x in sorted(set(s for _, _, s in lst)))
        print("%-12s %4d %7d %-9s %-12s %-8s %s" %
              ("0x%08x" % cid, len(lst), lst[0][1]["params_size"],
               "/".join(sorted(diffs)), rep_v, ss, st))


if __name__ == "__main__":
    main()
