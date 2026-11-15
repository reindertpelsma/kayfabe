#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for crates/kf-chan/tests/video_class_coverage.rs (STATUS: LIVE, 2026-11-15).

Each mutant is a one-line change of the test, of the derived video table or of the engine router; each
must be killed by at least two independently named failing tests of that test file. Every file touched
is overwritten in place and ALWAYS restored (also on an interrupt); needs a toolchain and the
workspace's dependencies. Run: python3 scripts/ci/video_class_coverage_mutants.py   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
TEST = "crates/kf-chan/tests/video_class_coverage.rs"
TABLE = "crates/kf-chan/src/video_generated.rs"
HOST = "crates/kf-chan/src/host.rs"
FILES = {}
for rel in (TEST, TABLE, HOST):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()


def changed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old)
    return rel, FILES[rel].replace(old, new)


CASES = [
    # A table entry disappears: its class is a new uncovered class.
    ("drops-a-table-entry", changed(TABLE, "        class: 0xc9b0,", "        class: 0xc9b2,")),
    # A stray allow-list entry for a class that has a table.
    (
        "stray-allow-list-entry",
        changed(TEST, "const KNOWN_GAPS: &[(u32, &str)] = &[\n", 'const KNOWN_GAPS: &[(u32, &str)] = &[\n    (0xc9d1, "x WUKONG_RELAUNCH_TDR_20261115"),\n'),
    ),
    # The OFA gap entry names the wrong class: 0xc9fa is uncovered and unlisted, 0xc9fb is a stray.
    ("allow-list-entry-misnamed", changed(TEST, "        0xc9fa,\n", "        0xc9fb,\n")),
    # The check that an uncovered class needs an entry is removed.
    (
        "removes-the-uncovered-check",
        changed(TEST, "if !is_covered(class) && !seen.contains(&class) {", "if false && !is_covered(class) && !seen.contains(&class) {"),
    ),
    # The check that a covered listed class must leave the list is removed.
    (
        "removes-the-covered-listed-check",
        changed(TEST, "} else if is_covered(class) {", "} else if false {"),
    ),
    # The stray check is removed.
    (
        "removes-the-stray-check",
        changed(TEST, "if !cands.contains(&class) {", "if false {"),
    ),
    # Duplicates are accepted.
    ("accepts-duplicates", changed(TEST, "if !seen.insert(class) {", "if !seen.insert(class) && false {")),
    # The enumeration stops reaching the OFA engine (a new engine kind escapes the audit).
    (
        "router-forgets-ofa",
        changed(
            HOST,
            "        .or_else(|| {\n            kf_abi::submit::ofa_index_of_engine_type(engine).map(|i| (i, Kind::OpticalFlow))\n        })",
            "        .or_else(|| None::<(u32, Kind)>)",
        ),
    ),
    # The predicate the audit uses stops being the table's.
    ("predicate-always-covered", changed(TEST, "VideoClass::of_class(class).is_some()", "true")),
]


def run(overrides):
    for rel, text in overrides.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)
    p = subprocess.run(
        ["cargo", "test", "-p", "kf-chan", "--test", "video_class_coverage"],
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
        base, rc = run({})
        assert not base and rc == 0, "baseline is not green: %s rc=%s" % (base, rc)
        print("baseline: green")
        weak = []
        for name, (rel, text) in CASES:
            killers, _rc = run({rel: text})
            with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
                f.write(FILES[rel])
            print(
                "%-34s killers=%d %s %s"
                % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers)
            )
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
