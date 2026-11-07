#!/usr/bin/env python3
"""summarize.py BOXDIR... -- one status row per viewset from collected spec_run output (statuses, durations, reset counters, peak GPU memory).
Reads only cli.log (status lines), END.txt, tdr.txt, smi.csv. Prints no score."""
import re, sys, pathlib

def rows(box):
    for d in sorted(p for p in pathlib.Path(box).iterdir() if p.is_dir()):
        w = d.name
        cli = (d / "cli.log").read_text(errors="replace") if (d / "cli.log").exists() else ""
        end = (d / "END.txt").read_text(errors="replace").strip() if (d / "END.txt").exists() else ""
        tdr = (d / "tdr.txt").read_text(errors="replace") if (d / "tdr.txt").exists() else ""
        sub = re.findall(r"\[workload: [^/\]]+ / subtest: (\S+)\]\s*(Started|Completed|Failed|Incomplete|Skipped)", cli)
        last = {}
        for name, st in sub:
            last[name] = st
        n_ids = len(set(re.findall(r"<id=\d+>\s+(\S+)", cli)))
        wl = re.findall(r"\[Workload:\s*%s\]\s*(\w+)" % re.escape(w), cli)
        wl = wl[-1] if wl else "-"
        errs = [l.strip() for l in cli.splitlines() if re.search(r"Error|Requirement|not meet|No graphics card|refus", l, re.I)]
        m = re.search(r"secs=(\d+)", end)
        rc = re.search(r"rc=(\S+)", end)
        t = re.search(r"total=(\d+)", tdr)
        peak = 0
        sp = d / "smi.csv"
        if sp.exists():
            for l in sp.read_text(errors="replace").splitlines():
                f = l.split(",")
                if len(f) > 1:
                    try: peak = max(peak, int(f[1].replace("MiB", "").strip()))
                    except ValueError: pass
        cnt = {}
        for st in last.values():
            cnt[st] = cnt.get(st, 0) + 1
        yield (w, wl, n_ids, cnt, int(m.group(1)) if m else None, rc.group(1) if rc else None, int(t.group(1)) if t else None, peak, errs[:3], "END" if end else "NO_END")

for box in sys.argv[1:]:
    print("##", box)
    for w, wl, n, cnt, secs, rc, resets, peak, errs, e in rows(box):
        print(f"{w:18s} workload={wl:10s} subtests_listed={n:2d} {cnt} secs={secs} cli_rc={rc} resets_total={resets} peak_gpu_mem_MiB={peak} {e} {errs if errs else ''}")
