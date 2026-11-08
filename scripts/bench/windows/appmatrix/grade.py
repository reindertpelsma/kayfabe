#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""grade.py -- the verdict rules of the Windows apps stage (docs/design/V3_WINDOWS_APPS_GATE.md, section 4).

STATUS: LIVE, 2026-11-08 (GPU-free; tests/test_grade.py). It reads a run directory of winapps.py (win.res,
win_isolated.res) and the list of apps the profile expected, and prints one `WINAPPS_GRADE` line plus one
`WINAPPS_APP` line per expected app, plus explicit non-gating `APP_POLICY` rows. It decides nothing about the host: windows_apps.sh combines this with the same-box
host reference (which only decides whether a guest failure is kayfabe's or a retry).

THE GATE IS THE GUEST PASSING (owner, 2026-11-01; same rule as windows_gate.sh): only a final verdict of PASS (or
PASS*) for EVERY expected app is a pass. The final verdict of an app is the one from its re-run alone in a fresh
guest (phase 2) when it has one, else the batched one. Classes of everything else:

  pass        PASS, PASS*
  guest_fail  FAIL, TIMEOUT (the app ran in the guest and did not prove GPU use, did not print its success line, or
              did not end): a verdict on the guest, kayfabe's only with the host reference green
  boot_fail   BOOT_FAIL (the guest did not come up): the gate's rule for a guest that does not boot
  harness     NOTRUN (a precondition was missing: a package would not stage, a binary is absent, the launch
              failed): never a statement about the GPU, never a pass, never an excuse: RETRY
  not_run     no row at all (the time budget ended, the driver died): RETRY

Exit code: 0 all pass, 1 at least one guest_fail or boot_fail, 3 only harness / not_run. A verdict row for an app
that was not expected is ignored (counted as `extra`). An empty expectation is a harness fault (exit 3).
"""
import argparse
import json
import os
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "ci"))
import app_policy

HERE = os.path.dirname(os.path.abspath(__file__))
KV = re.compile(r"(\w+)=((?:(?! \w+=).)*)")
PASS = {"PASS", "PASS*"}
GUEST_FAIL = {"FAIL", "TIMEOUT"}


def rows(path):
    out = {}
    if not os.path.exists(path):
        return out
    for line in open(path, errors="replace"):
        if line.startswith("APPRES "):
            kv = dict(KV.findall(line[7:].strip()))
            if kv.get("app"):
                out[kv["app"]] = kv                      # the LAST row per app wins
    return out


def classify(verdict):
    if verdict in PASS:
        return "pass"
    if verdict in GUEST_FAIL:
        return "guest_fail"
    if verdict == "BOOT_FAIL":
        return "boot_fail"
    if verdict == "NOTRUN":
        return "harness"
    return "harness"          # an unknown word is never a pass


def grade(run_dir, expected):
    batched = rows(os.path.join(run_dir, "win.res"))
    alone = rows(os.path.join(run_dir, "win_isolated.res"))
    per = {}
    for app in expected:
        row = alone.get(app) or batched.get(app)
        if row is None:
            per[app] = dict(cls="not_run", verdict="-", batched="-", alone="-", note="no row")
            continue
        per[app] = dict(cls=classify(row.get("verdict", "?")), verdict=row.get("verdict", "?"), secs=row.get("secs", "-"),
                        batched=(batched.get(app) or {}).get("verdict", "-"), alone=(alone.get(app) or {}).get("verdict", "-"),
                        note=(row.get("note") or "-")[:100], proof=row.get("proof", "-"), tdr=row.get("guest_tdr", "0"), wer=row.get("wer", "0"),
                        gsp=row.get("gsp_cycles", "-"))
    counts = {c: sum(1 for v in per.values() if v["cls"] == c) for c in ("pass", "guest_fail", "boot_fail", "harness", "not_run")}
    if not expected:
        rc, kind = 3, "RETRY"
    elif counts["guest_fail"] or counts["boot_fail"]:
        rc, kind = 1, "FAIL"
    elif counts["harness"] or counts["not_run"]:
        rc, kind = 3, "RETRY"
    else:
        rc, kind = 0, "PASS"
    extra = sorted((set(batched) | set(alone)) - set(expected))
    return rc, kind, counts, per, extra


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run_dir")
    ap.add_argument("--expected", required=True, help="comma list of app ids the profile expects")
    a = ap.parse_args(argv)
    expected = [x for x in a.expected.split(",") if x]
    rc, kind, counts, per, extra = grade(a.run_dir, expected)
    app_policy.main(["app_policy.py", "windows"])
    for app in expected:
        v = per[app]
        print(f"WINAPPS_APP app={app} class={v['cls']} verdict={v['verdict']} batched={v['batched']} alone={v['alone']} "
              f"secs={v.get('secs', '-')} tdr={v.get('tdr', '-')} wer={v.get('wer', '-')} gsp={v.get('gsp', '-')} note={v['note']}")
    bad = {c: [x for x in expected if per[x]["cls"] == c] for c in ("guest_fail", "boot_fail", "harness", "not_run")}
    print(f"WINAPPS_GRADE kind={kind} expected={len(expected)} pass={counts['pass']} guest_fail={counts['guest_fail']} boot_fail={counts['boot_fail']} "
          f"harness={counts['harness']} not_run={counts['not_run']} extra={len(extra)}")
    for c, ids in bad.items():
        if ids:
            print(f"WINAPPS_GRADE_{c.upper()} {','.join(ids)}")
    return rc


if __name__ == "__main__":
    sys.exit(main())
