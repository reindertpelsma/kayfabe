#!/usr/bin/env python3
"""Private-copy mutants for the one-policy skip decision and trusted bootstrap."""

import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ".github/workflows/gpu-vast.yml"
GUARD = "ci/vast-broker/workflow-guard.js"
PREFIX = "ci/vast-broker/test/"
ROUND = PREFIX + "round-workflow-v7.test.js"
BOOT = PREFIX + "round-policy-bootstrap.test.js"
CLIENT = PREFIX + "client.test.js"
SKIPS = PREFIX + "edits-not-needed.test.js"
CASES = []
SOURCE = (ROOT / WORKFLOW).read_text()
DECISION = SOURCE[SOURCE.index('          if [[ "$PENDING_RESULT"') :]
DECISION = DECISION.split("; then", 1)[0] + "; then"


def add(name, path, old, new, test=ROUND, pattern="one policy"):
    original = (ROOT / path).read_text()
    assert original.count(old) == 1, (name, old)
    CASES.append((name, path, original, original.replace(old, new), test, pattern))


for name, old, new, pattern in [
    (
        "run-required-skips",
        '"$POLICY_RUN" = 0',
        '( "$POLICY_RUN" = 0 || "$POLICY_RUN" = 1 )',
        "wrong skip run",
    ),
    (
        "missing-output-skips",
        '"$POLICY_RUN" = 0',
        '( "$POLICY_RUN" = 0 || -z "$POLICY_RUN" )',
        "wrong skip missing",
    ),
    (
        "failed-policy-skips",
        '"$POLICY_RESULT" = success',
        '( "$POLICY_RESULT" = success || "$POLICY_RESULT" = failure )',
        "wrong skip failed",
    ),
    ("failed-gpu-skips", '"$GPU_RESULT" = skipped', "true", "wrong skip gpu-failed"),
    ("failed-pending-skips", '"$PENDING_RESULT" = success', "true", "wrong skip pending-failed"),
]:
    assert DECISION.count(old) == 1
    add(name, WORKFLOW, DECISION, DECISION.replace(old, new), pattern=pattern)
add("unverified-pass", WORKFLOW, '"$BROKER_VERIFIED" = 1', "true", CLIENT, "unverified PASS")
add(
    "worker-supplies-skip",
    WORKFLOW,
    '( "$state" = GRACE || "$state" = NONE )',
    "true",
    SKIPS,
    "Worker-sent",
)
add(
    "duplicate-job-accepted",
    GUARD,
    "['pending', 'policy', 'gpu', 'report']",
    "gpu.map((job) => job.id)",
    pattern="guard refuses a duplicate",
)
action = next(
    line for line in (ROOT / GUARD).read_text().splitlines() if ".test(policy.body)" in line
)
add("policy-action-accepted", GUARD, action, "    false ||", pattern="guard refuses even pinned")
add(
    "policy-write-accepted",
    GUARD,
    "Object.entries({ contents: 'read', 'pull-requests': 'read' }).sort()",
    "Object.entries(policy.grants).sort()",
    pattern="guard keeps read-only",
)
add(
    "report-policy-dependency-removed",
    GUARD,
    "['pending', 'policy', 'gpu']",
    "report.needs",
    pattern="guard requires its dependency report",
)
add(
    "broker-policy-dependency-removed",
    GUARD,
    "['pending', 'policy']",
    "primary.needs",
    pattern="guard requires its dependency gpu",
)
policy = SOURCE.split("  policy:\n")[1].split("  gpu:\n")[0]
for name, old, new in [
    (
        "selected-source-downloaded",
        "$GITHUB_SHA/ci/vast-broker/$file",
        "$TARGET_SHA/ci/vast-broker/$file",
    ),
    ("https-cap-removed", "--proto '=https' ", ""),
    ("time-cap-removed", "--max-time 20 ", ""),
    ("size-cap-removed", "--max-filesize 131072 ", ""),
    ("environment-inherited", "env -i PATH=", "env PATH="),
]:
    assert policy.count(old) == 1
    add(name, WORKFLOW, policy, policy.replace(old, new), BOOT, "actual policy bootstrap")


def run(work, files, pattern=None):
    command = ["node", "--test", "--test-concurrency=1"]
    if pattern:
        command.append("--test-name-pattern=" + pattern)
    return subprocess.run(command + files, cwd=work, capture_output=True, text=True, timeout=120)


def main():
    results = []
    with tempfile.TemporaryDirectory(prefix="wk1-onepolicy-mutants-") as directory:
        work = Path(directory)
        shutil.copytree(
            ROOT / "ci/vast-broker",
            work / "ci/vast-broker",
            ignore=shutil.ignore_patterns("node_modules", "__pycache__", ".wrangler"),
        )
        shutil.copytree(
            ROOT / "ci/vast-broker/node_modules/jose", work / "ci/vast-broker/node_modules/jose"
        )
        shutil.copytree(ROOT / ".github/workflows", work / ".github/workflows")
        shutil.copytree(
            ROOT / "scripts/ci", work / "scripts/ci", ignore=shutil.ignore_patterns("__pycache__")
        )
        (work / "scripts/bench").mkdir(parents=True)
        shutil.copy2(ROOT / "scripts/bench/native_xid_guard.py", work / "scripts/bench")
        baseline = run(work, [ROUND, BOOT, CLIENT, SKIPS])
        assert baseline.returncode == 0, baseline.stdout[-4000:] + baseline.stderr[-1000:]
        print("BASELINE PASS", flush=True)
        for name, path, original, mutant, test, pattern in CASES:
            target = work / path
            try:
                target.write_text(mutant)
                result = run(work, [test], pattern)
                killers = re.findall(
                    r"^not ok [0-9]+ - ((?:one policy|round actual|final status|W2:).*)$",
                    result.stdout,
                    re.M,
                )
                assert result.returncode and len(killers) >= 2, (
                    name,
                    killers,
                    result.stdout[-4000:],
                    result.stderr[-1000:],
                )
                item = {"mutant": name, "kills": len(killers), "tests": killers}
                results.append(item)
                print(json.dumps(item), flush=True)
            finally:
                target.write_text(original)
    print(json.dumps({"baseline": "PASS", "mutants": results}, indent=2))


if __name__ == "__main__":
    main()
