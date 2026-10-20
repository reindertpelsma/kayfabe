#!/usr/bin/env python3
"""fp_table.py RESULTS_TSV [ARMMAP] - per-arm starvation hit rates from results_fp.tsv.

Input lines (tab separated): TAG files PAIRNAME hit=.. starved=.. gaps=A:x,B:y iters=..
The arm of a tag is its leading letters (PC1 -> PC, fp1_C -> PC via the built-in alias).
Prints per arm: n pairs, >=1 s, >=2 s, >=4 s (max over clients of the longest completion gap), max gap.
"""
import re
import sys
from collections import OrderedDict

ALIAS = {"fp": "PC"}


def arm_of(tag):
    m = re.match(r"([A-Za-z]+)", tag)
    a = m.group(1) if m else tag
    return ALIAS.get(a, a)


def parse(text):
    rows = []
    for line in text.splitlines():
        f = line.split("\t")
        if len(f) < 7 or f[2] == "TOTAL" or not f[3].startswith("hit="):
            continue
        gaps = [float(x.split(":")[1]) for x in f[5][len("gaps="):].split(",")]
        rows.append((f[0], f[2], max(gaps)))
    return rows


def table(rows):
    arms = OrderedDict()
    for tag, _pair, g in rows:
        arms.setdefault(arm_of(tag), []).append(g)
    out = []
    for a, gs in arms.items():
        out.append((a, len(gs), sum(g >= 1 for g in gs), sum(g >= 2 for g in gs),
                    sum(g >= 4 for g in gs), max(gs)))
    return out


def main(argv):
    rows = parse(open(argv[0]).read())
    print("arm\tn_pairs\tge1s\tge2s\tge4s\tmax_s")
    for r in table(rows):
        print("%s\t%d\t%d\t%d\t%d\t%.2f" % r)


if __name__ == "__main__":
    main(sys.argv[1:])
