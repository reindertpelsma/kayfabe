#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""gate_profiles.py -- which Windows apps the release gate runs, and which it never does (docs/design/V3_WINDOWS_APPS_GATE.md).

STATUS: LIVE, 2026-11-07 (GPU-free; tests/test_gate_profiles.py).

  gate_profiles.py ids PROFILE      comma list of app ids (small | mid | full | ids:a,b,c)
  gate_profiles.py budget PROFILE   the default wall-clock budget in seconds for the apps of that profile
  gate_profiles.py open             every app whose packages are all in the R2 bundle (the automatable set)
  gate_profiles.py classes          the three classes of apps (open / owner decision / manual) with counts
  gate_profiles.py check            consistency of gate_profiles.json with apps.json and manifest.json (exit 1 on a problem)

The R2 bundle (`winapps-open.iso`) holds ONLY the packages named in gate_profiles.json `open_packages`: redistributable
by their own licences. Apps that need any other package are `owner_decision` (free to download, redistribution not
granted, or a GPL build the owner keeps out of our storage, or a dead pin) or `manual` (licensed, account-bound, or
online-only); neither is ever in an automated profile. `full` is every open app minus `excluded`, and an exclusion needs
a reason and evidence (a native-reference failure); an app that fails on kayfabe is NOT excluded: it fails the release.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name):
    return json.load(open(os.path.join(HERE, name)))


def closure(pkgs, needs):
    out, st = set(), list(pkgs)
    while st:
        p = st.pop()
        if p in out:
            continue
        out.add(p)
        st += needs.get(p, [])
    return out


def open_apps(apps, manifest, prof):
    needs = {p["id"]: p.get("needs", []) for p in manifest["packages"]}
    allowed = set(prof["open_packages"])
    return [a["id"] for a in apps["apps"] if closure(a["pkgs"], needs) <= allowed]


def resolve(profile, apps=None, manifest=None, prof=None):
    apps, manifest, prof = apps or load("apps.json"), manifest or load("manifest.json"), prof or load("gate_profiles.json")
    known = {a["id"] for a in apps["apps"]}
    if profile.startswith("ids:"):
        ids = [x for x in profile[4:].split(",") if x]
        bad = [x for x in ids if x not in known]
        if not ids or bad:
            raise SystemExit(f"unknown or empty app list: {bad or profile}")
        return ids
    opn = open_apps(apps, manifest, prof)
    if profile == "full":
        ex = set(prof.get("excluded", {}))
        return [x for x in opn if x not in ex]
    if profile in ("small", "mid"):
        ids = prof["profiles"][profile]["apps"]
        bad = [x for x in ids if x not in opn]
        if bad:
            raise SystemExit(f"profile {profile} names apps outside the open bundle: {bad}")
        return list(ids)
    raise SystemExit(f"unknown profile {profile!r} (small, mid, full, ids:...)")


def budget(profile, prof=None):
    prof = prof or load("gate_profiles.json")
    if profile.startswith("ids:"):
        return prof["profiles"]["full"]["budget_s"]
    return prof["profiles"][profile]["budget_s"]


def classes(apps=None, manifest=None, prof=None):
    apps, manifest, prof = apps or load("apps.json"), manifest or load("manifest.json"), prof or load("gate_profiles.json")
    opn = set(open_apps(apps, manifest, prof))
    owner = set(prof["owner_decision"]["apps"])
    manual = set(prof["manual"]["apps"])
    return opn, owner, manual


def check():
    apps, manifest, prof = load("apps.json"), load("manifest.json"), load("gate_profiles.json")
    known = {a["id"] for a in apps["apps"]}
    pkgs = {p["id"] for p in manifest["packages"]}
    problems = []
    for p in prof["open_packages"]:
        if p not in pkgs:
            problems.append(f"open package {p} is not in manifest.json")
    for p in prof["open_packages"]:
        if p in prof["owner_decision"]["packages"]:
            problems.append(f"{p} is both open and owner_decision")
    opn, owner, manual = classes(apps, manifest, prof)
    if owner & opn or manual & opn or owner & manual:
        problems.append(f"classes overlap: open&owner={sorted(owner & opn)} open&manual={sorted(manual & opn)} owner&manual={sorted(owner & manual)}")
    rest = known - opn - owner - manual
    if rest:
        problems.append(f"apps in no class: {sorted(rest)}")
    for x in owner | manual:
        if x not in known:
            problems.append(f"{x} is listed but not in apps.json")
    for x in prof.get("excluded", {}):
        if x not in opn:
            problems.append(f"excluded {x} is not an open app")
    sm, mid = prof["profiles"]["small"]["apps"], prof["profiles"]["mid"]["apps"]
    if not (8 <= len(sm) <= 10 and 8 <= len(mid) <= 10 and set(sm) <= set(mid) and len(sm) <= len(mid)):
        problems.append("small and mid must be 8 to 10 apps each, small a subset of mid")
    for name in ("small", "mid"):
        for x in prof["profiles"][name]["apps"]:
            if x not in opn:
                problems.append(f"{name}: {x} is outside the open bundle")
    cats = {a["id"]: a["category"] for a in apps["apps"]}
    if len({cats[x] for x in sm if x in cats}) < 5:
        problems.append("small must span at least 5 categories")
    return problems


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    cmd = argv[1]
    if cmd == "ids" and len(argv) == 3:
        print(",".join(resolve(argv[2])))
    elif cmd == "budget" and len(argv) == 3:
        print(budget(argv[2]))
    elif cmd == "open":
        print(",".join(open_apps(load("apps.json"), load("manifest.json"), load("gate_profiles.json"))))
    elif cmd == "classes":
        o, w, m = classes()
        print(f"open={len(o)} owner_decision={len(w)} manual={len(m)} total={len(o) + len(w) + len(m)}")
    elif cmd == "check":
        pr = check()
        for p in pr:
            print("PROBLEM", p)
        print("GATE_PROFILES_OK" if not pr else f"GATE_PROFILES_FAILED problems={len(pr)}")
        return 1 if pr else 0
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
