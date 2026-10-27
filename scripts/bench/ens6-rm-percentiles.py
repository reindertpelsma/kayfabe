#!/usr/bin/env python3
"""Percentiles of the ENS6 host RM benchmark: one line per arm and operation (microseconds).

usage: ens6-rm-percentiles.py OUT_ROOT [--json]
Reads OUT_ROOT/<arm>/samples.csv (op,index,wall_ns) and OUT_ROOT/<arm>.out (the arm's summary).
"""
import csv
import json
import sys
from pathlib import Path


def pct(sorted_ns, p):
    if not sorted_ns:
        return None
    return sorted_ns[min(len(sorted_ns) - 1, int(len(sorted_ns) * p))] / 1000.0


def arm_rows(root: Path):
    for d in sorted(p for p in root.iterdir() if p.is_dir()):
        csv_path = d / "samples.csv"
        if not csv_path.exists():
            continue
        by_op = {}
        with csv_path.open() as f:
            for row in csv.DictReader(f):
                by_op.setdefault(row["op"], []).append(int(row["wall_ns"]))
        summary = ""
        out = root / f"{d.name}.out"
        if out.exists():
            summary = next((l.strip() for l in out.read_text().splitlines() if "_RM_" in l), "")
        for op, v in sorted(by_op.items()):
            v.sort()
            yield {
                "arm": d.name,
                "op": op,
                "n": len(v),
                "p50_us": pct(v, 0.50),
                "p90_us": pct(v, 0.90),
                "p99_us": pct(v, 0.99),
                "max_us": v[-1] / 1000.0,
                "total_ms": sum(v) / 1e6,
                "summary": summary,
            }


def main():
    root = Path(sys.argv[1])
    rows = list(arm_rows(root))
    if "--json" in sys.argv:
        print(json.dumps(rows, indent=1))
        return
    print(f"{'arm':<22}{'op':<15}{'n':>8}{'p50us':>10}{'p90us':>10}{'p99us':>10}{'maxus':>12}{'total_ms':>11}")
    for r in rows:
        print(
            f"{r['arm']:<22}{r['op']:<15}{r['n']:>8}{r['p50_us']:>10.1f}{r['p90_us']:>10.1f}"
            f"{r['p99_us']:>10.1f}{r['max_us']:>12.1f}{r['total_ms']:>11.1f}"
        )


if __name__ == "__main__":
    main()
