#!/usr/bin/env python3
"""GPU-free KFCI mutations. Every replacement must be killed by >=2 test methods.

Falsifier: baseline red, an anchor absent/duplicated, or a mutant with <2 killers.
Copies only this lane's library and tests; never rewrites the source worktree.
"""

import argparse
import ast
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
SCRIPTS = HERE.parent

# Names are deliberately rule-oriented; killing tests are reported, not hardcoded.
CASES = [
    (
        "wire-javascript-regex-ascii-lost",
        "planner_assignment.py",
        "re.search(pattern, value, re.ASCII)",
        "re.search(pattern, value)",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-javascript-end-anchor-lost",
        "planner_assignment.py",
        'pattern = pattern[:-1] + r"\\Z"',
        "pattern = pattern",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-javascript-string-length-lost",
        "planner_assignment.py",
        'return len(value.encode("utf-16-le", errors="surrogatepass")) // 2',
        "return len(value)",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "apps-image-permissions-widened",
        "apps_gate.py",
        "source.stat().st_mode & 0o777",
        "0o666",
        "ImageCopy",
    ),
    (
        "wire-seed-steps-silently-skipped",
        "planner_assignment.py",
        "if unsupported_seed:",
        "if False:",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-duplicate-driver-pin-admitted",
        "planner_assignment.py",
        'if len(pins) != len(value["driver_pins"]):',
        "if False:",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-marker-newline-admitted",
        "planner_assignment.py",
        (
            'if not all(\n        re.fullmatch(r"[A-Za-z0-9_.:-]{1,100}", name) for name in [value["id"], *ids]\n    ):',
            'pattern = pattern[:-1] + r"\\Z"',
        ),
        ("if False:", "pattern = pattern"),
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-worst-case-over-reservation-admitted",
        "planner_assignment.py",
        'if value["worst_case_minutes"] > value["reservation_minutes"]:',
        "if False:",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-schema-validation-bypassed",
        "planner_assignment.py",
        "error = check(SCHEMA, value)",
        "error = None",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-proposed-runs",
        "planner_assignment.py",
        'if value["status"] != "executable":',
        "if False:",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-unpinned-or-incomplete-driver-runs",
        "planner_assignment.py",
        'if pin["status"] != "PINNED" or any(\n            pin[key] is None for key in ("version", "url", "sha256", "size")\n        ):',
        "if False:",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-duplicate-instance-id-admitted",
        "planner_assignment.py",
        "if len(ids) != len(set(ids)):",
        "if False:",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-undeclared-driver-admitted",
        "planner_assignment.py",
        'if any(pin not in pins for pin in entry["drivers"].values()):',
        "if False:",
        "test_kfci_planner.PlannerSchema",
    ),
    (
        "wire-artifact-metadata-forged",
        "catalog.py",
        "if needs != set(step.needs) or produces != set(step.produces):",
        "if False:",
        "test_kfci_planner.PlannerExecution",
    ),
    (
        "wire-artifact-order-ignored",
        "catalog.py",
        'missing = set(entry["needs"]) - available',
        "missing = set()",
        "test_kfci_planner.PlannerExecution",
    ),
    (
        "wire-instance-names-lost",
        "runner.py",
        'name = entry["id"]',
        "name = step.id",
        "test_kfci_planner.PlannerExecution",
    ),
    (
        "wire-budget-ignored",
        "runner.py",
        'seconds = plan["reservation_minutes"] * 60',
        "seconds = 999999",
        "test_kfci_planner.PlannerExecution",
    ),
    (
        "wire-declared-estimates-ignored",
        "runner.py",
        'max(step.estimated_minutes, entry.get("minutes", 0))',
        "step.estimated_minutes",
        "test_kfci_planner.PlannerExecution",
    ),
    (
        "apps-product-environment-dropped",
        "apps_gate.py",
        'key.startswith(("KF3_", "KAYFABE_"))',
        "False",
        "Contracts",
    ),
    (
        "assignment-duplicate-json-keys-admitted",
        "assignment.py",
        "if key in result:",
        "if False:",
        "Assignments Contracts",
    ),
    (
        "assignment-duplicate-step-id-admitted",
        "assignment.py",
        "or name in seen",
        "",
        "Assignments Contracts",
    ),
    (
        "assignment-unknown-envelope-field-admitted",
        "assignment.py",
        "or value.keys() - allowed",
        "",
        "Assignments Contracts",
    ),
    (
        "dependency-cycle-unclear",
        "catalog.py",
        "if name in visiting:",
        "if False:",
        "Dependencies Contracts",
    ),
    (
        "dependency-ambiguity-guessed",
        "catalog.py",
        "if len(choices) != 1:",
        "if False:",
        "Dependencies Contracts",
    ),
    (
        "dependency-missing-producer-unclear",
        "catalog.py",
        "if not choices:",
        "if False:",
        "Dependencies Contracts",
    ),
    (
        "runner-overdue-pass-admitted",
        "runner.py",
        "self.clock() > deadline and final.verdict is Verdict.PASS",
        "False",
        "Execution Contracts",
    ),
    (
        "process-inherited-session",
        "proc.py",
        "start_new_session=True",
        "start_new_session=False",
        "Contracts",
    ),
    (
        "cancel-handler-missing",
        "cli.py",
        "signal.signal(sig, interrupted)",
        "signal.signal(sig, signal.SIG_DFL)",
        "Cancellation",
    ),
    (
        "cancel-swallowed-by-selector",
        "step.py",
        "class Cancelled(RuntimeError):",
        "class Cancelled(InterruptedError):",
        "Cancellation",
    ),
    (
        "assignment-required-fields",
        "assignment.py",
        "not required <= value.keys() or value.keys() - allowed",
        "value.keys() - allowed",
        "Assignments Contracts",
    ),
    (
        "assignment-budgets-unchecked",
        "assignment.py",
        'if not number(value["budget_seconds"], positive=True):',
        "if False:",
        "Assignments",
    ),
    (
        "assignment-identifiers-unchecked",
        "assignment.py",
        'if not identifier(value["id"]):',
        "if False:",
        "Assignments",
    ),
    (
        "dependency-order-unchecked",
        "catalog.py",
        "missing = set(step.needs) - available",
        "missing = set()",
        "Dependencies Execution",
    ),
    (
        "runner-continues-after-red",
        "runner.py",
        "if final.verdict is not Verdict.PASS:\n                break",
        "if False:\n                break",
        "Execution",
    ),
    (
        "runner-admission-guard-removed",
        "runner.py",
        "guarded = end - start < estimate * 60 or end <= start",
        "guarded = False",
        "Execution",
    ),
    (
        "runner-reserve-spent",
        "runner.py",
        ' - plan.get("reserve_seconds", 0)',
        "",
        "Execution Contracts",
    ),
    (
        "retry-misreported-as-pass",
        "step.py",
        "Verdict.RETRY: 3",
        "Verdict.RETRY: 0",
        "Execution LaunchModes",
    ),
    (
        "retry-class-optional",
        "step.py",
        "if not isinstance(self.retry_class, RetryClass):",
        "if False:",
        "Execution Contracts",
    ),
    (
        "process-output-unbounded",
        "proc.py",
        "if len(captured) > capture_limit:",
        "if False:",
        "Processes",
    ),
    (
        "process-env-inherited",
        "proc.py",
        "environment = {k: v for k, v in (env or {}).items() if k in allow_env}",
        "environment = dict(os.environ, **(env or {}))",
        "Processes",
    ),
    (
        "process-timeout-status-forged",
        "proc.py",
        "rc = 137 if forced else 124",
        "rc = 0",
        "Processes Contracts",
    ),
    (
        "apps-failure-ignored",
        "apps_gate.py",
        "if failed > 0:",
        "if False:",
        "AppsRules",
    ),
    (
        "apps-missing-rows-pass",
        "apps_gate.py",
        "if unrun > 0:",
        "if False:",
        "AppsRules",
    ),
    (
        "apps-timeout-class-lost",
        "apps_gate.py",
        "matrix_rc in (124, 137)",
        "False",
        "AppsRules",
    ),
    (
        "apps-matrix-rc-ignored",
        "apps_gate.py",
        "passed == count and matrix_rc == 0",
        "passed == count",
        "AppsRules",
    ),
    (
        "apps-harness-row-is-fail",
        "apps_gate.py",
        'elif verdict in ("NOTRUN", "BOOT_FAIL"):',
        "elif False:",
        "AppsRules",
    ),
    (
        "apps-isolated-ignored",
        "apps_gate.py",
        "for path in paths:",
        "for path in paths[1:]:",
        "AppsRules",
    ),
    (
        "apps-note-unbounded",
        "apps_gate.py",
        'row)[:120] or "-"',
        'row) or "-"',
        "AppsRules",
    ),
]

