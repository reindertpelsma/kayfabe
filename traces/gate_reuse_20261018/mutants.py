#!/usr/bin/env python3
"""Private-copy gate and release proof mutants; no network or provider calls."""
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BROKER = "ci/vast-broker/"
CASES = []


def add(name, file, old, new, tests="gate-reuse.test.js"):
    source = (ROOT / BROKER / file).read_text()
    assert source.count(old) == 1, (name, old)
    CASES.append((name, file, source, source.replace(old, new), tests))


add("tree-mismatch", "gate_reuse.py", "if candidate_tree != tree:", "if False:")
add("non-success-status", "gate_reuse.py", 'if status["state"] != "success":', "if False:")
add(
    "wrong-creator",
    "gate_reuse.py",
    'if any(status["creator"].get(key) != value for key, value in BOT.items()):',
    "if False:",
)
add(
    "pending-parent",
    "gate_reuse.py",
    'if status["state"] != "success":',
    'if status["state"] not in {"success", "pending"}:',
)
add("pending-ci", "gate_reuse.py", 'or check["status"] != "completed"', "or False")
add("tag-push", "gate_reuse.py", 'or ref != "refs/heads/master"', "or False")
add("pr-event", "gate_reuse.py", 'event != "push"', "False")
add("wrong-ci-app", "gate_reuse.py", 'or check["app"]["id"] != 15368', "or False")
add("wrong-run-revision", "gate_reuse.py", 'or run["head_sha"] != parent', "or False")
add(
    "wrong-workflow",
    "gate_reuse.py",
    'or run["path"] != ".github/workflows/gpu-vast.yml"',
    "or False",
)
add(
    "log-completeness",
    "gate_reuse.py",
    "if any(not group for group in groups) or sorted(stages) != sorted(expected):",
    "if False:",
)
add(
    "log-revision",
    "gate_reuse.py",
    'rungs=[a-z0-9,]+ rev=" + parent[:8]',
    'rungs=[a-z0-9,]+ rev=" + r"[0-9a-f]{8}"',
)
add("release-forged-child", "release_gate.py", 'or latest["creator"]["id"] != 41898282', "or False")
add(
    "release-lost-parent",
    "release_gate.py",
    "proof = policy.metadata(candidate, tree, get)",
    'proof = {"url": latest["target_url"]}',
)
add(
    "replay-contacts-api",
    "gate_reuse.py",
    'elif mode == "replay":',
    'elif mode == "replay":\n        api("/actions/runs/123")',
)

add(
    "replay-script-guard",
    "workflow-guard.js",
    "allSteps[0].trim() === reuseReplayStep.trim()",
    "true",
    "reuse-workflow.test.js",
)
add(
    "broker-branch-guard",
    "workflow-guard.js",
    "(replay && !/^        if: needs.pending.outputs.reused != 'true'$/m.test(steps[0]))",
    "false",
    "reuse-workflow.test.js",
)
add(
    "pending-write-guard",
    "workflow-guard.js",
    "      ![\n        { statuses: 'write' },",
    "      false && ![\n        { statuses: 'write' },",
    "reuse-workflow.test.js",
)

workflow = (ROOT / ".github/workflows/gpu-vast.yml").read_text()
a = workflow.index('if [[ "${REUSED:-false}"')
b = workflow.index("; then", a) + len("; then")
branch = workflow[a:b]
for name, condition in [
    ("pending", '"$PENDING_RESULT" = success'),
    ("policy", '"$POLICY_RESULT" = success'),
    ("policy-check", '"${POLICY_CHECK_RESULT:-}" = success'),
    ("gpu", '"$GPU_RESULT" = success'),
]:
    add(
        "report-reuses-failed-" + name,
        "../../.github/workflows/gpu-vast.yml",
        branch,
        branch.replace(condition, "1 = 1"),
        "reuse-workflow.test.js",
    )


def run(work, test):
    return subprocess.run(
        ["node", "--test", BROKER + "test/" + test],
        cwd=work,
        capture_output=True,
        text=True,
        timeout=120,
    )


def main():
    results = []
    with tempfile.TemporaryDirectory(prefix="wk2-gate-mutants-") as directory:
        work = Path(directory)
        shutil.copytree(
            ROOT / "ci/vast-broker",
            work / "ci/vast-broker",
            ignore=shutil.ignore_patterns("node_modules", ".wrangler", "__pycache__"),
        )
        shutil.copytree(ROOT / ".github/workflows", work / ".github/workflows")
        baseline = run(work, "gate-reuse.test.js")
        assert baseline.returncode == 0, baseline.stdout[-4000:]
        for name, file, original, mutant, test in CASES:
            target = work / BROKER / file
            try:
                target.write_text(mutant)
                result = run(work, test)
                kills = re.findall(r"^not ok [0-9]+ - (.+)$", result.stdout, re.M)
                assert result.returncode and len(kills) >= 2, (name, kills, result.stdout[-3000:])
                results.append({"mutant": name, "kills": len(kills), "tests": kills})
                print(name, len(kills), flush=True)
            finally:
                target.write_text(original)
    (ROOT / "traces/gate_reuse_20261018/mutations.json").write_text(
        json.dumps({"baseline": "PASS", "mutants": results}, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
