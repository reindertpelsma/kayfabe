#!/usr/bin/env python3
"""Census the guest kernel's NVRM 0x56 lines, clustered into bursts.

A burst = NVRM lines with < GAP seconds between consecutive lines. The point is
that the ctx workload is killed 180 s after it hangs, so its TEARDOWN lines are
separated from its INIT lines by a ~175 s hole. Counting them together ranks a
consequence of the wall as a candidate for the wall.
"""
import collections
import gzip
import re
import sys

GAP = 20.0


def kind(msg):
    """Collapse a line to its (kind, key) — never to a raw handle."""
    m = re.match(r"rpcRmApiAlloc_GSP: GspRmAlloc failed:.*hClass=(0x[0-9a-f]+)", msg)
    if m:
        return ("GspRmAlloc", m.group(1))
    if msg.startswith("rpcRmApiFree_GSP"):
        return ("GspRmFree", "-")
    m = re.search(r"returned from (?:pRmApi->)?Control\(.*?,\s*(NV[A-Z0-9_]+),", msg)
    if m:
        return ("Control", m.group(1))
    m = re.match(r"(\w+_IMPL|\w+): ", msg)
    fn = m.group(1) if m else msg[:40]
    m2 = re.search(r"returned from (\w+)\(", msg)
    if m2:
        return ("Check/Assert", m2.group(1) + "()")
    m3 = re.search(r"@ ([\w.]+:\d+)", msg)
    if m3:
        return ("Assert", m3.group(1))
    return (fn, "-")


def main(path):
    lines = []
    with gzip.open(path, "rt", errors="replace") as f:
        for ln in f:
            m = re.match(r"\[\s*([\d.]+)\] NVRM: (.*)", ln.rstrip("\n"))
            if m:
                lines.append((float(m.group(1)), m.group(2)))
    print("%s: %d NVRM lines" % (path, len(lines)))

    bursts, cur, prev = [], [], None
    for t, msg in lines:
        if prev is not None and t - prev > GAP:
            bursts.append(cur)
            cur = []
        cur.append((t, msg))
        prev = t
    bursts.append(cur)

    for i, b in enumerate(bursts):
        c = collections.Counter(kind(m) for _, m in b)
        print("\n--- burst %d: t=%.0f..%.0f s, %d lines ---" % (i, b[0][0], b[-1][0], len(b)))
        for (k, key), n in sorted(c.items(), key=lambda x: -x[1]):
            print("    %4d  %-14s %s" % (n, k, key))


if __name__ == "__main__":
    main(sys.argv[1])
