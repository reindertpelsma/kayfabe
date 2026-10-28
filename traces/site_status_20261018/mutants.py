#!/usr/bin/env python3
"""Isolated Site policy/status/Pages mutants. All outbound APIs remain simulated."""
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BROKER = "ci/vast-broker/"
CASES = []


def add(name, file, old, new, test="site-policy.test.js"):
    path = file if file.startswith(".github/") else BROKER + file
    source = (ROOT / path).read_text()
    assert source.count(old) == 1, (name, old)
    CASES.append((name, path, source, source.replace(old, new), test))


for directory in ["site/", "docs/"]:
    add(
        "ignore-" + directory[:-1],
        "site_policy.py",
        'path.startswith(("site/", "docs/"))',
        'path.startswith(("' + ("docs/" if directory == "site/" else "site/") + '",))',
    )
add("ignore-workflow", "site_policy.py", 'or path == ".github/workflows/site.yml"', "or False")
add("ignore-renamed-source", "site_policy.py", 'paths.append(file["previous_filename"])', "pass")
add(
    "uncertain-paths-skip",
    "site_policy.py",
    'unknown = {"run": True, "deploy": False, "known": False}',
    'unknown = {"run": False, "deploy": False, "known": True}',
)
add(
    "uncertain-push-deploys",
    "site_policy.py",
    'unknown = {"run": True, "deploy": False, "known": False}',
    'unknown = {"run": True, "deploy": True, "known": False}',
)
add(
    "pr-deploys",
    "site_policy.py",
    'return {"run": needed, "deploy": False, "known": True}',
    'return {"run": needed, "deploy": needed, "known": True}',
)
add(
    "publication-skips",
    "site_policy.py",
    'return {"run": True, "deploy": True, "known": True}',
    'return {"run": False, "deploy": False, "known": True}',
)
add("pr-count-unbounded", "site_policy.py", "or not 0 <= count <= 3000", "or not 0 <= count")
add("push-files-truncated", "site_policy.py", "or len(files) >= 300", "or False")
add(
    "site-tree-mismatch",
    "site_policy.py",
    'or candidate["commit"]["tree"]["sha"] != tree',
    "or False",
)
add(
    "site-foreign-status",
    "site_policy.py",
    'or any(status["creator"].get(k) != v for k, v in BOT.items())',
    "or False",
)
add("site-pending-status", "site_policy.py", 'status["state"] != "success"', "False")
add("site-failed-ci", "site_policy.py", 'and ci["conclusion"] == "success"', "and True")
add("site-pending-ci", "site_policy.py", 'and ci["status"] == "completed"', "and True")
add("site-forged-ci", "site_policy.py", 'and ci["app"]["id"] == 15368', "and True")
add("site-wrong-run-sha", "site_policy.py", 'or run["head_sha"] != parent', "or False")
add(
    "site-failed-run",
    "site_policy.py",
    '\n                    or run["conclusion"] != "success"',
    "\n                    or False",
)
add("site-tag-reuse", "site_policy.py", 'or ref != "refs/heads/master"', "or False")
add("site-pr-reuse", "site_policy.py", 'name != "push"', "False")
add(
    "site-foreign-source",
    "site_policy.py",
    '\n            or run["repository"]["full_name"] != REPO',
    "\n            or False",
)
add(
    "site-other-workflow",
    "site_policy.py",
    'or run["path"] != ".github/workflows/site.yml"',
    "or False",
)
add(
    "late-start-clears-final",
    "site_policy.py",
    'existing["state"] in {"success", "error", "failure"}',
    "False",
)
add(
    "late-running-clears-final",
    "site_policy.py",
    'event["action"] != "completed" and run["status"] != "completed"',
    'event["action"] != "completed"',
)
add(
    "site-skipped-needed-build",
    "site_policy.py",
    'elif build["conclusion"] == "skipped" and policy and policy["known"] and not policy["run"]:',
    'elif build["conclusion"] == "skipped" and policy and policy["known"]:',
)
add(
    "tag-tooling-executes",
    ".github/workflows/site.yml",
    "POLICY_REF=$(gh api repos/reindertpelsma/kayfabe/commits/master --jq '.sha')",
    "true",
    "site-bootstrap.test.js",
)
add(
    "status-guard-accepts-source",
    "workflow-guard.js",
    "actual !== expected",
    "false",
    "site-always.test.js",
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
    with tempfile.TemporaryDirectory(prefix="wk2-site-mutants-") as directory:
        work = Path(directory)
        shutil.copytree(
            ROOT / "ci/vast-broker",
            work / "ci/vast-broker",
            ignore=shutil.ignore_patterns("node_modules", ".wrangler", "__pycache__"),
        )
        shutil.copytree(ROOT / ".github/workflows", work / ".github/workflows")
        for name in {case[-1] for case in CASES}:
            result = run(work, name)
            assert result.returncode == 0, result.stdout[-4000:]
        print("BASELINE PASS", flush=True)
        for name, path, original, mutant, test in CASES:
            target = work / path
            try:
                target.write_text(mutant)
                result = run(work, test)
                kills = re.findall(r"^not ok [0-9]+ - (.+)$", result.stdout, re.M)
                assert result.returncode and len(kills) >= 2, (name, kills, result.stdout[-3000:])
                results.append({"mutant": name, "kills": len(kills), "tests": kills})
                print(name, len(kills), flush=True)
            finally:
                target.write_text(original)
    (ROOT / "traces/site_status_20261018/mutations.json").write_text(
        json.dumps({"baseline": "PASS", "mutants": results}, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
