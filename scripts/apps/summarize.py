#!/usr/bin/env python3
"""summarize.py <results-dir>... — one markdown row per app from host.res / guest.res /
guest_isolated.res (+ APPDIG digests compared host vs guest). The LAST row per (side, app) wins.
The guest verdict is the ISOLATED re-run when there is one (a fresh boot with that app alone), and
the batched verdict is shown beside it when they differ."""
import re, sys, os, collections

def rows(path):
    out = collections.OrderedDict()
    if not os.path.exists(path): return out
    for line in open(path, errors="replace"):
        if not line.startswith("APPRES "): continue
        kv = dict(re.findall(r"(\w+)=((?:(?! \w+=).)*)", line[7:].strip()))
        out[kv.get("app")] = kv
    return out

def digs(path):
    d = {}
    if not os.path.exists(path): return d
    for line in open(path, errors="replace"):
        m = re.match(r"APPDIG side=(\S+) app=(\S+) (?:OUTSHA|DIGEST) (\S+) (\S+)", line)
        if m: d[(m.group(2), m.group(3))] = m.group(4)
    return d

for R in sys.argv[1:]:
    host = rows(f"{R}/host.res"); gb = rows(f"{R}/guest.res"); gi = rows(f"{R}/guest_isolated.res")
    hd = {}
    for line in open(f"{R}/host.res", errors="replace") if os.path.exists(f"{R}/host.res") else []:
        m = re.match(r"APPDIG side=host app=(\S+) (?:OUTSHA|DIGEST) (\S+) (\S+)", line)
        if m: hd[(m.group(1), m.group(2))] = m.group(3)
    gd = digs(f"{R}/guest.dig"); gd.update(digs(f"{R}/iso/guest.dig"))
    print(f"\n### {R}\n")
    print("| app | host | guest (batched) | guest (alone) | guest detail |")
    print("|---|---|---|---|---|")
    cnt = collections.Counter()
    for app in host.keys() | gb.keys():
        pass
    order = list(host.keys()) + [a for a in gb if a not in host]
    for app in order:
        h = host.get(app, {}); b = gb.get(app, {}); i = gi.get(app, {})
        hv = h.get("verdict", "-"); bv = b.get("verdict", "-"); iv = i.get("verdict", "")
        final = iv or bv
        det = i or b
        detail = ""
        if final not in ("PASS", "-"):
            detail = f"rc={det.get('rc')} {det.get('secs')}s quiet={det.get('quiet')} xid={det.get('guest_xid')} kf3_refusals={det.get('kf3_refusals')} — {det.get('note','')[:90]}"
        for (a, k), v in hd.items():
            if a == app and (a, k) in gd:
                same = gd[(a, k)] == v
                detail += f" {k} digest {'==' if same else '!='} host"
                if not same and final == "PASS": final = "PASS*"
        cnt[(hv, final)] += 1
        print(f"| {app} | {hv} | {bv} | {iv or '-'} | {detail.strip()} |")
    print("\n" + ", ".join(f"host {k[0]} / guest {k[1]}: {v}" for k, v in sorted(cnt.items())))
