#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""summarize.py RUN_DIR... [--linux-ref DIR] [--apps apps.json] -- one markdown table per Windows app-matrix run,
in the shape of scripts/apps/summarize.py (the Linux matrix):

  | app | linux guest | win (batched) | win (alone) | detail |

`win (alone)` is the verdict of the re-run in a fresh guest (win_isolated.res) and is the final one when present
(the batched verdict is shown beside it when they differ); with --digest-ref FILE (APPDIG lines, e.g. a Linux
host.res) an app whose output digest differs becomes PASS*. `linux guest` is optional: --linux-ref points at a Linux
run directory with guest.res / guest_isolated.res (e.g. traces/v3_cdp/app_matrix_2830988f) and is shown for every
Windows app that maps to Linux rows (apps.json `linux`), as the worst verdict among them. The detail column carries
what Windows adds: rc/secs/quiet and the note of a non-PASS, plus guest TDR events, WER reports, kayfabe GSP cycles
and the GPU-use proof of every row. Closing lines: counts per verdict, per category, TDR totals, and the Linux rows
without a Windows equivalent. Reads the `APPRES` / `APPDIG` lines only.
"""
import collections
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
KV = re.compile(r"(\w+)=((?:(?! \w+=).)*)")
ORDER = {"PASS": 0, "PASS*": 0, "-": 1, "NOTRUN": 2, "FAIL": 3, "TIMEOUT": 4, "BOOT_FAIL": 5}


def rows(path):
    out = collections.OrderedDict()
    if not os.path.exists(path):
        return out
    for line in open(path, errors="replace"):
        if not line.startswith("APPRES "):
            continue
        kv = dict(KV.findall(line[7:].strip()))
        out[kv.get("app")] = kv                      # the LAST row per app wins
    return out


def digs(path):
    d = {}
    if os.path.exists(path):
        for line in open(path, errors="replace"):
            m = re.match(r"APPDIG side=(\S+) app=(\S+) (?:OUTSHA|DIGEST) (\S+) (\S+)", line)
            if m:
                d[(m.group(2), m.group(3))] = m.group(4)
    return d


def worst(vs):
    vs = [v for v in vs if v]
    return max(vs, key=lambda v: ORDER.get(v, 3)) if vs else "-"


def linux_verdict(ref, linux_names):
    if not ref or not linux_names:
        return "-"
    gb, gi = ref
    vs = []
    for n in linux_names:
        v = (gi.get(n) or gb.get(n) or {}).get("verdict")
        if v:
            vs.append(v)
    return worst(vs) if vs else "-"


def summarize(run, apps_doc, linux_ref=None, digest_ref=None):
    batched = rows(os.path.join(run, "win.res"))
    alone = rows(os.path.join(run, "win_isolated.res"))
    dig = digs(os.path.join(run, "win.res"))
    dig.update(digs(os.path.join(run, "win_isolated.res")))       # the alone run wins, as for verdicts
    dref = digs(digest_ref) if digest_ref else {}
    byid = {a["id"]: a for a in apps_doc["apps"]}
    ref = None
    if linux_ref:
        ref = (rows(os.path.join(linux_ref, "guest.res")), rows(os.path.join(linux_ref, "guest_isolated.res")))
    out = [f"\n### {run}\n", "| app | linux guest | win (batched) | win (alone) | detail |", "|---|---|---|---|---|"]
    cnt, cat, tdr, wer, cyc = collections.Counter(), collections.defaultdict(collections.Counter), 0, 0, 0
    order = list(batched.keys()) + [a for a in alone if a not in batched]
    for app in order:
        b, i = batched.get(app, {}), alone.get(app, {})
        bv, iv = b.get("verdict", "-"), i.get("verdict", "")
        final = iv or bv
        det = i or b
        spec = byid.get(app, {})
        notes = []
        if final not in ("PASS", "-"):
            notes.append(f"rc={det.get('rc')} {det.get('secs')}s quiet={det.get('quiet')} — {det.get('note', '')[:110]}")
        t = int(det.get("guest_tdr") or 0)
        if t:
            notes.append(f"{t} guest TDR")
        if int(det.get("wer") or 0):
            notes.append(f"{det.get('wer')} WER")
        if det.get("gsp_cycles") not in (None, "-", "0"):
            notes.append(f"gsp_cycles={det.get('gsp_cycles')}")
        if final == "PASS" and det.get("proof") and det.get("proof") != "-":
            notes.append(f"proof {det.get('proof')}")
        if dref:
            for (a, k), v in dig.items():
                if a == app and (a, k) in dref:
                    same = dref[(a, k)] == v
                    notes.append(f"{k} digest {'==' if same else '!='} ref")
                    if not same and final == "PASS":
                        final = "PASS*"
        tdr += t
        wer += int(det.get("wer") or 0)
        cnt[final] += 1
        cat[spec.get("category", "?")][final] += 1
        lin = linux_verdict(ref, spec.get("linux", [])) if ref else "-"
        print_row = f"| {app} | {lin} | {bv} | {iv or '-'} | {'; '.join(notes)} |"
        out.append(print_row)
    out.append("")
    out.append(", ".join(f"{k}: {v}" for k, v in sorted(cnt.items(), key=lambda kv: ORDER.get(kv[0], 3))) + f"  (total {sum(cnt.values())})")
    out.append("per category: " + "; ".join(f"{c}: {cat[c].get('PASS', 0) + cat[c].get('PASS*', 0)}/{sum(cat[c].values())}" for c in sorted(cat)))
    out.append(f"guest TDR events in passing+failing apps: {tdr}; WER reports: {wer}")
    miss = apps_doc.get("no_equivalent") or {}
    if miss:
        out.append("")
        out.append(f"Linux rows with no Windows equivalent ({len(miss)}): " + ", ".join(sorted(miss)))
    return "\n".join(out)


def main(argv):
    args, runs, ref, dref, apps_path = argv[1:], [], None, None, os.path.join(HERE, "apps.json")
    it = iter(args)
    for a in it:
        if a == "--linux-ref":
            ref = next(it)
        elif a == "--digest-ref":
            dref = next(it)
        elif a == "--apps":
            apps_path = next(it)
        else:
            runs.append(a)
    if not runs:
        print(__doc__)
        return 2
    doc = json.load(open(apps_path))
    for r in runs:
        print(summarize(r, doc, ref, dref))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
