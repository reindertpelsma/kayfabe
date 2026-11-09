#!/usr/bin/env python3
"""GPU-free middle-apps policy mutations; baseline green and >=2 named killers each.

Falsifier: a variant survives, an anchor is absent/ambiguous, syntax is invalid,
or a collector fails. Operates only in owned temporary copies; no remote service.
"""

import argparse
import ast
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BROKER = "ci/vast-broker/"
CASES = [
    (
        "missing-bundle-marker-trusted",
        "scripts/kfci/apps_gate.py",
        "            if (\n                not bundle.is_file()\n                or not bundle.stat().st_size\n                or not marker.is_file()\n            ):",
        "            if not marker.is_file():",
        "py",
    ),
    (
        "missing-bundle-not-media",
        "scripts/kfci/apps_gate.py",
        "RetryClass.MEDIA",
        "RetryClass.NOT_BUILT",
        "py",
    ),
    (
        "llm-payload-slot-enabled",
        "scripts/ci/vast_payload.py",
        '"llm_parity": "DESIGN_ONLY"',
        '"llm_parity": "PLUMBING"',
        "py",
    ),
    (
        "llm-worker-slot-enabled",
        BROKER + "defaults.json",
        '"llm_parity": {\n      "tiers": [\n        "large"\n      ],\n      "state": "DESIGN_ONLY"',
        '"llm_parity": {\n      "tiers": [\n        "large"\n      ],\n      "state": "PLUMBING"',
        "node",
    ),
    (
        "large-loses-middle-inheritance",
        BROKER + "src/tier.js",
        "inherits: 'middle'",
        "inherits: 'small'",
        "node",
    ),
    (
        "base-policy-read-required",
        BROKER + "runner.py",
        '"apps_policy_required": True,',
        '"apps_policy_required": False,',
        "py",
    ),
    (
        "policy-binding-unchecked",
        BROKER + "runner.py",
        "@read_apps_policy",
        'return availability.pop("apps_policy")["enabled"]',
        "py",
    ),
    (
        "linux-request-dropped",
        BROKER + "runner.py",
        'request.update(linuxApps=True, winApps="mid")',
        'request.update(winApps="mid")',
        "py",
    ),
    (
        "windows-request-dropped",
        BROKER + "runner.py",
        'request.update(linuxApps=True, winApps="mid")',
        "request.update(linuxApps=True)",
        "py",
    ),
    (
        "missing-stage-pass",
        BROKER + "runner.py",
        "@complete_requested_apps",
        "return True",
        "py",
    ),
    ("linux-count-weakened", BROKER + "runner.py", '"apps": 63', '"apps": 62', "py"),
    (
        "windows-count-weakened",
        BROKER + "runner.py",
        '"windows_apps": 10',
        '"windows_apps": 9',
        "py",
    ),
    (
        "apps-retry-output-lost",
        BROKER + "runner.py",
        '("windows_retry_", "apps_retry_", "windows_apps_retry_")',
        '("windows_retry_",)',
        "py",
    ),
    (
        "guest-failure-retried",
        BROKER + "runner.py",
        'r["name"].startswith(',
        'True or r["name"].startswith(',
        "py",
    ),
    (
        "explicit-linux-opt-in-ignored",
        "scripts/ci/vast_payload.py",
        "@linuxapps_requested",
        'environ.pop("KF_LINUX_APPS", None)\nreturn apps_requested(tier)',
        "py",
    ),
    (
        "linux-opt-in-tier-ignored",
        "scripts/ci/vast_payload.py",
        'value != "1" or tier != "middle"',
        'value != "1"',
        "py",
    ),
    (
        "linux-opt-in-value-ignored",
        "scripts/ci/vast_payload.py",
        'value != "1" or tier != "middle"',
        'tier != "middle"',
        "py",
    ),
    (
        "linux-transport-flag-leaks",
        "scripts/ci/vast_payload.py",
        'environ.pop("KF_LINUX_APPS", None)',
        'environ.get("KF_LINUX_APPS", None)',
        "py",
    ),
    (
        "linux-suffix-reserve-lost",
        "scripts/ci/vast_payload.py",
        "APPS_MIN_REMAINING_MS + (",
        "APPS_MIN_REMAINING_MS + 0 * (",
        "py",
    ),
    (
        "gate-suffix-reserve-lost",
        "scripts/ci/vast_payload.py",
        "WINDOWS_MIN_REMAINING_MS + (",
        "WINDOWS_MIN_REMAINING_MS + 0 * (",
        "py",
    ),
    (
        "mid-cleanup-margin-lost",
        "scripts/ci/vast_payload.py",
        '"mid": 70 * 60000',
        '"mid": 60 * 60000',
        "py",
    ),
    (
        "linux-log-late-bound",
        "scripts/ci/vast_payload.py",
        (
            "lambda rc, names=names, output=output_apps: apps_rows",
            "apps_rows(bounded_text(output), rc, names)",
        ),
        (
            "lambda rc, names=names: apps_rows",
            "apps_rows(bounded_text(output_apps), rc, names)",
        ),
        "py",
    ),
    (
        "media-alias-lost",
        "scripts/ci/vast_payload.py",
        'cls = "MEDIA" if cls == "NOT_BUILT" else cls',
        "cls = cls",
        "py",
    ),
    (
        "media-class-refused",
        "scripts/ci/vast_payload.py",
        'APPS_RETRY_CLASSES = {"MEDIA",',
        "APPS_RETRY_CLASSES = {",
        "py",
    ),
    (
        "owner-default-disabled",
        BROKER + "src/policy.js",
        "env.MIDDLE_APPS_ENABLED !== 'false'",
        "env.MIDDLE_APPS_ENABLED === 'true'",
        "node",
    ),
    (
        "owner-switch-ignored",
        BROKER + "src/policy.js",
        "env.MIDDLE_APPS_ENABLED !== 'false'",
        "true",
        "node",
    ),
    (
        "owner-switch-schema-lost",
        BROKER + "src/policy.js",
        "  'MIDDLE_APPS_ENABLED',\n",
        "",
        "node",
    ),
    (
        "signed-tier-ignored",
        BROKER + "src/tier.js",
        "tierName === 'middle' && enabled",
        "enabled",
        "node",
    ),
    (
        "linux-request-policy-unchecked",
        BROKER + "src/index.js",
        "rentRequest.linuxApps !== appRequest.linuxApps",
        "false",
        "node",
    ),
    (
        "windows-request-policy-unchecked",
        BROKER + "src/index.js",
        "rentRequest.winApps !== appRequest.winApps",
        "false",
        "node",
    ),
    (
        "linux-environment-dropped",
        BROKER + "src/tier.js",
        "env.KF_LINUX_APPS = '1';",
        "void linuxApps;",
        "node",
    ),
    (
        "windows-environment-dropped",
        BROKER + "src/tier.js",
        "env.KF_WINAPPS = winApps;",
        "void winApps;",
        "node",
    ),
    (
        "stored-pass-not-checked",
        BROKER + "src/index.js",
        "payload.result = middleAppsResult(payload.result, r.middleAppsRequested === true);",
        "payload.result = payload.result;",
        "node",
    ),
    (
        "stored-count-not-checked",
        BROKER + "src/tier.js",
        "row.name === name && row.pass === count",
        "row.name === name",
        "node",
    ),
    (
        "stored-guest-failure-relabelled",
        BROKER + "src/tier.js",
        "result.verdict !== 'PASS'",
        "false",
        "node",
    ),
    (
        "stored-retry-row-duplicated",
        BROKER + "src/tier.js",
        "row.name !== missing[0] + '_retry_operator'",
        "true",
        "node",
    ),
    (
        "deployed-switch-not-boolean",
        BROKER + "deploy.mjs",
        "  'MIDDLE_APPS_ENABLED',\n]);\nexport const ALLOWED",
        "]);\nexport const ALLOWED",
        "config",
    ),
    (
        "deployed-switch-not-allowed",
        BROKER + "deploy.mjs",
        "  'MIDDLE_APPS_ENABLED',\n  // Tiers",
        "  // Tiers",
        "config",
    ),
]
COLLECT = """import io,json,sys,unittest
sys.path.insert(0,'scripts/ci')
s=unittest.defaultTestLoader.loadTestsFromName('test_middle_apps')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps(dict(tests=r.testsRun,killers=sorted({getattr(t,'test_case',t).id() for t,_ in r.failures+r.errors}),details={t.id():v[-2500:] for t,v in r.failures+r.errors})))
"""


