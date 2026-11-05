#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for scripts/ci/ignore_ladder.py (PROPOSAL, 2026-11-05).

Each mutant must be killed by at least two independently named failing tests of
scripts/ci/test_ignore_ladder.py, like the other *_mutants.py harnesses in this directory.
Run: python3 scripts/ci/ignore_ladder_mutants.py   (exit 0 only if every mutant has >= 2 killers)
"""

import io
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = open(os.path.join(HERE, "ignore_ladder.py")).read()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    ("docker-star-crosses-slash", changed('out += "[^/]*"', 'out += ".*"')),
    (
        "parent-directory-match-dropped",
        changed(
            "for k in range(1, len(comps) + 1))\n\n\n@functools.lru_cache(maxsize=None)\ndef _dir_matches",
            "for k in range(len(comps), len(comps) + 1))\n\n\n@functools.lru_cache(maxsize=None)\ndef _dir_matches",
        ),
    ),
    (
        "doublestar-needs-one-directory",
        (
            changed(
                "for k in range(len(comps) + 1))\n    return bool",
                "for k in range(1, len(comps) + 1))\n    return bool",
            )
            if False
            else changed(
                "any(_match_segs(rest, comps[k:]) for k in range(len(comps) + 1))",
                "any(_match_segs(rest, comps[k:]) for k in range(1, len(comps) + 1))",
            )
        ),
    ),
    ("negation-message-lost", changed('"negation is not allowed"', '"not allowed"')),
    (
        "bare-name-accepted",
        changed('if not (line.startswith("/") or line.startswith("**/")):', "if False:"),
    ),
    ("trailing-slash-accepted", changed('if line.endswith("/"):', "if False:")),
    ("trailing-doublestar-accepted", changed("if i == len(segs) - 1:", "if False:")),
    ("ladder-check-inverted", changed("if line not in hi:", "if line in hi:")),
    (
        "self-ignore-never-checked",
        changed("for name, need in LADDER_SELF.items():", "for name, need in {}.items():"),
    ),
    (
        "hash-drops-mode",
        changed(
            'h.update(p + b"\\0" + m.encode() + b"\\0" + s.encode() + b"\\n")',
            'h.update(p + b"\\0" + s.encode() + b"\\n")',
        ),
    ),
    ("hash-unsorted", changed("    keep.sort()\n", "")),
    ("hash-keeps-ignored", changed("if not matches(patterns, p)]", "if matches(patterns, p)]")),
    (
        "hash-drops-blob-id",
        changed(
            'h.update(p + b"\\0" + m.encode() + b"\\0" + s.encode() + b"\\n")',
            'h.update(p + b"\\0" + m.encode() + b"\\n")',
        ),
    ),
    ("docker-doublestar-needs-directory", changed('out += "(.*/)?"', 'out += "(.*/)"')),
    ("fail-closed-exit-code-lost", changed("return 3", "return 0")),
    ("own-ignore-flag-ignored", changed("if not a.own_ignore:", "if True:")),
    ("git-comparison-skipped", changed("if got != gi:", "if False:")),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('.',pattern='test_ignore_ladder.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'skipped':len(r.skipped),'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run(source):
    with tempfile.TemporaryDirectory() as d:
        shutil.copy(os.path.join(HERE, "test_ignore_ladder.py"), d)
        with open(os.path.join(d, "ignore_ladder.py"), "w") as f:
            f.write(source)
        p = subprocess.run(
            [sys.executable, "-c", COLLECT], cwd=d, capture_output=True, text=True, timeout=300
        )
        assert p.returncode == 0, p.stderr[-300:]
        return json.loads(p.stdout.splitlines()[-1])


def main():
    base = run(SOURCE)
    assert base["killers"] == [], base
    print("baseline: %d tests, %d skipped, 0 failing" % (base["tests"], base["skipped"]))
    weak = []
    for name, src in CASES:
        killers = run(src)["killers"]
        print(
            "%-36s killers=%d %s" % (name, len(killers), "" if len(killers) >= 2 else "  <-- WEAK")
        )
        if len(killers) < 2:
            weak.append(name)
    print("MUTANTS total=%d weak=%d" % (len(CASES), len(weak)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
