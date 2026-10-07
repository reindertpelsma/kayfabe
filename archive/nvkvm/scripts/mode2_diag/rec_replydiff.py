#!/usr/bin/env python3
"""rec_replydiff.py — diff two §6 replay traces on EVERYTHING EXCEPT GuestRead.

Built for one question: *did a recorder-side change alter what the guest sees?*

A change that only makes the emulator READ more of guest RAM adds `GuestRead`
records and must add nothing else.  So project both traces down to the
non-GuestRead stream and compare, ignoring sequence numbers — which necessarily
shift the moment records are inserted.

★ THERE IS A NOISE FLOOR, AND IT IS LARGE.  Two captures of the SAME binary are
not identical, for reasons that have nothing to do with any change:

  * the guest polls `PTIMER_TIME_0/1` and the GSP mailbox a wall-clock-dependent
    number of times, so a single-stream positional diff desynchronises early and
    then reports ~everything after it as different.  That is an artefact of the
    projection, not a finding;
  * the guest kernel allocates the GSP message queues at a different guest
    physical address every boot, so raw GPAs are not comparable;
  * a handful of RPC replies legitimately carry wall-clock values (and the queue
    element checksum over them).

Hence: DIFF EACH KIND AS ITS OWN SUBSEQUENCE, and normalise.  Each kind's own
sequence is stable across boots even though their interleaving is not — which is
exactly the property that makes this a usable instrument.  GuestWrite offsets are
taken relative to the FIRST GuestWrite (the queue-region base), so a differently
placed queue compares equal.

★ ALWAYS RUN THE CONTROL.  Capture twice with the unmodified binary, diff those,
and only then read a before/after diff.  A before/after diff is evidence only
insofar as it is no worse than the control — same kinds, same indices, same
byte offsets.  A number with no control next to it means nothing here.

    rec_replydiff.py A.rec B.rec              per-kind, the useful view
    rec_replydiff.py A.rec B.rec --whole      + the (noisy) single-stream diff
    rec_replydiff.py A.rec B.rec --bytes      + byte offsets of payload diffs
"""
import argparse
import hashlib
import struct
import sys

MAGIC = 0x4352434552564B4E
HDR_FMT = "<QIIIIQQQQQQ3Q"
HDR_SIZE = struct.calcsize(HDR_FMT)
ENT_FMT = "<QBBBBIQQ"
ENT_SIZE = struct.calcsize(ENT_FMT)

KINDS = {1: "MmioRead", 2: "MmioWrite", 3: "GuestRead", 4: "GuestWrite",
         5: "IrqRaise", 6: "Clock", 7: "OverlaySnap"}

# GA10x PTIMER_TIME_0/1 (mode2_regs_ga10x.h) — the poll storm.  Both the COUNT
# of these reads and the value returned are wall-clock dependent at any revision.
PTIMER_OFFS = {0x00BB0080, 0x00BB0084}


def load(path):
    with open(path, "rb") as fh:
        blob = fh.read()
    if len(blob) < HDR_SIZE:
        sys.exit("FATAL: %s shorter than a header" % path)
    hdr = struct.unpack_from(HDR_FMT, blob, 0)
    if hdr[0] != MAGIC:
        sys.exit("FATAL: %s bad magic" % path)
    recs = []
    off = hdr[2]
    while off + ENT_SIZE <= len(blob):
        seq, kind, width, bar, _p, ln, a, b = struct.unpack_from(ENT_FMT, blob, off)
        pend = off + ENT_SIZE + ln
        if pend > len(blob):
            break
        recs.append((seq, kind, width, bar, ln, a, b, blob[off + ENT_SIZE:pend]))
        off = pend + ((8 - (ln & 7)) & 7)
    return hdr, recs


def sub(recs, kind):
    return [r for r in recs if r[1] == kind]


def proj_mmio(recs, drop_ptimer):
    return [(r[3], r[5], r[2], r[6]) for r in recs
            if not (drop_ptimer and r[3] == 0 and r[5] in PTIMER_OFFS)]


def proj_guestwr(recs):
    """(offset from the first GuestWrite, len, sha1) — GPA-base independent."""
    if not recs:
        return []
    base = recs[0][5]
    return [(r[5] - base, r[4], hashlib.sha1(r[7]).hexdigest()) for r in recs]


