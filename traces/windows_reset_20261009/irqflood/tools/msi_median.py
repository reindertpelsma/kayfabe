#!/usr/bin/env python3
"""msi_median.py RUN/per-second.txt HH:MM:SS HH:MM:SS -- median, min and max MSI per second in [from, to] (UTC)."""
import ast
import statistics
import sys

lo, hi = sys.argv[2], sys.argv[3]
v = []
for line in open(sys.argv[1]):
    t, _, d = line.partition(" ")
    if lo <= t <= hi:
        try:
            v.append(ast.literal_eval(d).get("msi", 0))
        except (ValueError, SyntaxError):
            pass
print(sys.argv[1], "n=%d median=%s min=%s max=%s" % (len(v), statistics.median(v) if v else None, min(v, default=None), max(v, default=None)))
