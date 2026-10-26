#!/usr/bin/env python3
"""Merge kfwc / observer control censuses into one table of the controls a Windows driver issues.

usage: windows_controls.py label=census.controls.tsv [label=...] > windows_controls.tsv
       (also writes windows_ids.txt next to the tsv when --ids FILE is given before the labels)

Each input is a `*.controls.tsv` written by kf-census (columns cmd, name, req_sizes, ogkm, ...).
Output columns: cmd, name, ogkm (status in the census), then one `req_sizes` column per label
(`-` when the control was not issued in that capture).  Measured data only; no body bytes.
"""
import csv
import sys

args = sys.argv[1:]
idsf = None
if args and args[0] == "--ids":
    idsf, args = args[1], args[2:]
labels, rows = [], {}
for a in args:
    lab, path = a.split("=", 1)
    labels.append(lab)
    with open(path, newline="") as f:
        for r in csv.DictReader(f, delimiter="\t"):
            e = rows.setdefault(r["cmd"], {"name": r["name"], "ogkm": r["ogkm"], "sz": {}})
            if e["name"] == "-" and r["name"] != "-":
                e["name"] = r["name"]
            e["sz"][lab] = r["req_sizes"]
w = csv.writer(sys.stdout, delimiter="\t", lineterminator="\n")
w.writerow(["cmd", "name", "ogkm"] + labels)
for cmd in sorted(rows, key=lambda c: int(c, 16)):
    e = rows[cmd]
    w.writerow([cmd, e["name"], e["ogkm"]] + [e["sz"].get(l, "-") for l in labels])
if idsf:
    with open(idsf, "w") as f:
        f.write("# controls issued by a Windows driver in the kfwc / observer censuses (windows_controls.tsv)\n")
        for cmd in sorted(rows, key=lambda c: int(c, 16)):
            f.write("0x%08x\n" % int(cmd, 16))
