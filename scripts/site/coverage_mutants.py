#!/usr/bin/env python3
"""Mutants of scripts/site/coverage_from_traces.py (STATUS: LIVE, 2026-11-05). Each edit must make
test_coverage_from_traces.py fail. Run from the repository root:
    python3 scripts/site/coverage_mutants.py
The generator is restored after every mutant, also on an error or an interrupt."""

from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
TARGET = HERE / "coverage_from_traces.py"
MUTANTS = [
    ("two different Windows verdict lines are accepted", "if len(found) != 1:", "if len(found) == 0:"),
    ("a missing birth census still passes the Linux bar", "and census is not None", "and True"),
    ("a short bare suite still passes", "and b[0] == b[3] and", "and b[0] >= 0 and"),
    ("a short thin-guest suite still passes", "and f[0] == f[4] and", "and f[0] >= 0 and"),
    ("a failed gate summary is ignored", "g[1] == 0 and g[0] == 9", "g[0] == 9"),
    ("the Worker's own verdict is not required", 'facts["verdict"] != "PASS" or facts["verified"] != "1"', "False"),
    ("a Worker job with a FAIL summary still passes", 'and final.group(1) == "PASS")\n    failed', ")\n    failed"),
    ("imported and Worker rows share a group", "key = (r[\"source\"], r[\"rev\"])", "key = (r[\"rev\"],)"),
    ("revisions share a group", "key = (r[\"source\"], r[\"rev\"])", "key = (r[\"source\"],)"),
    ("the Worker set is labelled imported", '"source": WORKER,\n        "date": "2026-11-05"', '"source": IMPORTED,\n        "date": "2026-11-05"'),
    ("a failed run is counted as a pass", '{"PASS": "pass", "FAIL": "fail", "RETRY": "retry"}', '{"PASS": "pass", "FAIL": "pass", "RETRY": "retry"}'),
    ("an unreadable run is counted as a pass", '.get(r["verdict"], "unknown")', '.get(r["verdict"], "pass")'),
    ("an uncovered die loses its reason", "uncovered_cell(UNCOVERED[die][0],", 'uncovered_cell("",'),
    ("a trio directory is not warned about", "for p in PENDING_SETS if (TRACES / p).exists()]", "for p in PENDING_SETS if False]"),
    ("the evidence path is dropped", '"evidence": str(Path("traces") / SETS[r["set"]]["dir"]),', '"evidence": "traces/",'),
    ("the headline flag is always true", 'r["rev"] == SETS[r["set"]]["headline_rev"]', "True"),
]


def main():
    original = TARGET.read_text()
    survivors = []
    try:
        for name, old, new in MUTANTS:
            if original.count(old) != 1:
                print(f"BAD MUTANT (pattern count {original.count(old)}): {name}")
                survivors.append(name)
                continue
            TARGET.write_text(original.replace(old, new))
            # A fresh bytecode cache per mutant: same-size edits within one second would otherwise
            # reuse the previous mutant's compiled module.
            cache = tempfile.mkdtemp(prefix="cov-mutant-")
            run = subprocess.run(
                [sys.executable, "-X", f"pycache_prefix={cache}", "-m", "unittest", "discover", "-s", str(HERE), "-p", "test_coverage_from_traces.py"],
                capture_output=True, text=True)
            killed = run.returncode != 0
            print(("killed   " if killed else "SURVIVED ") + name)
            if not killed:
                survivors.append(name)
    finally:
        TARGET.write_text(original)
    print(f"{len(MUTANTS) - len(survivors)}/{len(MUTANTS)} killed")
    return 1 if survivors else 0


if __name__ == "__main__":
    sys.exit(main())