def elem(pay):
    """Decode a GSP_MSG_QUEUE_ELEMENT header, if this payload is one.
    authTag[16] aad[16] checkSum@32 seqNum@36 elemCount@40 rpc@48
    (rpc: hdrver@48 sig@52 length@56 function@60 rpc_result@64)."""
    if len(pay) < 80:
        return ""
    f = struct.unpack_from("<IIIIIIII", pay, 32)   # 32,36,40,44,48,52,56,60
    return (" [elem seq=%d elemCount=%d rpclen=%d fn=%d]"
            % (f[1], f[2], f[6], f[7]))


def diff(name, xa, xb, limit, payload=None):
    n = min(len(xa), len(xb))
    bad = [i for i in range(n) if xa[i] != xb[i]]
    same = (not bad) and len(xa) == len(xb)
    print("  %-26s A=%-8d B=%-8d differing=%-6d %s"
          % (name, len(xa), len(xb), len(bad), "IDENTICAL" if same else ""))
    for i in bad[:limit]:
        print("      [%d] A=%s" % (i, xa[i]))
        print("      [%d] B=%s" % (i, xb[i]))
        if payload:
            pa, pb = payload[0][i][7], payload[1][i][7]
            print("          A%s   B%s" % (elem(pa), elem(pb)))
            if len(pa) == len(pb):
                d = [j for j in range(len(pa)) if pa[j] != pb[j]]
                print("          %d differing payload bytes at %s"
                      % (len(d), d[:24] + (["..."] if len(d) > 24 else [])))
    if len(bad) > limit:
        print("      ... %d more" % (len(bad) - limit))
    return len(bad), abs(len(xa) - len(xb))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--whole", action="store_true")
    ap.add_argument("--bytes", action="store_true")
    ap.add_argument("--limit", type=int, default=8)
    args = ap.parse_args()

    ha, ra = load(args.a)
    hb, rb = load(args.b)
    for tag, path, h, r in (("A", args.a, ha, ra), ("B", args.b, hb, rb)):
        cnt = {}
        for e in r:
            cnt[KINDS.get(e[1], e[1])] = cnt.get(KINDS.get(e[1], e[1]), 0) + 1
        print("%s %s  props=0x%x mask=0x%x records=%d" % (tag, path, h[5], h[6], len(r)))
        print("   " + "  ".join("%s=%d" % kv for kv in sorted(cnt.items())))
    print("-" * 74)

    ga, gb = sub(ra, 4), sub(rb, 4)
    print("PER-KIND SUBSEQUENCES (the useful view)")
    nd = 0
    nd += diff("MmioWrite (guest->dev)", proj_mmio(sub(ra, 2), False),
               proj_mmio(sub(rb, 2), False), args.limit)[0]
    nd += diff("MmioRead  (dev->guest)", proj_mmio(sub(ra, 1), True),
               proj_mmio(sub(rb, 1), True), args.limit)[0]
    # Two projections, because they fail for different reasons.  WITH the offset
    # catches "same bytes, wrong place"; CONTENT-ONLY is immune to the guest
    # having allocated a buffer somewhere else this boot, which it does.
    nd += diff("GuestWrite (REPLIES)", proj_guestwr(ga), proj_guestwr(gb),
               args.limit, payload=(ga, gb) if args.bytes else None)[0]
    nd += diff("GuestWrite CONTENT only", [(r[4], hashlib.sha1(r[7]).hexdigest()) for r in ga],
               [(r[4], hashlib.sha1(r[7]).hexdigest()) for r in gb],
               args.limit, payload=(ga, gb) if args.bytes else None)[0]
    nd += diff("IrqRaise", [(r[5], r[6]) for r in sub(ra, 5)],
               [(r[5], r[6]) for r in sub(rb, 5)], args.limit)[0]
    print("  %-26s A=%-8d B=%-8d (ns is wall clock, not compared)"
          % ("Clock", len(sub(ra, 6)), len(sub(rb, 6))))
    print("  %-26s A=%-8d B=%-8d (the added observations)"
          % ("GuestRead", len(sub(ra, 3)), len(sub(rb, 3))))

    if args.whole:
        print("-" * 74)
        print("WHOLE-STREAM positional diff — EXPECT NOISE, see the docstring")
        pa = [(r[1], r[3], r[5], r[2], r[6], r[4]) for r in ra if r[1] != 3]
        pb = [(r[1], r[3], r[5], r[2], r[6], r[4]) for r in rb if r[1] != 3]
        diff("non-GuestRead", pa, pb, 4)

    print("-" * 74)
    print("TOTAL differing entries across per-kind subsequences: %d" % nd)
    return 0 if nd == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
