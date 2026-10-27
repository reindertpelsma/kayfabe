#!/usr/bin/env python3
"""Mutation checks for the gate-visibility changes (2026-10-26): every mutant needs two killers.

Targets: ci/vast-broker/runner.py (head+tail display, driven through the real Runner.execute by
ci/vast-broker/test/runner_head_tail_fixture.py) and scripts/bench/box/provision_full.sh (quiet
ladder build, driven by scripts/ci/test_vast_quiet_cargo.py). Stdlib only; GPU-free.

    python3 scripts/ci/visibility_mutants.py [substring-of-mutant-name]
"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
RUNNER = ROOT / "ci/vast-broker/runner.py"
FIXTURE = ROOT / "ci/vast-broker/test/runner_head_tail_fixture.py"
PROVISION = ROOT / "scripts/bench/box/provision_full.sh"

RUNNER_CASES = [
    "markers",
    "count",
    "head-size",
    "tail-size",
    "wide-budget",
    "wide-sizes",
    "ragged",
    "ragged-again",
    "ragged-ramp",
    "flood-lines",
    "scrub",
    "summary-tail",
    "small-log",
    "exact-head",
    "budget-line-kept",
    "constants",
]
QUIET = "test_vast_quiet_cargo.QuietLadderBuild."

# (name, target file, old text (exactly once), new text)
MUTANTS = [
    (
        "head-never-full",
        RUNNER,
        "        if rest:\n            self.head_full = True",
        "        if rest:\n            pass",
    ),
    (
        "tail-never-shown",
        RUNNER,
        "        self.show(tail)\n\n    def pages",
        "        pass\n\n    def pages",
    ),
    ("middle-marker-off", RUNNER, "        if middle > 0:", "        if False:"),
    (
        "middle-count-ignores-the-head",
        RUNNER,
        "middle = len(raw) - self.head_end - len(tail.encode())",
        "middle = len(raw) - len(tail.encode())",
    ),
    (
        "tail-takes-the-start",
        RUNNER,
        "for line in reversed(text.splitlines(keepends=True)):",
        "for line in text.splitlines(keepends=True):",
    ),
    (
        "tail-line-budget-ignored",
        RUNNER,
        "room_lines = max(0, min(TAIL_LINES, 199 - self.display_lines))",
        "room_lines = 10**6",
    ),
    (
        "head-byte-budget-ignored",
        RUNNER,
        "if count >= max_lines or used + cost > max_bytes:\n            break\n        used, count, cut = used + cost, count + 1, cut + len(line)",
        "if count >= max_lines:\n            break\n        used, count, cut = used + cost, count + 1, cut + len(line)",
    ),
    (
        "tail-bypasses-the-scrubber",
        RUNNER,
        "        self.show(tail)\n\n    def pages",
        "        self.emit(tail)\n\n    def pages",
    ),
    ("line-cost-without-prefix", RUNNER, 'return len("[untrusted] ") + min', "return 0 + min"),
    (
        "head-size-constant",
        RUNNER,
        "HEAD_BYTES, HEAD_LINES = 24000, 100",
        "HEAD_BYTES, HEAD_LINES = 12000, 100",
    ),
    (
        "tail-size-constant",
        RUNNER,
        "TAIL_BYTES, TAIL_LINES = 16000, 70",
        "TAIL_BYTES, TAIL_LINES = 8000, 70",
    ),
    (
        "ladder-warnings-reach-the-log",
        PROVISION,
        "> /root/prov/ladder.log 2>&1; LADDER_RC=$?",
        "; LADDER_RC=$?",
    ),
    (
        "ladder-errors-hidden",
        PROVISION,
        "[ \"$LADDER_RC\" -eq 0 ] || { grep -a '^error' /root/prov/ladder.log | head -n 20; tail -n 40 /root/prov/ladder.log; }",
        "true",
    ),
    ("ladder-rc-lost", PROVISION, "LADDER_RC=$?", "LADDER_RC=0"),
    (
        "ladder-lints-capped",
        PROVISION,
        "cargo build -q --release -p kayfabe-rm-ladder",
        "cargo build -q --release --cap-lints allow -p kayfabe-rm-ladder",
    ),
]


def run_runner(mutated):
    env = {"PATH": os.environ["PATH"], "KF_RUNNER_PATH": str(mutated)}
    failed = []
    for case in RUNNER_CASES:
        p = subprocess.run(
            [sys.executable, "-I", str(FIXTURE), case], env=env, capture_output=True, timeout=120
        )
        if p.returncode:
            failed.append(case)
    return failed


def run_quiet(mutated):
    env = dict(os.environ, KF_PROVISION_FULL=str(mutated))
    p = subprocess.run(
        [sys.executable, "-m", "unittest", "-v", "test_vast_quiet_cargo"],
        cwd=HERE,
        env=env,
        capture_output=True,
        text=True,
        timeout=300,
    )
    failed = []
    for line in (p.stdout + p.stderr).splitlines():
        if line.startswith(("FAIL: ", "ERROR: ")):
            failed.append(line.split()[1])
    return failed


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    rows, bad = [], 0
    with tempfile.TemporaryDirectory(prefix="kf-visibility-") as directory:
        for target, runner in ((RUNNER, run_runner), (PROVISION, run_quiet)):
            work = Path(directory) / ("base-" + target.name)
            work.write_text(target.read_text())
            baseline = runner(work)
            if baseline:
                print("BASELINE RED for %s: %s" % (target.name, baseline), file=sys.stderr)
                return 1
        for name, target, old, new in MUTANTS:
            if only not in name:
                continue
            source = target.read_text()
            assert source.count(old) == 1, (name, source.count(old))
            work = Path(directory) / ("mut-" + name + "-" + target.name)
            work.write_text(source.replace(old, new))
            killers = (run_runner if target == RUNNER else run_quiet)(work)
            rows.append({"mutant": name, "killers": killers, "count": len(killers)})
            print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
            bad += len(killers) < 2
    print(json.dumps({"cases": rows}, indent=2))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
