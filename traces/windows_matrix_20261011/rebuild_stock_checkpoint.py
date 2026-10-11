#!/usr/bin/env python3
"""Rebuild the latest stock580.88 checkpoint from explicitly admitted evidence.

No diagnostic substitutions, no best-result selection, no assumed GSP health.
Earlier attempts remain in their original evidence directories.
"""
import csv
import json
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
# Frozen directory names recorded in matrix.json supply the source revision.
# The old mutable-tree BlackScholes retry lacks a revision record; the fresh
# frozen retry replaces it when present, never by guessing its old revision.
SOURCES = [
    ("reference-focus", "d6dd9c95"),
    ("reference-focus/blackscholes-retry2", "UNRECORDED"),
    ("reference-supplement-retry", "41c797ec"),
    ("reference-vector-gravity", "8037e19b"),
    ("reference-hf-cdp-vector", "22e5ce7b"),
    ("reference-fresh-cdp-cup8-gravity", "02766ef3"),
    ("reference-video-managed", "c5c97c00"),
    ("reference-furmark", "c5c97c00"),
    ("reference-demo-qa-gui", "582d72dc"),
    ("reference-unigine-nbody", "582d72dc"),
    ("reference-video-recovery", "582d72dc"),
    ("reference-remaining", "582d72dc"),
    ("reference-clpeak", "3335adcc"),
    ("reference-rest", "582d72dc"),
    ("reference-direct-stream", "738b5385"),
    ("reference-black-frozen", "738b5385"),
]


def main():
    inv = json.loads((REPO / "scripts/bench/windows/appmatrix/apps.json").read_text())
    if isinstance(inv, dict):
        inv = inv["apps"]
    wanted = {a["id"] for a in inv if not a["id"].startswith("overlay_")}
    selected = {}
    for directory, revision in SOURCES:
        root = HERE / directory
        if not (root / "win.res").exists():
            continue
        for line in (root / "win.res").read_text().splitlines():
            if not line.startswith("APPRES "):
                continue
            d = dict(x.split("=", 1) for x in line.split()[1:] if "=" in x)
            app = d["app"]
            if app not in wanted:
                continue
            result = json.loads((root / "apps" / f"{app}.json").read_text())
            end = result["end_utc"]
            setup = json.loads((root / "guests" / d["boot"] / "setup.json").read_text())
            assert "RTX 4090, 580.88," in setup["smi"], (directory, setup["smi"])
            assert d["gsp_cycles"] == "-", (directory, d)
            status = d["verdict"]
            if status == "PASS" and (d["guest_tdr"] != "0" or d["wer"] != "0"):
                status = "PASS_WITH_EVENTS"
            row = dict(app=app, status=status, verdict=d["verdict"],
                       guest_tdr=d["guest_tdr"], wer=d["wer"],
                       gsp_cycles="UNMEASURED", harness=revision, end_utc=end,
                       source=f"{directory}/win.res")
            if app not in selected or end > selected[app]["end_utc"]:
                selected[app] = row
    fields = ["app", "status", "verdict", "guest_tdr", "wer", "gsp_cycles",
              "harness", "end_utc", "source"]
    with (HERE / "stock_580_checkpoint.tsv").open("w", newline="") as f:
        writer = csv.DictWriter(f, fields, delimiter="\t")
        writer.writeheader()
        for app in sorted(wanted):
            writer.writerow(selected.get(app, dict(app=app, status="PENDING")))
    counts = Counter(row["status"] for row in selected.values())
    counts["PENDING"] = len(wanted - selected.keys())
    unrecorded = [r["app"] for r in selected.values() if r["harness"] == "UNRECORDED"]
    print(json.dumps(dict(total=len(wanted), counts=counts,
                          harness_revision_unrecorded=unrecorded), indent=2))


if __name__ == "__main__":
    main()
