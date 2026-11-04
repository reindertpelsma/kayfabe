#!/usr/bin/env python3
"""Check control_sim.py's prediction against every iteration's measured control.

usage: check_sim.py <phase-dir> ...   (each holds summary.tsv and it_<i>_fuzz.txt)
For each iteration: the seed's predicted control outcome (engine reached or not) vs the measured
control line ("ok    W385 control = ... of which N ran the ENGINE" / "CONTROL FAILED").
"""
import importlib.util
import os
import re
import sys

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("cs", os.path.join(here, "control_sim.py"))
cs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cs)

agree = disagree = unknown = 0
for d in sys.argv[1:]:
    rows = [l.rstrip("\n").split("\t") for l in open(os.path.join(d, "summary.tsv"))][1:]
    for r in rows:
        i, seed = r[0], r[6]
        txt = os.path.join(d, f"it_{i}_fuzz.txt")
        body = open(txt, errors="replace").read() if os.path.exists(txt) else ""
        m = re.search(r"of which (\d+) ran the ENGINE", body)
        measured = None
        if m:
            measured = int(m.group(1)) > 0
        elif "CONTROL FAILED" in body:
            m2 = re.search(r"(\d+) engine ops", body)
            measured = bool(m2 and int(m2.group(1)) > 0)
        if not seed.isdigit() or measured is None:
            unknown += 1
            print(f"{d} it={i} seed={seed} measured=? verdict={r[1]}")
            continue
        pred = cs.control(int(seed))[0]
        if pred == measured:
            agree += 1
        else:
            disagree += 1
            print(f"DISAGREE {d} it={i} seed={seed} predicted={pred} measured={measured}")
        if not pred:
            print(f"PREDICTED_NO_ENGINE {d} it={i} seed={seed} verdict={r[1]} reason={r[5]}")
print(f"SIM_CHECK agree={agree} disagree={disagree} unknown={unknown}")