COLLECT = """import io,json,sys,unittest
suite=unittest.defaultTestLoader.loadTestsFromNames([n if n.startswith('test_') else 'test_kfci.'+n for n in sys.argv[1:]])
r=unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps(dict(tests=r.testsRun, killers=[getattr(t,'test_case',t).id() for t,_ in r.failures+r.errors], details={t.id():detail[-6000:] for t,detail in r.failures+r.errors})))
"""


def collect(root, classes):
    # Isolated CLI children may write bytecode despite the collector's env.
    # Every mutation must read its edited source, including same-size edits.
    for directory in ("kfci", "ci"):
        shutil.rmtree(root / directory / "__pycache__", ignore_errors=True)
    ran = subprocess.run(
        [sys.executable, "-c", COLLECT, *classes],
        cwd=root / "ci",
        env={"PATH": os.environ["PATH"], "PYTHONDONTWRITEBYTECODE": "1"},
        capture_output=True,
        text=True,
        timeout=120,
    )
    if ran.returncode:
        raise RuntimeError("mutation test process failed: " + ran.stderr[-800:])
    return json.loads(ran.stdout.splitlines()[-1])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    rows = []
    with tempfile.TemporaryDirectory(prefix="PM1-kfci-mutants-") as directory:
        root = Path(directory) / "scripts"
        shutil.copytree(
            SCRIPTS / "kfci",
            root / "kfci",
            ignore=shutil.ignore_patterns("__pycache__"),
        )
        (root / "ci").mkdir()
        (root / "apps").mkdir()
        shutil.copyfile(SCRIPTS / "apps/run_apps.sh", root / "apps/run_apps.sh")
        shutil.copyfile(HERE / "test_kfci.py", root / "ci/test_kfci.py")
        shutil.copyfile(HERE / "test_kfci_planner.py", root / "ci/test_kfci_planner.py")
        fixture = root / "ci/fixtures/kfci"
        fixture.mkdir(parents=True)
        shutil.copyfile(
            HERE / "fixtures/kfci/planner_assignment.json",
            fixture / "planner_assignment.json",
        )
        baseline = collect(
            root,
            [
                "Assignments",
                "Dependencies",
                "Execution",
                "Processes",
                "LaunchModes",
                "AppsRules",
                "Contracts",
                "Cancellation",
                "ImageCopy",
                "test_kfci_planner.PlannerSchema",
                "test_kfci_planner.PlannerExecution",
                "test_kfci_planner.PlannerStageAliases",
            ],
        )
        if baseline["killers"]:
            raise RuntimeError("baseline red: " + repr(baseline))
        print(
            "baseline green: %d tests" % baseline["tests"], file=sys.stderr, flush=True
        )
        for name, filename, old, new, classes in CASES:
            if args.only not in name:
                continue
            path = root / "kfci" / filename
            original = path.read_text()
            # A rule can have two independent guards. Its admission mutant must
            # disable both; removing just a redundant guard is an equivalent
            # implementation and cannot be killed by an input/output test.
            edits = (
                [(old, new)]
                if isinstance(old, str)
                else list(zip(old, new, strict=True))
            )
            mutated = original
            for before, after in edits:
                if mutated.count(before) != 1:
                    raise RuntimeError("mutant anchor must match exactly once: " + name)
                mutated = mutated.replace(before, after)
            if mutated == original:
                raise RuntimeError("mutant made no change: " + name)
            ast.parse(mutated)
            path.write_text(mutated)
            try:
                result = collect(root, classes.split())
            finally:
                path.write_text(original)
            killers = sorted(set(result["killers"]))
            if any("_FailedTest" in name for name in killers):
                raise RuntimeError("mutation suite failed to import: " + repr(killers))
            rows.append(dict(mutant=name, count=len(killers), killers=killers))
            print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
    print(json.dumps(dict(baseline_tests=baseline["tests"], cases=rows), indent=2))
    return 1 if any(r["count"] < 2 for r in rows) else 0


if __name__ == "__main__":
    sys.exit(main())
