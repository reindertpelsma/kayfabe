#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the oracle gate of crates/kf-chip/tests/hwref_is_current.rs (STATUS: LIVE, 2026-11-09).

Each mutant is a one-line change of the test file; each must be killed by at least two independently
named failing tests of that same file (the file carries its own tests of `gate` and `oracles_required`).
The long derivation test is skipped here: it needs the oracle tree and takes minutes. The file is
overwritten in place for each run and ALWAYS restored (also on an interrupt); needs a toolchain and the
workspace's dependencies. Run: python3 scripts/ci/oracle_gate_mutants.py   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
TARGET = os.path.join(ROOT, "crates/kf-chip/tests/hwref_is_current.rs")
SOURCE = open(TARGET, encoding="utf-8").read()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    (
        "required-gate-skips-instead-of-failing",
        changed(
            "    if required {\n        Gate::Fail(missing)\n    } else {",
            "    if false {\n        Gate::Fail(missing)\n    } else {",
        ),
    ),
    (
        "optional-gate-fails-instead-of-skipping",
        changed(
            "    if required {\n        Gate::Fail(missing)\n    } else {",
            "    if true {\n        Gate::Fail(missing)\n    } else {",
        ),
    ),
    (
        "missing-compiler-ignored",
        changed("    } else if !gcc_found {", "    } else if false {"),
    ),
    (
        "missing-oracle-ignored",
        changed("    let missing = if !ogkm_found {", "    let missing = if false {"),
    ),
    (
        "only-one-requires",
        changed('!matches!(value, None | Some("" | "0"))', 'matches!(value, Some("1"))'),
    ),
    ("nothing-requires", changed('!matches!(value, None | Some("" | "0"))', "false")),
    ("everything-requires", changed('!matches!(value, None | Some("" | "0"))', "true")),
    (
        "zero-requires",
        changed('!matches!(value, None | Some("" | "0"))', '!matches!(value, None | Some(""))'),
    ),
    (
        "empty-requires",
        changed('!matches!(value, None | Some("" | "0"))', '!matches!(value, None | Some("0"))'),
    ),
]


def run(source):
    with open(TARGET, "w", encoding="utf-8") as f:
        f.write(source)
    p = subprocess.run(
        [
            "cargo",
            "test",
            "-p",
            "kf-chip",
            "--test",
            "hwref_is_current",
            "--",
            "--skip",
            "the_committed_table_is_a_fresh_derivation_from_ogkm_580",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    out = p.stdout + p.stderr
    if "could not compile" in out:
        raise SystemExit("the mutant does not compile:\n" + out[-800:])
    return sorted(set(re.findall(r"(?m)^test (\S+) \.\.\. FAILED$", out))), p.returncode


def main():
    try:
        base, rc = run(SOURCE)
        assert not base and rc == 0, "baseline is not green: %s rc=%s" % (base, rc)
        print("baseline: green")
        weak = []
        for name, source in CASES:
            killers, _rc = run(source)
            print(
                "%-42s killers=%d %s %s"
                % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers)
            )
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(CASES), len(weak)))
        return 1 if weak else 0
    finally:
        with open(TARGET, "w", encoding="utf-8") as f:
            f.write(SOURCE)


if __name__ == "__main__":
    sys.exit(main())
