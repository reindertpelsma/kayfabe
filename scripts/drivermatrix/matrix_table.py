#!/usr/bin/env python3
"""The host x guest matrix, DERIVED from the committed walk evidence (never typed).

Reads every queue log under traces/driver_matrix/walk/<box>/summary/ (q*.log, hostwalk*.log) and
keeps, per (host, guest), the LATEST revision's rows:
  MATRIX_ROW host=H guest=G rev=R FAST_SUITE_PASS=p ... ARMS=n   -> thin p/n
  LADDER host=H guest=G rev=R k/n                              -> ladder k/n
  GATES host=H V3_GATES_SUMMARY pass=p fail=f                    -> gates (per host)
Queue logs that predate the host= field on LADDER lines (a guest walk on the bench host) are read
with the log's own `host=` from its *_START line.

  usage: matrix_table.py [walk-dir]   (default: traces/driver_matrix/walk)
"""
import os
import re
import sys

WALK = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(__file__), "..", "..", "traces", "driver_matrix", "walk")

cells = {}   # (host, guest) -> {"thin": (order, rev, text), "ladder": (...)}
gates = {}   # host -> (order, rev, text)
order = (0, 0)   # (the queue's *_START timestamp, line number): the LATEST measurement wins


def vkey(v):
    return tuple(int(x) for x in re.findall(r"\d+", v))


def put(kind, host, guest, rev, text):
    c = cells.setdefault((host, guest), {})
    if kind not in c or c[kind][0] <= order:
        c[kind] = (order, rev, text)


for box in sorted(os.listdir(WALK)):
    sdir = os.path.join(WALK, box, "summary")
    if not os.path.isdir(sdir):
        continue
    logs = sorted(f for f in os.listdir(sdir) if re.match(r"(q\d+\w*|hostwalk\d*)\.log$", f))
    for f in logs:
        start_host, start_ts = None, ""
        for n, line in enumerate(open(os.path.join(sdir, f), errors="replace")):
            m = re.match(r"\S+_START (\S+) rev=(\w+)(?: host=([\d.]+))?", line)
            if m:
                start_ts, start_host = m.group(1), m.group(3)
            order = (start_ts, n)
            m = re.match(r"V3_GATES_SUMMARY pass=(\d+) fail=(\d+)", line)
            if m and start_host:
                if start_host not in gates or gates[start_host][0] <= order:
                    gates[start_host] = (order, "", f"{m.group(1)}/{int(m.group(1)) + int(m.group(2))}")
                continue
            m = re.match(r"MATRIX_ROW host=([\d.]+) guest=([\d.]+) rev=(\w+) .*?(?:thin=(\d+/\d+)|FAST_SUITE_PASS=(\d+) .*ARMS=(\d+))", line)
            if m:
                thin = m.group(4) or f"{m.group(5)}/{m.group(6)}"
                put("thin", m.group(1), m.group(2), m.group(3), thin)
                continue
            m = re.match(r"LADDER host=([\d.]+) guest=([\d.]+) rev=(\w+) (\d+/\d+)", line)
            if m:
                put("ladder", m.group(1), m.group(2), m.group(3), m.group(4))
                continue
            m = re.match(r"LADDER guest=([\d.]+) (UNSTAGED)", line)
            if m and start_host:
                put("ladder", start_host, m.group(1), "-", "unstaged")
                continue
            m = re.match(r"GATES host=([\d.]+) V3_GATES_SUMMARY pass=(\d+) fail=(\d+)", line)
            if m and (m.group(1) not in gates or gates[m.group(1)][0] <= order):
                gates[m.group(1)] = (order, "", f"{m.group(2)}/{int(m.group(2)) + int(m.group(3))}")

hosts = sorted({h for h, _ in cells} | set(gates), key=vkey)
guests = sorted({g for _, g in cells}, key=vkey)
print("| guest \\ host | " + " | ".join(hosts) + " |")
print("|---|" + "---|" * len(hosts))
print("| *gates* | " + " | ".join(gates.get(h, ((), "", "—"))[2] for h in hosts) + " |")
for g in guests:
    row = []
    for h in hosts:
        c = cells.get((h, g), {})
        parts = []
        if "thin" in c:
            parts.append(f"thin {c['thin'][2]}")
        if "ladder" in c:
            parts.append(f"ladder {c['ladder'][2]}")
        revs = sorted({v[1] for v in c.values() if v[1] and v[1] != "-"})
        row.append((", ".join(parts) + (f" `{'/'.join(revs)}`" if revs else "")) if parts else "")
    print(f"| {g} | " + " | ".join(row) + " |")
