#!/usr/bin/env python3
"""Mutation checks of the bandwidth table guard, the calibration and the transferred-bytes record (stdlib, GPU-free).

STATUS: DESIGN-ONLY, 2026-11-16. Every mutant needs at least two named killers. Each mutant is one exact-once text replacement in a
private copy of the tree; test_bandwidth_check.py and test_net_bytes.py run against the copy and the failing test names are the
killers. The unmutated copy must be green first.

    python3 scripts/ci/bandwidth_mutants.py [--jobs N] [substring-of-mutant-name]
"""

import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FILES = {
    "check": "scripts/ci/bandwidth_check.py",
    "cal": "scripts/ci/bandwidth_calibrate.py",
    "payload": "scripts/ci/vast_payload.py",
    "report": "scripts/ci/vast_report.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITES = ["test_bandwidth_check", "test_net_bytes"]
COPY_DIRS = ["scripts/ci", "scripts/kfci"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "scripts/bench/windows/propimage_runspec.py",
    "scripts/bench/windows/propimage_registry.py",
    "scripts/bench/windows/app_defs/core_apps.py",
    "ci/vast-broker/planner/schemas/instruction.schema.json",
    "ci/vast-broker/release-matrix.json",
    "ci/vast-broker/propimages.json",
    "ci/vast-broker/bandwidth.json",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""
C, K, P, R = "test_bandwidth_check.Committed.", "test_bandwidth_check.Refusals.", "test_bandwidth_check.Calibrate.", "test_net_bytes."
# [name, file key, exact-once text, replacement]
MUTANTS = [
    ("check-stage-missing-not-reported", "check", 'for name in sorted(set(stage_names) - set(stages)):\n        errors.append("STAGE_MISSING:" + name)',
     'for name in []:\n        errors.append("STAGE_MISSING:" + name)'),
    ("check-stage-unknown-not-reported", "check", 'for name in sorted(set(stages) - set(stage_names)):\n        errors.append("STAGE_UNKNOWN:" + name)',
     'for name in []:\n        errors.append("STAGE_UNKNOWN:" + name)'),
    ("check-estimate-never-stale", "check", "if day is not None and (today - day).days > MAX_ESTIMATE_AGE_DAYS:", "if False:"),
    ("check-estimate-note-not-required", "check", 'if not (type(row.get("note")) is str and row["note"].strip()):', "if False:"),
    ("check-unknown-entry-field-allowed", "check", 'for key in sorted(set(entry) - ENTRY_KEYS):', "for key in []:"),
    ("check-class-not-checked", "check", 'if "class" in entry and entry["class"] not in CLASSES:', "if False:"),
    ("check-date-not-checked", "check", 'if "date" in row and day is None:', "if False:"),
    ("check-copied-pin-size-allowed", "check", 'elif resolved[0] in ("propimage", "driver") and "down_bytes" in entry:', "elif False:"),
    ("cal-negative-remainder-kept", "cal", "rest = 0\n            target.update", "pass\n            target.update"),
    ("cal-upload-not-updated", "cal", 'stage["upload"].update(up_bytes=_up(tx, UP_ROUND),', 'stage["upload"].update(up_bytes=stage["upload"]["up_bytes"],'),
    ("cal-runs-not-averaged", "cal", "rx = sum(s[0] for s in samples) / len(samples)", "rx = samples[0][0]"),
    ("payload-rx-column", "payload", "return best[1], int(f[0]), int(f[8])", "return best[1], int(f[8]), int(f[0])"),
    ("payload-default-route-not-lowest-metric", "payload", 'and (best is None or int(f[6]) < best[0]):', "and best is None:"),
    ("payload-interface-change-recorded", "payload", "if before is None or after is None or before[0] != after[0]:", "if before is None or after is None:"),
    ("report-negative-bytes-accepted", "report", "all(type(n) is int and 0 <= n <= 1 << 50 for n in v.values())", "all(type(n) is int for n in v.values())"),
]


def tree(parent, name, overrides):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    for rel in COPY_DIRS:
        shutil.copytree(ROOT / rel, root / rel, ignore=shutil.ignore_patterns("__pycache__"))
    for rel in COPY_FILES:
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, root / rel)
    for key, text in overrides.items():
        (root / FILES[key]).write_text(text)
    return root


def run_tests(root):
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT, *SUITES], cwd=root / "scripts/ci", capture_output=True, text=True, timeout=900,
                          env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), "PYTHONDONTWRITEBYTECODE": "1"})
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, old, new = mutant
    text = SOURCES[key]
    if text.count(old) != 1:
        return {"mutant": name, "count": 0, "killers": [], "error": "anchor x%d: %r" % (text.count(old), old[:60])}
    root = tree(parent, "m", {key: text.replace(old, new)})
    killers = sorted(set(run_tests(root)["killers"]))
    shutil.rmtree(root, ignore_errors=True)
    print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-bandwidth-mutants-") as parent:
        baseline = run_tests(tree(parent, "base", {}))
        if baseline["killers"]:
            print("BASELINE RED: %s" % baseline["killers"], file=sys.stderr)
            return 1
        print("baseline green: %d tests" % baseline["tests"], file=sys.stderr, flush=True)
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(lambda m: one(parent, m), chosen))
    bad = [r for r in rows if r["count"] < 2]
    print(json.dumps({"cases": rows}, indent=2))
    for row in bad:
        print("TOO FEW KILLERS: %(mutant)s (%(count)d) %(error)s" % dict(row, error=row.get("error", "")), file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
