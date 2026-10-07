#!/usr/bin/env python3
"""nvd_phases.py — split an nvdiff capture into the phases the workload named.

Answers one question the flat trace cannot: *between the launch and the
cuCtxSynchronize that reported the fault, did the process issue any ioctl at all?*

`nvd_prog`'s fault stages write markers into the trace as `_IOW('F',0x7f,char[32])`
calls on /dev/nvidiactl carrying an ASCII tag (see the block comment in nvd_prog.c).
This tool finds them, splits the capture, and prints per-phase ioctl counts plus any
nonzero rc / NV status inside each phase.

    ./nvd_phases.py <capture.jsonl>

⊘ An UNMARKED capture is refused, not treated as one phase: an unsplit trace looks
exactly like a trace whose fault phase happens to be empty, and those are opposite
findings.
"""
import sys
import os
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import nvdiff  # noqa: E402

MARK_NR = 0x7f


def marker_tag(rec):
    """The ASCII tag of a marker record, or None if this is not a marker."""
    if rec.get("t") != "ioctl" or rec.get("nr") != MARK_NR:
        return None
    raw = nvdiff.unhex(rec.get("hpre", ""))
    if not raw.startswith(b"NVDMARK:"):
        return None
    return raw[len(b"NVDMARK:"):].split(b"\0")[0].decode("ascii", "replace")


def main(path):
    recs = nvdiff.load(path)
    marks = [(i, marker_tag(r)) for i, r in enumerate(recs)]
    marks = [(i, t) for i, t in marks if t]
    print("capture: %s" % path)
    print("records=%d  markers=%d" % (len(recs), len(marks)))
    if not marks:
        print("⊘ REFUSED: no NVDMARK records. This capture cannot be phase-split, and")
        print("  an unsplit trace must NOT be read as 'the phase was empty'.")
        return 2

    bounds = [(t, i) for i, t in marks]
    bounds.append(("<end-of-capture>", len(recs)))
    print()
    print("%-26s %-8s %6s  %s" % ("phase (from marker)", "records", "ioctls", "ops / status"))
    for k in range(len(bounds) - 1):
        tag, lo = bounds[k]
        hi = bounds[k + 1][1]
        span = recs[lo + 1:hi]                     # exclude the marker itself
        io = [r for r in span if r.get("t") == "ioctl"]
        bad = [r for r in io
               if r.get("rc", 0) != 0 or (r.get("f_post", {}).get("status") or 0) != 0]
        c = Counter(r["op"] for r in io)
        top = ", ".join("%s×%d" % (o, n) for o, n in c.most_common(4)) or "(none)"
        print("%-26s %-8d %6d  %s" % (tag, len(span), len(io), top))
        for r in bad:
            print("%-26s %-8s %6s    ★ i=%d %s rc=%s status=0x%x"
                  % ("", "", "", r["i"], r["op"], r.get("rc"),
                     r.get("f_post", {}).get("status") or 0))
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.stderr.write(__doc__)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
