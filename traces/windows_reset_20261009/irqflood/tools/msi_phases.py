#!/usr/bin/env python3
"""msi_phases.py RUN/per-second.txt [flood_floor] -- MSI per second and the second the guest's own interrupts stop.

per-second.txt lines are `HH:MM:SS {dict}` (tl.py); `msi` is the MSI count. With a flood the floor is its ~100/s
(period 10 ms); the guest is 'quiet' from the first second after which every later second is <= floor+8 (flood) or <= 5 (none).
"""
import ast
import sys

rows = []
for line in open(sys.argv[1]):
    t, _, d = line.partition(" ")
    try:
        rows.append((t, ast.literal_eval(d).get("msi", 0)))
    except (ValueError, SyntaxError):
        pass
floor = int(sys.argv[2]) if len(sys.argv) > 2 else 0
lim = floor + 8 if floor else 5
quiet = None
for i, (t, _) in enumerate(rows):
    if all(m <= lim for _, m in rows[i:]):
        quiet = i
        break
print("first MSI second", rows[0][0], "last", rows[-1][0], "total", sum(m for _, m in rows))
print("guest quiet (<=%d/s) from" % lim, rows[quiet][0] if quiet is not None else None)
end = quiet + 3 if quiet is not None else len(rows)
print(" ".join("%s=%d" % (t[3:], m) for t, m in rows[:end]))
