#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bundle-pin-vs-tree mutants: each rule needs two distinct named killing tests (tests/test_bundle_pin.py).

Only PinDriftDetection is collected: it builds its own consistent pin in a disposable copy of the tree, so it is green on the
baseline. RepoPin (the check on the real tree) is red until the bundle is republished and is NOT a killer here: any edit of a
hashed file would otherwise "kill" a mutant by moving the real tree's hash, not by the test noticing the rule.
"""
import argparse
import concurrent.futures
import json
import sys
import tempfile

import bundle_mutants as M

GUEST_SUFFIXES = 'if p.suffix not in (".ps1", ".py", ".html"):'
RECIPE_LOOP = 'for name in ("package_bundle.py", "bundle_iso.py", "bundle_tools.py", "bundle_compliance.py", "bundle_wheels.py", "bundle_repro.py"):'
CASES = [
    ("pin-compare-skipped", "bundle_ci.py", 'return None if pinned == derived else dict(pinned=pinned, derived=derived)', 'return None'),
    ("pin-compare-inverted", "bundle_ci.py", 'return None if pinned == derived else dict(pinned=pinned, derived=derived)', 'return None if pinned != derived else dict(pinned=pinned, derived=derived)'),
    ("pin-missing-inputs-is-a-pass", "bundle_ci.py", 'bundle_class).get("inputs", "")', 'bundle_class).get("inputs", derived)'),
    ("pin-wrong-file-set-guest-python", "package_bundle.py", GUEST_SUFFIXES, 'if p.suffix not in (".ps1", ".html"):'),
    ("pin-wrong-file-set-guest-powershell", "package_bundle.py", GUEST_SUFFIXES, 'if p.suffix not in (".py", ".html"):'),
    ("pin-wrong-file-set-guest-html", "package_bundle.py", GUEST_SUFFIXES, 'if p.suffix not in (".ps1", ".py"):'),
    ("pin-wrong-file-set-apps-json", "package_bundle.py", 'files["apps.json"] = canonical(load(root / "apps.json"))', 'pass'),
    ("pin-wrong-file-set-recipe-files", "package_bundle.py", RECIPE_LOOP, 'for name in ():'),
    ("pin-wrong-file-set-licences", "package_bundle.py", 'for name in ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md"):', 'for name in ():'),
    ("pin-wrong-file-set-probe-sources", "package_bundle.py", 'for name in bundle_tools.SOURCES:', 'for name in ():'),
    ("pin-wrong-selection", "package_bundle.py", 'return iso_input_hash(packages, toolchain, profile["open_packages"], root)', 'return iso_input_hash([], toolchain, [], root)'),
]

COLLECT = '''import io,json,sys,unittest
sys.path.insert(0,'tests')
import test_bundle_pin as T
s=unittest.defaultTestLoader.loadTestsFromTestCase(T.PinDriftDetection)
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))
'''


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("only", nargs="?", default="", help="run only the cases whose name contains this text")
    args = ap.parse_args(argv)
    M.COLLECT = COLLECT
    cases = [c for c in CASES if args.only in c[0]]
    with tempfile.TemporaryDirectory(prefix="kf-bundle-pin-mutants-") as parent:
        baseline = M.run(M.fixture(parent, "baseline"))
        if baseline["killers"]:
            raise RuntimeError("BASELINE RED: " + str(baseline["killers"]))
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            rows = list(pool.map(lambda case: M.one(parent, case), cases))
    print(json.dumps(dict(tests=baseline["tests"], cases=rows), indent=2))
    return int(any(row["count"] < 2 for row in rows))


if __name__ == "__main__":
    sys.exit(main())
