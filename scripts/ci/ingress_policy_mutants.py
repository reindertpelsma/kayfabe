#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the guest-ingress gate (STATUS: LIVE, 2026-11-20; task I7).

`docs/design/V3_GUEST_INGRESS_TABLE.md`: the generator's policy (`tools/api_boundaries/ingress_policy.py`)
and the gate (`crates/kf-abi/src/capability.rs`). Each mutant is a one-line change (a policy flip, a
privileged id admitted, the class check skipped, an unknown or unlisted id admitted); each must be killed
by at least two independently named failing tests. Every file touched is overwritten in place and ALWAYS
restored (also on an interrupt).
Run: python3 scripts/ci/ingress_policy_mutants.py [mutant-name ...]   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
GEN = "tools/api_boundaries/ingress_policy.py"
GATE = "crates/kf-abi/src/capability.rs"
FILES = {}
for rel in (GEN, GATE):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()


def changed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old)
    return rel, FILES[rel].replace(old, new)


CASES = [
    # generator policy
    ("privileged-control-admitted", changed(GEN, '    if any("PRIVILEGED" in f for f in each):', "    if False:")),
    ("internal-control-admitted", changed(GEN, '    if any("INTERNAL" in f for f in each):', "    if False:")),
    ("kernel-only-control-admitted", changed(GEN, '    if not all("NON_PRIVILEGED" in f for f in each):', "    if False:")),
    ("control-policy-flipped", changed(GEN, '    return True, "OgkmNonPrivileged"\n\n\ndef class_verdict',
                                       '    return False, "KernelOnly"\n\n\ndef class_verdict')),
    ("class-check-skipped", changed(GEN, "                    ok, why = class_verdict(e)",
                                    '                    ok, why = e is not None, "OgkmNonPrivileged"')),
    ("privileged-class-admitted", changed(GEN, '    if "ALLOC_PRIVILEGED" in f:', "    if False:")),
    ("absent-control-admitted", changed(GEN, '        return False, "NotExported"', '        return True, "OgkmNonPrivileged"')),
    ("unknown-control-given-a-row", changed(GEN, "                if not ruled:\n                    continue",
                                            "                if False:\n                    continue")),
    ("duplicate-id-first-wins", changed(GEN, "    if key not in sink or value < sink[key]:", "    if key not in sink:")),
    # the gate
    ("gate-unlisted-admitted", changed(GATE, "        matches!(self, ControlPermit::Admitted { .. })",
                                       "        !matches!(self, ControlPermit::Denied(Denial::Refused { .. }))")),
    ("gate-refusal-admitted", changed(GATE, "        }) => Err(Denial::Refused { name, why: *why }),",
                                      "        }) => { let _ = why; Ok((name, Basis::OgkmNonPrivileged)) }")),
    ("gate-class-check-skipped", changed(GATE, "        matches!(self, AllocPermit::Admitted { .. })", "        true")),
]

PY_TESTS = ["python3", "-I", "-m", "unittest", "discover", "-s", "tools/api_boundaries", "-p",
            "test_ingress_policy.py", "-v"]
CARGO_TESTS = [
    ["-p", "kf-abi", "--lib", "--", "capability::"],
    ["-p", "kf-rm", "--test", "ingress_parity"],
    ["-p", "kf-rm", "--test", "rmrpc_bridge"],
]


def run(overrides):
    for rel, text in overrides.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)
    killed, rc = set(), 0
    p = subprocess.run(PY_TESTS, cwd=ROOT, capture_output=True, text=True)
    killed |= {"py:" + m for m in re.findall(r"(?m)^(?:FAIL|ERROR): (\w+) \(", p.stderr)}
    rc |= p.returncode
    if any(rel.endswith(".rs") for rel in overrides) or not overrides:
        for args in CARGO_TESTS:
            p = subprocess.run(["cargo", "test"] + args, cwd=ROOT, capture_output=True, text=True)
            out = p.stdout + p.stderr
            if "could not compile" in out:
                raise SystemExit("the mutant does not compile:\n" + out[-800:])
            killed |= set(re.findall(r"(?m)^test (\S+) \.\.\. FAILED$", out))
            rc |= p.returncode
    return sorted(killed), rc


def main():
    try:
        base, rc = run({})
        assert not base and rc == 0, "baseline is not green: %s rc=%s" % (base, rc)
        print("baseline: green")
        weak = []
        wanted = sys.argv[1:]
        for name, (rel, text) in CASES:
            if wanted and name not in wanted:
                continue
            killers, _rc = run({rel: text})
            with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
                f.write(FILES[rel])
            print("%-30s killers=%d %s %s" % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers))
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(CASES), len(weak)))
        return 1 if weak else 0
    finally:
        for rel, text in FILES.items():
            with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
                f.write(text)


if __name__ == "__main__":
    sys.exit(main())