def collect(root, mode):
    if mode == "py":
        for directory in [root / "scripts/ci", root / "ci/vast-broker"]:
            shutil.rmtree(directory / "__pycache__", ignore_errors=True)
        run = subprocess.run(
            [sys.executable, "-c", COLLECT],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=180,
        )
        if run.returncode:
            raise RuntimeError(run.stderr[-2000:])
        return json.loads(run.stdout.splitlines()[-1])
    tests = ["config-booleans.test.js"] if mode == "config" else ["middle-apps.test.js"]
    run = subprocess.run(
        ["node", "--test", *[str(root / BROKER / "test" / name) for name in tests]],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=180,
    )
    match = re.search(r"^# tests (\d+)$", run.stdout, re.M)
    if not match:
        raise RuntimeError(
            "Node collection failed: " + run.stderr[-2000:] + run.stdout[-2000:]
        )
    return dict(
        tests=int(match[1]),
        killers=sorted(set(re.findall(r"^not ok \d+ - (.+)$", run.stdout, re.M))),
        details=run.stdout[-4000:],
    )


def mutate(text, old, new):
    if isinstance(old, str) and old.startswith("@"):
        nodes = [
            n
            for n in ast.parse(text).body
            if isinstance(n, ast.FunctionDef) and n.name == old[1:]
        ]
        if len(nodes) != 1:
            raise RuntimeError("function anchor absent/ambiguous " + old)
        node = nodes[0]
        lines = text.splitlines(keepends=True)
        return (
            "".join(lines[: node.body[0].lineno - 1])
            + "".join("    " + line + "\n" for line in new.splitlines())
            + "".join(lines[node.end_lineno :])
        )
    edits = [(old, new)] if isinstance(old, str) else list(zip(old, new, strict=True))
    for before, after in edits:
        if text.count(before) != 1:
            raise RuntimeError("anchor absent/ambiguous: " + repr(before))
        text = text.replace(before, after)
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="PM1-middle-mutants-") as directory:
        root = Path(directory)
        ignore = shutil.ignore_patterns("node_modules", ".wrangler", "__pycache__")
        for name in [
            "scripts",
            "ci/vast-broker",
            "ci/vast-reaper",
            "ci/platform-api/migrations",
        ]:
            shutil.copytree(ROOT / name, root / name, ignore=ignore)
        shutil.copyfile(ROOT / "ci/requirements.toml", root / "ci/requirements.toml")
        (root / BROKER / "node_modules").symlink_to(
            ROOT / BROKER / "node_modules", target_is_directory=True
        )
        baselines = {}
        for mode in ("py", "node", "config"):
            result = collect(root, mode)
            if result["killers"]:
                raise RuntimeError("baseline red " + mode + ": " + repr(result))
            baselines[mode] = result["tests"]
        print("baseline green " + json.dumps(baselines), file=sys.stderr, flush=True)
        rows = []
        for name, filename, old, new, mode in CASES:
            if args.only not in name:
                continue
            path = root / filename
            original = path.read_text()
            changed = mutate(original, old, new)
            if changed == original:
                raise RuntimeError("empty mutant " + name)
            if filename.endswith(".py"):
                ast.parse(changed)
            elif filename.endswith(".json"):
                json.loads(changed)
            path.write_text(changed)
            try:
                if filename.endswith((".js", ".mjs")):
                    checked = subprocess.run(
                        ["node", "--check", str(path)],
                        capture_output=True,
                        text=True,
                        timeout=30,
                    )
                    if checked.returncode:
                        raise RuntimeError(
                            "syntax error " + name + ": " + checked.stderr
                        )
                result = collect(root, mode)
            finally:
                path.write_text(original)
            killers = result["killers"]
            if any("_FailedTest" in name for name in killers):
                raise RuntimeError("import failed " + repr(result))
            rows.append(dict(mutant=name, count=len(killers), killers=killers))
            print(
                name + ": " + str(len(killers)) + " killers",
                file=sys.stderr,
                flush=True,
            )
        print(json.dumps(dict(baselines=baselines, cases=rows), indent=2))
        return int(any(row["count"] < 2 for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
