#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""baseline_report.py OUT_DIR RUN [RUN ...] -- merge native-baseline runs into one table.

For every app the LAST run (argument order) that contains it wins; its facts (apps/ID.json) and log are re-judged with the CURRENT
verdict.py / apps.json (so a rule fixed after a run applies to the older facts too), and written to OUT_DIR as
  results.tsv           one row per app: id, category, tier, reasoned difficulty, verdict, secs, proof, tdr/153 events, run, note
  results.md            the same as a markdown table plus counts per verdict / category / reasoned difficulty
  apps/ID.json          the supervisor's facts (log_tail included, <= 40 lines)
  apps/ID.log           the app's log, capped at 24 KiB (head+tail) so the evidence stays small
  verdict/ID.json       the verdict
Only text/JSON is read and written (no executables)."""
import collections
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import verdict as V  # noqa: E402


def cap(text, n=24 * 1024):
    if len(text) <= n:
        return text
    return text[: n // 2] + "\n=== [elided for the repository] ===\n" + text[-n // 2:]


def main():
    out, runs = sys.argv[1], sys.argv[2:]
    apps = {a["id"]: a for a in json.load(open(os.path.join(HERE, "..", "apps.json")))["apps"]}
    pick = {}
    for r in runs:
        ad = os.path.join(r, "apps")
        if not os.path.isdir(ad):
            continue
        for f in os.listdir(ad):
            if f.endswith(".json") and not f.endswith(".verdict.json"):
                aid = f[:-5]
                if aid in apps and os.path.getsize(os.path.join(ad, f)):
                    pick[aid] = r
    for d in ("apps", "verdict"):
        os.makedirs(os.path.join(out, d), exist_ok=True)
    rows = []
    for aid, r in sorted(pick.items()):
        a = apps[aid]
        facts = json.load(open(os.path.join(r, "apps", aid + ".json")))
        lp = os.path.join(r, "apps", aid + ".log")
        log = open(lp, errors="replace").read() if os.path.exists(lp) else ""
        d = V.decide(a, facts, log)
        ev = facts.get("events") or {}
        pdh = (facts.get("pdh") or {}).get("nv") or {}
        eng = ",".join(f"{k}={v:.0f}" for k, v in sorted(pdh.items()) if v and v >= 0.5)
        smi = facts.get("smi") or {}
        rows.append(dict(app=aid, category=a["category"], tier=a["tier"], difficulty=a["difficulty"], session=a["session"], verdict=d["verdict"],
                         secs=d["secs"], rc=d["rc"], proof=d["proof"], engines=eng, smi_util=smi.get("util_max"), tdr=d["tdr"],
                         nv153=ev.get("nvlddmkm_153", 0), wer=d["wer"], run=os.path.basename(r.rstrip("/")), note=d["note"]))
        json.dump(facts, open(os.path.join(out, "apps", aid + ".json"), "w"), indent=1)
        open(os.path.join(out, "apps", aid + ".log"), "w").write(cap(log))
        json.dump(dict(d, run=os.path.basename(r.rstrip("/"))), open(os.path.join(out, "verdict", aid + ".json"), "w"), indent=1)
    cols = ["app", "category", "tier", "difficulty", "session", "verdict", "secs", "rc", "proof", "engines", "smi_util", "tdr", "nv153", "wer", "run", "note"]
    with open(os.path.join(out, "results.tsv"), "w") as f:
        f.write("\t".join(cols) + "\n")
        for x in rows:
            f.write("\t".join(str(x[c]).replace("\t", " ") for c in cols) + "\n")
    cnt = collections.Counter(x["verdict"] for x in rows)
    bycat = collections.defaultdict(collections.Counter)
    bydif = collections.defaultdict(collections.Counter)
    for x in rows:
        bycat[x["category"]][x["verdict"]] += 1
        bydif[x["difficulty"]][x["verdict"]] += 1
    with open(os.path.join(out, "results.md"), "w") as f:
        f.write(f"{len(rows)} apps: " + ", ".join(f"{k} {v}" for k, v in sorted(cnt.items())) + "\n\n")
        f.write("| category | " + " | ".join(["PASS", "FAIL", "TIMEOUT", "NOTRUN"]) + " |\n|---|---|---|---|---|\n")
        for c in sorted(bycat):
            f.write(f"| {c} | " + " | ".join(str(bycat[c].get(v, 0)) for v in ["PASS", "FAIL", "TIMEOUT", "NOTRUN"]) + " |\n")
        f.write("\n| reasoned difficulty for kayfabe | " + " | ".join(["PASS", "FAIL", "TIMEOUT", "NOTRUN"]) + " |\n|---|---|---|---|---|\n")
        for c in ("low", "medium", "high"):
            f.write(f"| {c} | " + " | ".join(str(bydif[c].get(v, 0)) for v in ["PASS", "FAIL", "TIMEOUT", "NOTRUN"]) + " |\n")
        f.write("\n| app | cat | tier | diff | verdict | secs | proof | GPU engines (peak %) | nvlddmkm/153 | note |\n|---|---|---|---|---|---|---|---|---|---|\n")
        for x in rows:
            f.write(f"| {x['app']} | {x['category']} | {x['tier']} | {x['difficulty']} | {x['verdict']} | {x['secs']} | {x['proof']} | {x['engines']} | {x['nv153']} | {x['note'][:90].replace('|', '/')} |\n")
    print(open(os.path.join(out, "results.md")).read().split("\n\n")[0])


if __name__ == "__main__":
    main()
