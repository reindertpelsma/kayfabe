#!/usr/bin/env python3
"""Dump DxgKrnl ETW CSV rows (tracerpt) in a UTC time window, optionally filtered by a substring.
usage: etwwin.py file.csv HH:MM:SS.mmm HH:MM:SS.mmm [filter...]"""
import sys, datetime

def ft2utc(ft):
    us = ft // 10
    return datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=us)

def parse(path):
    with open(path, errors="replace") as f:
        for line in f:
            if not line.startswith("Microsoft-Windows-DxgKrnl"):
                continue
            cols = [c.strip() for c in line.rstrip("\n").split(",")]
            try:
                eid = int(cols[2]); ft = int(cols[16])
            except Exception:
                continue
            yield ft2utc(ft), eid, cols[1], cols[9], cols[10], cols[11], [c for c in cols[19:] if c != ""]

def tod(s):
    h, m, rest = s.split(":")
    return int(h) * 3600 + int(m) * 60 + float(rest)

def main():
    path, a, b = sys.argv[1], tod(sys.argv[2]), tod(sys.argv[3])
    flt = sys.argv[4:]
    excl = [f[1:] for f in flt if f.startswith("-")]
    inc = [f for f in flt if not f.startswith("-")]
    for t, eid, typ, pid, tid, cpu, data in parse(path):
        x = t.hour * 3600 + t.minute * 60 + t.second + t.microsecond / 1e6
        if x < a or x > b:
            continue
        line = f"{t.strftime('%H:%M:%S.%f')} {eid:4d} {typ:5s} pid={pid} tid={tid} cpu={cpu} " + " ".join(data)
        if inc and not any(f in line for f in inc):
            continue
        if any(f in line for f in excl):
            continue
        print(line)

main()
