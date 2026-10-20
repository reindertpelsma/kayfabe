#!/usr/bin/env python3
"""z_report.py RUNS_DIR RESULTS_TSV - per-boot table for the control-vs-fix campaign.

Per boot (tag): arm (letters of the tag: ZB = fix B, ZC = control C), pairs >= 1/2/4 s (largest completion gap of any
client per pair), max gap, PROGRESS_STRAND lines, last backstop_found(work, carried, contended), worker CPU (cores,
whole session, from qsample.tsv). Then per arm totals. Tab separated.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qs_cpu  # noqa: E402


def pairs(tsv):
    out = {}
    for line in open(tsv):
        f = line.rstrip("\n").split("\t")
        if len(f) < 7 or f[2] == "TOTAL" or not f[3].startswith("hit="):
            continue
        g = [float(x.split(":")[1]) for x in f[5][len("gaps="):].split(",")]
        out.setdefault(f[0], []).append(max(g))
    return out


def boot_info(runs, tag):
    d = os.path.join(runs, tag)
    q = [p for p in os.listdir(d) if p.endswith("_qemu.log")] if os.path.isdir(d) else []
    strands = work = carried = cont = 0
    if q:
        txt = open(os.path.join(d, q[0]), errors="replace").read()
        strands = len(re.findall(r"PROGRESS_STRAND for_ms", txt))
        m = re.findall(r"backstop_found\(work=(\d+) carried=(\d+) contended=(\d+)\)", txt)
        if m:
            work, carried, cont = map(int, m[-1])
    cores = 0.0
    qs = os.path.join(d, "qsample.tsv")
    if os.path.exists(qs):
        span, s = qs_cpu.summarize(qs_cpu.load(qs))
        cores = max([v[1] for c, v in s.items() if c.startswith("kf3-worker")] or [0.0])
    return strands, work, carried, cont, cores


def main(argv):
    runs, tsv = argv[0], argv[1]
    p = pairs(tsv)
    print("tag\tarm\tpairs\tge1\tge2\tge4\tmax_gap\tstrand_lines\tbf_work\tbf_carried\tbf_contended\tworker_cores_max")
    for tag, gs in p.items():
        arm = re.match(r"[A-Za-z]+", tag).group(0)
        s, w, c, ct, cores = boot_info(runs, tag)
        r = (len(gs), sum(g >= 1 for g in gs), sum(g >= 2 for g in gs), sum(g >= 4 for g in gs), max(gs), s, w)
        print("%s\t%s\t%d\t%d\t%d\t%d\t%.2f\t%d\t%d\t%d\t%d\t%.3f" % ((tag, arm) + r[:5] + (s, w, c, ct, cores)))
    print("ARM_TOTALS arm boots pairs ge1 ge2 ge4 maxgap strands bf_work")
    arms = {}
    for tag, gs in p.items():
        arm = re.match(r"[A-Za-z]+", tag).group(0)
        s, w, c, ct, cores = boot_info(runs, tag)
        a = arms.setdefault(arm, [0, 0, 0, 0, 0, 0.0, 0, 0])
        a[0] += 1
        a[1] += len(gs)
        a[2] += sum(g >= 1 for g in gs)
        a[3] += sum(g >= 2 for g in gs)
        a[4] += sum(g >= 4 for g in gs)
        a[5] = max(a[5], max(gs))
        a[6] += s
        a[7] += w
    for arm, a in sorted(arms.items()):
        print("%s\tboots=%d\tpairs=%d\tge1=%d\tge2=%d\tge4=%d\tmax=%.2f\tstrands=%d\tbf_work=%d" % ((arm,) + tuple(a)))


if __name__ == "__main__":
    main(sys.argv[1:])
