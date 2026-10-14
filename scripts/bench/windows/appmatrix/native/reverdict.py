#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""reverdict.py RUN_DIR [--apps-json apps.json] [--out win_re.res] -- recompute every verdict of a finished run from the facts
(apps/ID.json) and logs (apps/ID.log) the guest supervisor collected, with the CURRENT verdict.py / apps.json. Used when the
verdict rules or an app's success/proof predicate was fixed after a run: the measured facts are unchanged, only the judgement is.
Prints `ID old -> new` for every row that changes and writes the new APPRES lines."""
import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import verdict as V  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("--apps-json", default=os.path.join(HERE, "..", "apps.json"))
    ap.add_argument("--out", default="")
    a = ap.parse_args()
    apps = {x["id"]: x for x in json.load(open(a.apps_json))["apps"]}
    old = {}
    for l in open(os.path.join(a.run, "win.res"), errors="replace"):
        m = re.match(r"APPRES side=\S+ app=(\S+) verdict=(\S+)", l)
        if m:
            old[m.group(1)] = m.group(2)
    lines = []
    for aid, ov in old.items():
        fp, lp = os.path.join(a.run, "apps", aid + ".json"), os.path.join(a.run, "apps", aid + ".log")
        if aid not in apps or not os.path.exists(fp) or not os.path.getsize(fp):
            continue
        facts = json.load(open(fp))
        log = open(lp, errors="replace").read() if os.path.exists(lp) else ""
        d = V.decide(apps[aid], facts, log)
        app = apps[aid]
        lines.append(V.appres_line("win", aid, d, boot="re", extra=f"tier={app['tier']} cat={app['category']}"))
        if d["verdict"] != ov:
            print(f"{aid}: {ov} -> {d['verdict']} {d['note']}")
    if a.out:
        open(a.out, "w").write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
