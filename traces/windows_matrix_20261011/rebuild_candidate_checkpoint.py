#!/usr/bin/env python3
"""Fold newer executed app rows into the saved84-row mixed-revision checkpoint.

BOOT_FAIL is retained as setup evidence and never replaces an executed oracle.
Runs without per-app timestamps use their recorded run start, explicitly named.
"""
import csv
import json
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
SOURCES = [
    ("managed-routing/windows-focus", "72741e2b", "72741e2b"),
    ("managed-routing/nbody-fresh", "72741e2b", "738b5385"),
    ("managed-routing/furmark-off", "72741e2b", "72741e2b"),
    ("managed-routing/remaining-apps", "72741e2b", "72741e2b"),
    ("managed-routing/clpeak-cuda", "72741e2b", "72741e2b"),
    ("stream-startup/candidate-fresh", "72741e2b", "738b5385"),
    ("gravity-vulkan/furmark-fresh-result", "72741e2b", "0c5fa450"),
]


def timestamp(root, app):
    result = root / "apps" / f"{app}.json"
    if result.exists():
        end = json.loads(result.read_text()).get("end_utc")
        if end:
            return end, "app_end"
    meta = json.loads((root / "matrix.json").read_text())
    return meta["started_utc"], "run_start_only"


def write_tsv(name, fields, rows):
    with (HERE / name).open("w", newline="") as f:
        w = csv.DictWriter(f, fields, delimiter="\t")
        w.writeheader()
        w.writerows(rows)


def main():
    with (HERE / "candidate_combined_checkpoint_84.tsv").open() as f:
        selected = {r["app"]: r for r in csv.DictReader(f, delimiter="\t")}
    for row in selected.values():
        row["observation_utc"], row["timestamp_basis"] = timestamp(
            (HERE / row["source"]).parent, row["app"])
    for directory, product, harness in SOURCES:
        root = HERE / directory
        if not (root / "win.res").exists():
            continue
        for line in (root / "win.res").read_text().splitlines():
            if not line.startswith("APPRES "):
                continue
            d = dict(x.split("=", 1) for x in line.split()[1:] if "=" in x)
            if d["verdict"] in ("BOOT_FAIL", "NOTRUN"):
                continue
            app = d["app"]
            observed, basis = timestamp(root, app)
            if app in selected and observed <= selected[app]["observation_utc"]:
                continue
            status = d["verdict"]
            if status == "PASS":
                if d["guest_tdr"] == "0" and d["wer"] == "0" and d["gsp_cycles"] == "0":
                    status = "PASS"
                elif d["gsp_cycles"] == "-":
                    status = "PASS_HEALTH_UNMEASURED"
                else:
                    status = "PASS_WITH_RECOVERY"
            selected[app] = dict(app=app, status=status, product=product, harness=harness,
                                 verdict=d["verdict"], guest_tdr=d["guest_tdr"],
                                 gsp_cycles=d["gsp_cycles"], source=f"{directory}/win.res",
                                 observation_utc=observed, timestamp_basis=basis)
    inv = json.loads((REPO / "scripts/bench/windows/appmatrix/apps.json").read_text())
    if isinstance(inv, dict):
        inv = inv["apps"]
    wanted = {a["id"] for a in inv if not a["id"].startswith("overlay_")}
    assert selected.keys() <= wanted
    rows = [selected.get(a, dict(app=a, status="PENDING")) for a in sorted(wanted)]
    fields = ["app", "status", "product", "harness", "verdict", "guest_tdr",
              "gsp_cycles", "source", "observation_utc", "timestamp_basis"]
    write_tsv("candidate_combined_checkpoint.tsv", fields, rows)
    with (HERE / "linux_parity_checkpoint_84.tsv").open() as f:
        parity = list(csv.DictReader(f, delimiter="\t"))
    for r in parity:
        if r["windows_checkpoint"] == "OMITTED":
            continue
        counterparts = r["windows_counterparts"].split(",")
        statuses = [(a, selected.get(a, {}).get("status", "PENDING")) for a in counterparts]
        r["windows_checkpoint"] = "PASS" if all(s == "PASS" for _, s in statuses) else ",".join(
            f"{a}:{s}" for a, s in statuses)
    write_tsv("linux_parity_checkpoint_current.tsv", list(parity[0]), parity)
    counts = Counter(r["status"] for r in rows)
    applicable = [r for r in parity if r["windows_checkpoint"] != "OMITTED"]
    print(json.dumps(dict(total=len(rows), counts=counts,
                          linux_applicable=len(applicable),
                          linux_all_counterparts_pass=sum(r["windows_checkpoint"] == "PASS" for r in applicable)), indent=2))


if __name__ == "__main__":
    main()
