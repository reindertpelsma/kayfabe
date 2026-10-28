"""Isolated final-tree mutations; real source, simulated APIs, no provider calls."""

import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BROKER = "ci/vast-broker/"
cases = [
    (
        "site-pr-head-reread",
        "site_policy.py",
        'current["head"]["sha"] != sha',
        "False",
        "site-head-race.test.js",
    ),
    (
        "site-pr-base-reread",
        "site_policy.py",
        'current["base"].get("sha") != pr["base"].get("sha")',
        "False",
        "site-head-race.test.js",
    ),
    (
        "site-pr-count-reread",
        "site_policy.py",
        'current["changed_files"] != count',
        "False",
        "site-head-race.test.js",
    ),
    (
        "gpu-stage-count-only",
        "gate_reuse.py",
        "sorted(stages) != sorted(expected)",
        "len(groups[-1]) != 4",
        "gate-reuse.test.js",
    ),
    (
        "site-pattern-removed",
        "../../.github/workflows/gpu-vast.yml",
        ',"site/**"',
        "",
        "site-gpu-skip.test.js",
    ),
    (
        "site-prefix-unanchored",
        "trigger-policy.mjs",
        "new RegExp(expression + '$')",
        "new RegExp(expression.slice(1) + '$')",
        "site-gpu-skip.test.js",
    ),
    (
        "site-nested-anywhere",
        "../../.github/workflows/gpu-vast.yml",
        '"site/**"',
        '"**/site/**"',
        "site-gpu-skip.test.js",
    ),
    (
        "site-depth-one",
        "../../.github/workflows/gpu-vast.yml",
        '"site/**"',
        '"site/*"',
        "site-gpu-skip.test.js",
    ),
    (
        "root-md-requires-slash",
        "trigger-policy.mjs",
        "expression += '(?:.*/)?'",
        "expression += '.*/'",
        "site-gpu-skip.test.js",
    ),
    (
        "site-any-skipped-file",
        "trigger-policy.mjs",
        "return files.every(",
        "return files.some(",
        "site-gpu-skip.test.js",
    ),
]
for file, test, name, variable in [
    ("gate_reuse.py", "gate-reuse.test.js", "GPU", "check"),
    ("site_policy.py", "site-policy.test.js", "Site", "ci"),
]:
    cases += [
        (
            name + "-CI-workflow",
            file,
            'or run["path"] != ".github/workflows/ci.yml"',
            "or False",
            test,
        ),
        (
            name + "-CI-ambiguity",
            file,
            "for " + variable + " in matches:",
            "for " + variable + " in matches[:1]:",
            test,
        ),
        (
            name + "-CI-suite",
            file,
            'or run["check_suite_id"] != ' + variable + '["check_suite"]["id"]',
            "or False",
            test,
        ),
    ]
cases += [
    (
        "gpu-complete-schema",
        "gate_reuse.py",
        'sorted(modes) != ["guest", "host"] or final_blocks != 1',
        "False",
        "gate-reuse.test.js",
    ),
    (
        "gpu-ambiguous-rental",
        "gate_reuse.py",
        "any(len(groups[index]) != 1 for index in [0, 1, 3])",
        "False",
        "gate-reuse.test.js",
    ),
    (
        "prepare-premature-success",
        "gate_reuse.py",
        '"state": "pending"',
        '"state": "success"',
        "gate-reuse.test.js",
    ),
    (
        "release-incomplete-child",
        "release_gate.py",
        "or not completed_replay(sha, get)",
        "or False",
        "gate-reuse.test.js",
    ),
]

rows = []
with tempfile.TemporaryDirectory(prefix="wk2-final-mutants-") as directory:
    work = Path(directory)
    shutil.copytree(
        ROOT / "ci/vast-broker",
        work / BROKER,
        ignore=shutil.ignore_patterns("node_modules", ".wrangler", "__pycache__"),
    )
    shutil.copytree(ROOT / ".github/workflows", work / ".github/workflows")
    for name, file, old, new, test in cases:
        path = work / BROKER / file
        original = path.read_text()
        assert old in original, name
        try:
            path.write_text(original.replace(old, new))
            run = subprocess.run(
                ["node", "--test", BROKER + "test/" + test],
                cwd=work,
                capture_output=True,
                text=True,
                timeout=120,
            )
            kills = re.findall(r"^not ok [0-9]+ - (.+)$", run.stdout, re.M)
            assert run.returncode and len(kills) >= 2, (name, kills, run.stdout[-3000:])
            rows.append({"mutant": name, "kills": len(kills), "tests": kills})
            print(name, len(kills), flush=True)
        finally:
            path.write_text(original)
(ROOT / "traces/wk2_resume_20261018/extra-mutants.json").write_text(
    json.dumps(rows, indent=2) + "\n"
)
