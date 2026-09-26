#!/usr/bin/env python3
"""gbparse.py <geekbench stdout> — the GPU composite and every workload's score from Geekbench's console
report. Prints GB_SCORE <n> and GB_WORKLOAD <name> <score>, one per workload (name with spaces → _).
Tolerant of the layout: a workload line is '<Name>   <score>   <rate unit…>' in the results block."""
import re, sys
t = open(sys.argv[1], errors="replace").read()
m = re.search(r"(?:Vulkan|OpenCL|GPU)\s+Score\s+(\d+)", t)
print("GB_SCORE", m.group(1) if m else "none")
blk = t.split("Workload", 1)[-1] if "Workload" in t else t
seen = set()
for line in blk.splitlines():
    w = re.match(r"^\s{2,}([A-Z][A-Za-z0-9 +\-/()]+?)\s{2,}(\d+)\s*$", line) or \
        re.match(r"^\s{2,}([A-Z][A-Za-z0-9 +\-/()]+?)\s{2,}(\d+)\s{2,}\S", line)
    if w and "Score" not in w.group(1):
        n = w.group(1).strip().replace(" ", "_")
        if n not in seen:
            seen.add(n); print("GB_WORKLOAD", n, w.group(2))
