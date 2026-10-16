#!/usr/bin/env python3
"""s3_table.py : S3 table from /data/claude-perf2/ev/heaven/s3{ctl,exp}{1..3}_{d3d11,gl}.out (+ s3pure*). Medians, spreads, ratios."""
import re, os, sys, statistics as st, subprocess
EV = "/data/claude-perf2/ev/heaven"
def fps(label):
    p = f"{EV}/{label}.out"
    if not os.path.exists(p):
        return None
    t = re.sub(r"\d\d:\d\d:\d\d", " ", open(p, errors="replace").read().replace("&nbsp;", " ").replace("\n", " "))
    m = re.search(r"Frames:\s+(\d+).*?FPS:\s+([\d.]+)\D+?Min FPS:\s+([\d.]+).*?Max FPS:\s+([\d.]+)", t)
    return float(m.group(2)) if m else None
def block(prefixes, api):
    out = {}
    for arm, pre in prefixes.items():
        vals = [(f"{pre}{i}_{api}", fps(f"{pre}{i}_{api}")) for i in (1, 2, 3)]
        out[arm] = vals
    return out
for api in ("d3d11", "gl"):
    print(f"== {api}")
    res = block({"ctl": "s3ctl", "exp": "s3exp"}, api)
    meds = {}
    for arm, vals in res.items():
        xs = [v for _, v in vals if v]
        print(f"  {arm}: " + ", ".join(f"{l}={v}" for l, v in vals))
        if xs:
            m = st.median(xs); sp = (max(xs) - min(xs)) / m * 100
            meds[arm] = m
            print(f"     median {m:.2f} spread {sp:.1f}% n={len(xs)}")
    if len(meds) == 2:
        print(f"  exp/ctl = {meds['exp'] / meds['ctl']:.4f}  ({(meds['exp'] / meds['ctl'] - 1) * 100:+.1f}%)")
    pure = {a: fps(f"s3pure{a}_{api}") for a in ("ctl", "exp")}
    if any(pure.values()):
        print(f"  pure (no counters, 1 run each): {pure}", end="")
        if all(pure.values()):
            print(f"  exp/ctl = {pure['exp'] / pure['ctl']:.4f}")
        else:
            print()
