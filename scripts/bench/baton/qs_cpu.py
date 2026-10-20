#!/usr/bin/env python3
"""qs_cpu.py QSAMPLE.tsv [T0 T1] - per-thread-name CPU fraction from a qsampler.py file.

qsampler.py writes a row only when a thread advanced CPU ticks or is not in a plain wait, so the
CPU fraction is sum(d_cpu_ticks)/CLK_TCK over the wall time window, NOT a share of rows.
Prints: comm, cpu_seconds, fraction_of_one_core, rows_in_state_R, rows.  Window: [T0,T1] epoch
(default: the whole file).
"""
import collections
import sys

CLK_TCK = 100.0


def load(path):
    rows = []
    with open(path) as f:
        for line in f:
            if line.startswith("#"):
                continue
            p = line.rstrip("\n").split("\t")
            if len(p) < 7:
                continue
            rows.append((float(p[0]), p[1], p[2], p[3], int(p[6])))
    return rows


def summarize(rows, t0=None, t1=None):
    if not rows:
        return 0.0, {}
    t0 = rows[0][0] if t0 is None else t0
    t1 = rows[-1][0] if t1 is None else t1
    span = max(t1 - t0, 1e-9)
    cpu = collections.defaultdict(float)
    rr = collections.Counter()
    n = collections.Counter()
    for ts, tid, comm, st, d in rows:
        if ts < t0 or ts > t1:
            continue
        cpu[comm] += d / CLK_TCK
        n[comm] += 1
        rr[comm] += st == "R"
    return span, {c: (cpu[c], cpu[c] / span, rr[c], n[c]) for c in n}


def main(argv):
    rows = load(argv[0])
    t0 = float(argv[1]) if len(argv) > 2 else None
    t1 = float(argv[2]) if len(argv) > 2 else None
    span, s = summarize(rows, t0, t1)
    print("window_s=%.1f" % span)
    for c, (sec, fr, r, n) in sorted(s.items(), key=lambda kv: -kv[1][0]):
        print("%s\tcpu_s=%.2f\tcores=%.3f\tR_rows=%d\trows=%d" % (c, sec, fr, r, n))


if __name__ == "__main__":
    main(sys.argv[1:])
