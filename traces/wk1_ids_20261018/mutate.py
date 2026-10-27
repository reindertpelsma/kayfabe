"""Run from the repository root, without another test/build in this checkout."""

import json
import os
from pathlib import Path
import re
import subprocess


INDEX = "ci/vast-broker/src/index.js"
RESULT = "ci/vast-broker/src/results.js"
LITERAL = "ci/vast-broker/public-log.js"
RUNNER = "ci/vast-broker/runner.py"
PAYLOAD = "scripts/ci/vast_payload.py"
REPORT = "scripts/ci/vast_report.py"
NODE = [
    "node",
    "--test",
    "ci/vast-broker/test/v4-public-log.test.js",
    "ci/vast-broker/test/logs.test.js",
]
PYTHON = [
    "python3",
    "-m",
    "unittest",
    "discover",
    "-s",
    "scripts/ci",
    "-p",
    "test_vast_host_reference.py",
]
mutants = [
    ("instance alias", INDEX, "instance_id: r.id,", "instance_id: null,", NODE),
    (
        "offer geolocation",
        INDEX,
        "geolocation: r.geolocation ?? 'unknown'",
        "geolocation: 'unknown'",
        NODE,
    ),
    ("offer hourly price", INDEX, "dph_total: r.dphTotal ?? null", "dph_total: null", NODE),
    (
        "host driver label",
        INDEX,
        "host_driver_version: r.hostDriverVersion ?? 'unknown'",
        "host_driver_version: 'unknown'",
        NODE,
    ),
    (
        "host reference status",
        INDEX,
        "host_reference: r.result?.host_reference ?? 'MISSING'",
        "host_reference: 'MISSING'",
        NODE,
    ),
    ("geolocation bound", INDEX, ".slice(0, 128)", "", NODE),
    ("geolocation controls", INDEX, ".replace(/[\\p{Cc}\\p{Cf}]/gu, ' ')", "", NODE),
    (
        "result extension",
        RESULT,
        ".filter((k) => k !== 'host_reference')",
        ".filter((k) => true)",
        NODE,
    ),
    (
        "result enum",
        RESULT,
        "(result.host_reference !== undefined &&\n"
        "      !['GREEN', 'NOT_GREEN', 'MISSING'].includes(result.host_reference))",
        "false",
        NODE,
    ),
    (
        "literal non-Ada models",
        LITERAL,
        "and re.fullmatch(pattern,value,re.I|re.A)",
        "and value == 'RTX 4070'",
        NODE,
    ),
    ("literal rental line", LITERAL, "data.get('cursor',0)==0", "False", NODE),
    (
        "BASE non-Ada models",
        RUNNER,
        "and re.fullmatch(pattern, value, re.I | re.A)",
        'and value == "RTX 4070"',
        NODE,
    ),
    ("BASE rental line", RUNNER, 'self.emit(rental_line(data) + "\\n")', "pass", NODE),
    (
        "native green",
        PAYLOAD,
        'result["host_reference"] = "GREEN"',
        'result["host_reference"] = "MISSING"',
        PYTHON,
    ),
    (
        "native not green",
        PAYLOAD,
        'result["host_reference"] = "NOT_GREEN"',
        'result["host_reference"] = "MISSING"',
        PYTHON,
    ),
    (
        "native missing",
        PAYLOAD,
        'result["host_reference"] = "MISSING"',
        'result["host_reference"] = "GREEN"',
        PYTHON,
    ),
    ("reporter forwarding", REPORT, "        **reference,\n", "", PYTHON),
    ("reporter local schema", REPORT, '            "host_reference",\n', "", PYTHON),
]
original = {p: Path(p).read_text() for _, p, *_ in mutants}
failed = False
try:
    for name, path, before, after, command in mutants:
        source = original[path]
        if source.count(before) != 1:
            raise RuntimeError("ambiguous mutation: " + name)
        Path(path).write_text(source.replace(before, after))
        try:
            # No stale same-size/same-second bytecode may stand in for this edit.
            environment = {
                **os.environ,
                "PYTHONDONTWRITEBYTECODE": "1",
                "PYTHONPYCACHEPREFIX": "/tmp/wk1-ids-disabled-bytecode",
            }
            run = subprocess.run(
                command, capture_output=True, text=True, timeout=60, env=environment
            )
            log = run.stdout + run.stderr
            pattern = r"^not ok \d+ - (.+)$" if command == NODE else r"^(?:ERROR|FAIL): (.+)$"
            kills = re.findall(pattern, log, re.M)
            ok = run.returncode == 1 and len(kills) >= 2
            failed |= not ok
            print(json.dumps(dict(mutant=name, kills=kills, ok=ok)), flush=True)
        finally:
            Path(path).write_text(source)
finally:
    for path, source in original.items():
        Path(path).write_text(source)
raise SystemExit(1 if failed else 0)
