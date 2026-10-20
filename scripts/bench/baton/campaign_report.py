#!/usr/bin/env python3
"""campaign_report.py - per-run verdicts of a baton campaign.

A session directory holds files NAME.LABEL.{log,csv,wd} (as split from the serial log by
session3.sh).  A run is a HIT when some client is starved >= --min-gap seconds while another
client completes (baton_parse.detect_batons).

  python3 -I campaign_report.py [--min-gap S] DIR [DIR...]   one TSV line per run (default 4 s)
"""
import glob
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import baton_parse as bp  # noqa: E402


def group_runs(d):
    names = set()
    for p in glob.glob(os.path.join(d, "*.log")):
        b = os.path.basename(p)[:-4]
        if b.count(".") == 1 and os.path.exists(os.path.join(d, b + ".csv")):
            names.add(b.split(".")[0])
    return sorted(names)


def analyze_run(d, name, min_gap=4.0):
    clients, longest = {}, {}
    for p in sorted(glob.glob(os.path.join(d, name + ".*.log"))):
        lab = os.path.basename(p)[len(name) + 1:-4]
        with open(p) as f:
            log = bp.parse_log(f.read())
        with open(p[:-4] + ".csv") as f:
            its = bp.parse_dump(f.read())
        clients[lab] = (log["t0_epoch"], its)
        done = sorted(i[3] for i in its)
        prev, g = 0.0, 0.0
        for x in done:
            g = max(g, x - prev)
            prev = x
        longest[lab] = g
    ev = bp.detect_batons(clients, min_gap_s=min_gap) if len(clients) > 1 else []
    worst = max(ev, key=lambda e: e["gap_s"]) if ev else None
    return {"run": name, "hit": bool(ev), "events": ev,
            "max_starved_s": worst["gap_s"] if worst else 0.0,
            "starved_label": worst["starved"] if worst else "",
            "longest_gap_s": longest,
            "iters": {l: len(c[1]) for l, c in clients.items()},
            "wd": sorted(glob.glob(os.path.join(d, name + ".*.wd")))}


def rate(runs):
    return (sum(1 for r in runs if r["hit"]), len(runs))


def main(argv):
    min_gap = 4.0
    if argv and argv[0] == "--min-gap":
        min_gap, argv = float(argv[1]), argv[2:]
    for d in argv:
        runs = [analyze_run(d, n, min_gap) for n in group_runs(d)]
        for r in runs:
            print("%s\t%s\thit=%d\tstarved=%s %.1fs\tgaps=%s\titers=%s" % (
                os.path.basename(d.rstrip("/")), r["run"], r["hit"], r["starved_label"] or "-", r["max_starved_s"],
                ",".join("%s:%.2f" % kv for kv in sorted(r["longest_gap_s"].items())),
                ",".join("%s:%d" % kv for kv in sorted(r["iters"].items()))))
        h, n = rate(runs)
        print("%s\tTOTAL\t%d/%d" % (os.path.basename(d.rstrip("/")), h, n))


if __name__ == "__main__":
    main(sys.argv[1:])
