#!/usr/bin/env python3
"""Per-arm A/B: client_ms from <tag>_suite.out and walk timing from vat[...] lines of each arm's qemu.log.
usage: ab_compare.py <benchA> <tagA> <benchB> <tagB>"""
import re, sys
from pathlib import Path


def load(bench, tag):
    b = Path(bench)
    arms = {}
    for l in (b / f'{tag}_suite.out').read_text().splitlines():
        m = re.match(r'FAST_CELL_ARM arm=(\S+) verdict=(\S+) secs=(\d+) client_ms=(\d+)', l)
        if m:
            arms[m.group(1)] = {'v': m.group(2), 'ms': int(m.group(4))}
    for arm, d in arms.items():
        f = b / f'fast_{tag}_{arm}_qemu.log'
        walks = wsum = gsum = inv = isum = 0
        if f.exists():
            # vat windows are per-heartbeat deltas; sum them
            for m in re.finditer(r'vat\[invals=(\d+) arrive->clear_avg_us=(\d+) max_us=\d+ walks=(\d+) '
                                 r'walk_avg_us=(\d+) gpu_avg_us=(\d+)', f.read_text(errors='replace')):
                i, ia, w, wa, ga = map(int, m.groups())
                walks += w; wsum += w * wa; gsum += w * ga; inv += i; isum += i * ia
        d.update(walks=walks, walk_us=wsum // walks if walks else 0, gpu_us=gsum // walks if walks else 0,
                 invals=inv, clear_us=isum // inv if inv else 0)
    return arms


a, b = load(sys.argv[1], sys.argv[2]), load(sys.argv[3], sys.argv[4])
print(f'{"arm":24s} | {"A verdict/client_ms":>19s} {"walks":>6s} {"walk_us":>7s} {"gpu_us":>6s} {"clear_us":>8s} | '
      f'{"B verdict/client_ms":>19s} {"walks":>6s} {"walk_us":>7s} {"gpu_us":>6s} {"clear_us":>8s}')
ta = tb = 0
for arm in a:
    x, y = a[arm], b.get(arm, {})
    ta += x['ms']; tb += y.get('ms', 0)
    print(f'{arm:24s} | {x["v"]:>8s} {x["ms"]:>10d} {x["walks"]:>6d} {x["walk_us"]:>7d} {x["gpu_us"]:>6d} {x["clear_us"]:>8d} | '
          f'{y.get("v","-"):>8s} {y.get("ms",0):>10d} {y.get("walks",0):>6d} {y.get("walk_us",0):>7d} '
          f'{y.get("gpu_us",0):>6d} {y.get("clear_us",0):>8d}')
print(f'TOTAL client_ms A={ta} B={tb} ratio A/B={ta/tb if tb else 0:.2f}')
