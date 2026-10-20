#!/usr/bin/env python3
"""ab_table.py - hit rates per arm from baton session directories.

  python3 -I ab_table.py RUNS_DIR      (RUNS_DIR contains one directory per session, each with files/)

A run counts at a threshold when ANY client has a gap between successive completions >= threshold
while another client completes in it (baton_parse.detect_batons semantics).  max_gap_s is the longest
such starved gap (0 when none >= 0.5 s).
"""
import glob
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import campaign_report as cr  # noqa: E402


def arm_of(tag):
    for p in ("ab1_", "ab2_"):
        if tag.startswith(p):
            return tag[len(p):]
    return tag


def summarize(runs):
    gaps = [r["max_gap_s"] for r in runs]
    return {"n": len(runs), "ge1": sum(g >= 1 for g in gaps), "ge2": sum(g >= 2 for g in gaps),
            "ge4": sum(g >= 4 for g in gaps), "max": max(gaps) if gaps else 0.0}


def pool(by_arm, arms):
    out = []
    for a in arms:
        out += by_arm.get(a, [])
    return out


def collect(runs_dir):
    by_arm = {}
    for d in sorted(glob.glob(os.path.join(runs_dir, "*", "files"))):
        tag = os.path.basename(os.path.dirname(d))
        for name in cr.group_runs(d):
            r = cr.analyze_run(d, name, 0.5)
            by_arm.setdefault(arm_of(tag), []).append(
                {"tag": tag, "run": name, "max_gap_s": r["max_starved_s"], "starved": r["starved_label"]})
    return by_arm


def main(argv):
    by_arm = collect(argv[0])
    print("%-14s %3s %5s %5s %5s %8s" % ("arm", "n", ">=1s", ">=2s", ">=4s", "max_s"))
    for arm in sorted(by_arm):
        s = summarize(by_arm[arm])
        print("%-14s %3d %5d %5d %5d %8.1f" % (arm, s["n"], s["ge1"], s["ge2"], s["ge4"], s["max"]))


if __name__ == "__main__":
    main(sys.argv[1:])
