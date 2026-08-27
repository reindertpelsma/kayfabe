#!/usr/bin/env python3
"""rec_dump.py — read a §6 replay trace produced by src/qemu/nvkvm_m2_rec.c.

The binary format is the durable artefact; this is the reference decoder for it
and the thing that proves a capture is well formed before anyone builds on it.

    rec_dump.py FILE                 header + per-kind counts + integrity check
    rec_dump.py FILE --head N        also print the first N records
    rec_dump.py FILE --tail N        also print the last N records
    rec_dump.py FILE --kinds K,...   restrict printing to those kinds
    rec_dump.py FILE --payload       print payload bytes (hex) for GuestRead/Write

Integrity check, in the sense the consumer needs (kayfabe-trace's
`check_dense_order`): sequence numbers must start at 0, be strictly increasing
and be DENSE.  A gap means the stream cannot be replayed — filtered events do
not consume a sequence number, so a legitimately masked capture is still dense.
"""
import argparse
import struct
import sys

MAGIC = 0x4352434552564B4E
# NvkvmRecHdr, in full — INCLUDING reserved1[3].  Leaving the reserved words out
# gives the right field values but the wrong sizeof, so the provenance block
# (which starts at sizeof(NvkvmRecHdr), not at the end of the last named field)
# reads back as empty.  That is exactly what it did the first time.
HDR_FMT = "<QIIIIQQQQQQ3Q"   # magic ver hdrlen recsize rsv props mask nrec nbytes nerr t0 rsv1[3]
HDR_SIZE = struct.calcsize(HDR_FMT)
assert HDR_SIZE == 96, HDR_SIZE
ENT_FMT = "<QBBBBIQQ"
ENT_SIZE = struct.calcsize(ENT_FMT)
assert ENT_SIZE == 32, ENT_SIZE

KINDS = {1: "MmioRead", 2: "MmioWrite", 3: "GuestRead", 4: "GuestWrite",
         5: "IrqRaise", 6: "Clock", 7: "OverlaySnap"}

MASK_BITS = [(0, "MMIO_RD"), (1, "MMIO_WR"), (2, "GUEST_RD"), (3, "GUEST_WR"),
             (4, "IRQ"), (5, "CLOCK"), (6, "OVERLAY"), (8, "BAR0"), (9, "BAR1"),
             (10, "BAR2"), (11, "PROM"), (12, "PRAMIN"), (13, "PTIMER")]

PROP_BITS = [(0, "trace"), (1, "m2fwd"), (2, "m2exec"), (3, "m2hostsem"),
             (4, "m2cefwd"), (5, "m2cexec"), (6, "m2opaque"), (7, "m2trace"),
             (8, "m2romregs"), (32, "NON-HERMETIC")]


def bits(v, table):
    return ",".join(n for b, n in table if v & (1 << b)) or "-"


def fmt(e, payload, show_payload):
    seq, kind, width, bar, _pad, ln, a, b = e
    k = KINDS.get(kind, "kind%d" % kind)
    if kind in (1, 2):
        s = "#%-9d %-10s bar%d off=0x%08x sz=%d val=0x%x" % (seq, k, bar, a, width, b)
    elif kind in (3, 4, 7):
        s = "#%-9d %-10s gpa=0x%012x len=%d" % (seq, k, a, ln)
        if show_payload and payload:
            s += "\n            " + payload.hex()
    elif kind == 5:
        s = "#%-9d %-10s %s" % (seq, k, "msix vec=%d" % b if a == 0
                                else "intx level=%d" % b)
    elif kind == 6:
        s = "#%-9d %-10s ns=%d" % (seq, k, a)
    else:
        s = "#%-9d %-10s a=0x%x b=0x%x len=%d" % (seq, k, a, b, ln)
    return s


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--head", type=int, default=0)
    ap.add_argument("--tail", type=int, default=0)
    ap.add_argument("--kinds", default="")
    ap.add_argument("--payload", action="store_true")
    args = ap.parse_args()

    want = set(args.kinds.split(",")) if args.kinds else None

    with open(args.file, "rb") as fh:
        blob = fh.read()

    if len(blob) < HDR_SIZE:
        sys.exit("FATAL: shorter than a header (%d bytes)" % len(blob))
    (magic, ver, hdrlen, recsize, _r0, props, mask,
     nrec, nbytes, nerr, t0, _r1, _r2, _r3) = struct.unpack_from(HDR_FMT, blob, 0)
    if magic != MAGIC:
        sys.exit("FATAL: bad magic 0x%016x" % magic)
    prov = blob[HDR_SIZE:hdrlen].split(b"\0", 1)[0].decode("utf-8", "replace")

    print("file        : %s (%d bytes)" % (args.file, len(blob)))
    print("version     : %d   rec_size=%d   hdr_len=%d" % (ver, recsize, hdrlen))
    print("props       : 0x%016x  [%s]" % (props, bits(props, PROP_BITS)))
    print("mask        : 0x%016x  [%s]" % (mask, bits(mask, MASK_BITS)))
    print("hdr counters: n_records=%d n_bytes=%d n_errors=%d t0_ns=%d"
          % (nrec, nbytes, nerr, t0))
    print("provenance  :")
    for line in prov.rstrip().splitlines():
        print("    " + line)
    print("-" * 72)

    off = hdrlen
    counts = {}
    n = 0
    expected = 0
    order_ok = True
    first_bad = None
    records = []          # kept only if we need head/tail printing
    keep_head = args.head
    tail_buf = []
    while off + ENT_SIZE <= len(blob):
        e = struct.unpack_from(ENT_FMT, blob, off)
        ln = e[5]
        pstart = off + ENT_SIZE
        pend = pstart + ln
        if pend > len(blob):
            print("TRUNCATED: record #%d claims %d payload bytes, file ends" % (e[0], ln))
            break
        payload = blob[pstart:pend] if ln else b""
        off = pend + ((8 - (ln & 7)) & 7)
        if e[0] != expected and order_ok:
            order_ok = False
            first_bad = (n, expected, e[0])
        expected = e[0] + 1
        k = KINDS.get(e[1], "kind%d" % e[1])
        counts[k] = counts.get(k, 0) + 1
        n += 1
        if want and k not in want:
            continue
        if len(records) < keep_head:
            records.append((e, payload))
        if args.tail:
            tail_buf.append((e, payload))
            if len(tail_buf) > args.tail:
                tail_buf.pop(0)

    print("records scanned : %d" % n)
    if nrec and nrec != n:
        print("  ! header says %d — the file was NOT closed cleanly "
              "(usable dense prefix, see nvkvm_rec_close)" % nrec)
    if nerr:
        print("  !! n_errors=%d — the sink hit short/failed writes; "
              "this artefact is NOT trustworthy" % nerr)
    print("dense order     : %s" % ("OK" if order_ok else
          "BROKEN at index %d (expected seq %d, got %d)" % first_bad))
    print("by kind:")
    for k in sorted(counts, key=lambda x: -counts[x]):
        print("    %-12s %d" % (k, counts[k]))
    silent = [k for k in KINDS.values() if k not in counts]
    print("never seen      : %s" % (", ".join(silent) if silent else "(none)"))

    if records:
        print("-" * 72)
        for e, p in records:
            print(fmt(e, p, args.payload))
    if tail_buf:
        print("-" * 72 + " (tail)")
        for e, p in tail_buf:
            print(fmt(e, p, args.payload))


if __name__ == "__main__":
    main()
