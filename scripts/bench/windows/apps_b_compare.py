#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Differential classification of application rows: the same image, row and script on kf3 and on VFIO (the control) (stdlib only).

STATUS: LIVE, 2026-11-16 (GPU-free tests: scripts/ci/test_kf3_census.py). No VFIO control has run yet when this line was written.

    apps_b_compare.py --kf3 RESULT.json [--vfio RESULT.json] [--out FILE.md]

RESULT.json is the stage record (`propimage_apps_b_stage.py run --out`, key `apps`) or a list of rows {app, verdict, ...}. Classes:
    PASS/PASS        both platforms pass: fine
    FAIL/FAIL        the row or the application is the problem: fix or drop the row; NOT a kayfabe defect
    KAYFABE_DEFECT   kf3 fails, VFIO passes: the robustness backlog, highest priority (census attached)
    KF3_ONLY_PASS    kf3 passes, VFIO fails: odd, recorded
    NO_CONTROL_FAIL  kf3 failed and there is no control yet: a candidate defect (census attached)
    NO_CONTROL_PASS  kf3 passed, no control yet
A row's verdict counts as pass when it is PASS or PASS_LIMITED.
"""
import argparse
import json
import sys

OK = ("PASS", "PASS_LIMITED")
ORDER = ("KAYFABE_DEFECT", "NO_CONTROL_FAIL", "KF3_ONLY_PASS", "FAIL/FAIL", "NO_CONTROL_PASS", "PASS/PASS")


def rows_of(doc):
    if isinstance(doc, dict):
        doc = doc.get("apps", [])
    return {r["app"]: r for r in doc if isinstance(r, dict) and "app" in r}


def classify(kf3, vfio):
    out = []
    for app in kf3:
        a = kf3[app]
        b = vfio.get(app) if vfio is not None else None
        a_ok = a.get("verdict") in OK
        if b is None:
            cls = "NO_CONTROL_PASS" if a_ok else "NO_CONTROL_FAIL"
        else:
            b_ok = b.get("verdict") in OK
            cls = "PASS/PASS" if a_ok and b_ok else "FAIL/FAIL" if not a_ok and not b_ok else "KAYFABE_DEFECT" if b_ok else "KF3_ONLY_PASS"
        out.append({"app": app, "class": cls, "kf3": a.get("verdict"), "vfio": None if b is None else b.get("verdict"),
                    "detail": str(a.get("detail", ""))[:160], "census": a.get("census", {}), "gpu": a.get("gpu", {})})
    out.sort(key=lambda r: (ORDER.index(r["class"]), r["app"]))
    return out


def markdown(rows):
    lines = ["| row | class | kf3 | vfio | census (counts) | detail |", "|---|---|---|---|---|---|"]
    for r in rows:
        c = ", ".join("%s=%s" % kv for kv in (r["census"].get("counts") or {}).items()) or "-"
        lines.append("| %s | %s | %s | %s | %s | %s |" % (r["app"], r["class"], r["kf3"], r["vfio"] or "no control", c, r["detail"].replace("|", "/")))
    return "\n".join(lines) + "\n"


def main(argv=None):
    p = argparse.ArgumentParser()
    p.add_argument("--kf3", required=True)
    p.add_argument("--vfio")
    p.add_argument("--out")
    a = p.parse_args(argv)
    kf3 = rows_of(json.load(open(a.kf3)))
    vfio = rows_of(json.load(open(a.vfio))) if a.vfio else None
    rows = classify(kf3, vfio)
    text = markdown(rows)
    if a.out:
        open(a.out, "w").write(text)
    sys.stdout.write(text)
    return 1 if any(r["class"] == "KAYFABE_DEFECT" for r in rows) else 0


if __name__ == "__main__":
    sys.exit(main())
