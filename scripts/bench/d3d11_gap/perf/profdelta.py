#!/usr/bin/env python3
"""profdelta.py PROF.txt.gz : first and last heartbeat of selected cumulative PROF counters in a window, with rates/s."""
import gzip, re, sys
from datetime import datetime
L = [l for l in gzip.open(sys.argv[1], "rt", errors="replace")]
def ts(l): 
    h, m, s = l.split()[0].split(":"); return int(h) * 3600 + int(m) * 60 + float(s)
def grab(pat):
    r = [(ts(l), l) for l in L if pat in l]
    return r[0], r[-1]
def nums(l): return {k: float(v) for k, v in re.findall(r"(\w+)=([0-9.]+)", l)}
out = []
for name, pat in [("bar0_handler", "bar0_handler n="), ("rpc", "PROF rpc doorbells"), ("rpc_service", "rpc_service n="), ("rpc_held", "rpc_held n="), ("vidmem_view_reads", "vidmem_view_reads"), ("workers", "PROF workers busy_ms"), ("acts", "PROF acts run"), ("drainer", "PROF drainer busy_ms"), ("vamgr", "PROF vamgr busy_ms"), ("va", "PROF va invals")]:
    try: a, b = grab(pat)
    except IndexError: continue
    dt = b[0] - a[0]; na, nb = nums(a[1]), nums(b[1]); parts = []
    for k in nb:
        if k in ("max_us", "worst_us", "max_busy_us") or k.startswith(("p5", "p9", "avg", "t")) or k in ("busy_pct",): continue
        d = nb[k] - na.get(k, 0)
        parts.append("%s=%g(%.4g/s)" % (k, d, d / dt if dt else 0))
    print("%-18s window=%.1fs  %s" % (name, dt, " ".join(parts)))
