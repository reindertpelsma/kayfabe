#!/usr/bin/env python3
"""Isolated attempt-admission mutants; no external service calls or deployment."""

import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BROKER = "ci/vast-broker/"
SOURCE = BROKER + "src/github.js"
UNIT = [BROKER + "test/auth-attempt.test.js"]
RUNTIME = [BROKER + "workerd/auth-attempt.test.js"]
LOGS = [BROKER + "test/refusal-log.test.js"]


def run(work, tests, pattern=None):
    command = ["node", "--test", "--test-concurrency=1"]
    if pattern:
        command += ["--test-name-pattern=" + pattern]
    return subprocess.run(command + tests, cwd=work, capture_output=True, text=True, timeout=180)


def main():
    original = (ROOT / SOURCE).read_text()
    cases = []

    def add(name, old, new, tests=UNIT, pattern=None):
        assert original.count(old) == 1, (name, old)
        cases.append((name, original.replace(old, new), tests, pattern))

    add(
        "latest-run-used-for-admission",
        "this.get('actions/runs/' + c.run_id + '/attempts/' + c.run_attempt)",
        "this.run(c.run_id)",
        RUNTIME,
    )
    add(
        "attempt-3-ceiling",
        "const r = await this.get('actions/runs/'",
        "if (Number(c.run_attempt) >= 3) refuse('UNAUTHORIZED', 401);\n"
        "    const r = await this.get('actions/runs/'",
    )
    for field in [
        "id",
        "run_attempt",
        "event",
        "status",
        "repository",
        "head_repository",
        "head_sha",
        "head_branch",
        "triggering_actor",
        "sha",
    ]:
        expression = re.search(r"^      \['" + field + r"', (.+)\],$", original, re.M)
        assert expression, field
        add("binding-removed-" + field, expression[0], "      ['" + field + "', true],")
    add(
        "field-diagnostic-removed",
        "'github:run:' + field",
        "'github:run'",
        LOGS,
        "run refusal logs only",
    )
    add(
        "metadata-value-logged",
        "'github:run:' + field",
        "'github:run:' + field + ':' + String(r[field])",
        LOGS,
        "run refusal logs only",
    )
    results = []
    with tempfile.TemporaryDirectory(prefix="wk1-auth401-mutants-") as directory:
        work = Path(directory)
        for part in ["src", "test", "workerd", "node_modules"]:
            shutil.copytree(
                ROOT / BROKER / part,
                work / BROKER / part,
                ignore=shutil.ignore_patterns("__pycache__", ".wrangler"),
            )
        for part in ["package.json", "defaults.json"]:
            shutil.copy2(ROOT / BROKER / part, work / BROKER / part)
        (work / "scripts/ci").mkdir(parents=True)
        shutil.copy2(ROOT / "scripts/ci/vast_report.py", work / "scripts/ci")
        baseline = run(work, UNIT + RUNTIME + LOGS)
        assert baseline.returncode == 0, baseline.stdout[-4000:] + baseline.stderr[-1000:]
        print("BASELINE PASS", flush=True)
        for name, mutant, tests, pattern in cases:
            target = work / SOURCE
            try:
                target.write_text(mutant)
                process = run(work, tests, pattern)
                kills = re.findall(r"^not ok [0-9]+ - (.+)$", process.stdout, re.M)
                assert process.returncode and len(kills) >= 2, (
                    name,
                    kills,
                    process.stdout[-4000:],
                    process.stderr[-1000:],
                )
                results.append({"mutant": name, "kills": len(kills), "tests": kills})
                print(name + ": " + str(len(kills)) + " killers", flush=True)
            finally:
                target.write_text(original)
    output = {"baseline": "PASS", "mutants": results}
    (ROOT / "traces/vast_auth401_20261018/mutations.json").write_text(
        json.dumps(output, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
