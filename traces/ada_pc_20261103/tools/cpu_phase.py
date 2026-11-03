#!/usr/bin/env python3
"""cpu_phase.py <run dir> <from_s> <to_s>: per-thread-name CPU % of one core between two controller offsets."""
import json, sys
from collections import defaultdict
from pathlib import Path
rows = [json.loads(l) for l in (Path(sys.argv[1]) / 'cpu.jsonl').read_text().splitlines() if l.strip()]
lo, hi = float(sys.argv[2]), float(sys.argv[3])
a = min((r for r in rows if r['t'] >= lo), key=lambda r: r['t'])
b = max((r for r in rows if r['t'] <= hi), key=lambda r: r['t'])
dt = b['t'] - a['t']
per = defaultdict(int)
for tid, (comm, ticks) in b['th'].items():
    per[comm] += ticks - a['th'].get(tid, (comm, ticks))[1]
tot = sum(per.values())
print(f'CPU {sys.argv[1]} t={a["t"]:.0f}..{b["t"]:.0f}s ({dt:.0f}s): total {tot/dt:.1f}% of one core; ' +
      ', '.join(f'{c}={t/dt:.2f}%' for c, t in sorted(per.items(), key=lambda x: -x[1])[:9]))
