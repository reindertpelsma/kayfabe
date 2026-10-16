#!/usr/bin/env python3
"""l2_cmp.py <suite.out>... : paired per-arm table. Files named with a label: LABEL=path. Prints verdict, secs, client_ms."""
import sys, re, statistics as st
runs = {}
for a in sys.argv[1:]:
    lab, path = a.split("=", 1)
    d = {}
    for l in open(path, errors="replace"):
        m = re.match(r"FAST_CELL_ARM arm=(\S+) verdict=(\S+) secs=(\S+) client_ms=(\S+)", l)
        if m:
            d[m[1]] = (m[2], m[3], m[4])
    runs[lab] = d
labs = list(runs)
arms = list(runs[labs[0]])
def num(x):
    try: return float(x)
    except: return None
print("%-22s" % "arm" + "".join("%12s" % (l + " ms") for l in labs) + "  verdicts")
sums = {l: 0.0 for l in labs}; secs = {l: 0.0 for l in labs}
for arm in arms:
    row = "%-22s" % arm; vs = []
    for l in labs:
        v, s, c = runs[l].get(arm, ("-", "-", "-"))
        row += "%12s" % c; vs.append(v)
        if num(c): sums[l] += num(c)
        if num(s): secs[l] += num(s)
    print(row + "  " + "/".join(vs))
print("%-22s" % "SUM client_ms" + "".join("%12.0f" % sums[l] for l in labs))
print("%-22s" % "SUM wall secs" + "".join("%12.0f" % secs[l] for l in labs))
print("pass counts: " + ", ".join("%s=%d/%d" % (l, sum(1 for a in arms if runs[l].get(a, ("",))[0] == "PASS"), len(arms)) for l in labs))
