#!/usr/bin/env python3
"""l2_stats.py: paired statistics of the fast-suite client times. A = the candidate 525d7d7a (A1 first/cold, A2, A3), B = 0a746117 (B1, B2)."""
import re, sys, statistics as st
def load(p):
    d = {}
    for l in open(p, errors="replace"):
        m = re.match(r"FAST_CELL_ARM arm=(\S+) verdict=(\S+) secs=(\S+) client_ms=(\S+)", l)
        if m: d[m[1]] = float(m[4])
    return d
D = sys.argv[1]
A = [load(f"{D}/{t}_suite.out") for t in ("cand1", "cand2", "cand3")]
B = [load(f"{D}/{t}_suite.out") for t in ("base1", "base2")]
arms = list(A[0])
print("%-22s %8s %8s %8s | %8s %8s | %7s %7s %s" % ("arm", "A1", "A2", "A3", "B1", "B2", "A23avg", "Bavg", "dA23-B (ms, %)  within-B spread"))
big = []
for a in arms:
    a1, a2, a3 = (x[a] for x in A); b1, b2 = (x[a] for x in B)
    a23 = (a2 + a3) / 2; b = (b1 + b2) / 2; d = a23 - b
    sp = max(abs(b1 - b2), abs(a2 - a3), 20.0)   # noise floor: the 10 ms resolution doubled
    flag = "  <-- beyond spread" if abs(d) > 2 * sp and abs(d) > 0.05 * b else ""
    if flag: big.append(a)
    print("%-22s %8.0f %8.0f %8.0f | %8.0f %8.0f | %7.0f %7.0f %+6.0f %+5.1f%%  %5.0f%s" % (a, a1, a2, a3, b1, b2, a23, b, d, 100 * d / b, abs(b1 - b2), flag))
S = lambda x: sum(x.values())
print("\nSUM client_ms: A1=%.0f A2=%.0f A3=%.0f | B1=%.0f B2=%.0f | A23avg=%.0f Bavg=%.0f  diff=%+.0f (%+.2f%%)" % (S(A[0]), S(A[1]), S(A[2]), S(B[0]), S(B[1]), (S(A[1]) + S(A[2])) / 2, (S(B[0]) + S(B[1])) / 2, (S(A[1]) + S(A[2])) / 2 - (S(B[0]) + S(B[1])) / 2, 100 * ((S(A[1]) + S(A[2])) / 2 / ((S(B[0]) + S(B[1])) / 2) - 1)))
print("A1 (the first suite after the build) vs the B average: %+.2f%%" % (100 * (S(A[0]) / ((S(B[0]) + S(B[1])) / 2) - 1)))
print("arms beyond 2x spread and 5%% (A2/A3 average vs B average): %s" % (big or "none"))
