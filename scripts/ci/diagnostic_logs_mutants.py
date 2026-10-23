#!/usr/bin/env python3
"""Isolated diagnostic policy mutants; each requires two named failing tests."""

import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
SOURCE = (HERE / "vast_diagnostic_logs.py").read_text()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    (
        "ownership-unchecked",
        changed(
            "if type(owned_id) is not int or owned_id <= 0 or owned_id != requested_id:",
            "if False:",
        ),
    ),
    ("s3-host-unchecked", changed("or not s3_host", "or False")),
    ("authorization-header-leak", changed("headers={}", 'headers={"Authorization": "secret"}')),
    ("cookie-header-leak", changed("headers={}", 'headers={"Cookie": "secret"}')),
    ("wrong-timeout", changed("TIMEOUT_SECONDS = 30", "TIMEOUT_SECONDS = 60")),
    ("larger-size-cap", changed("MAX_BYTES = 5_000_000", "MAX_BYTES = 6_000_000")),
    ("redirect-followed", changed("        return None", "        return newurl")),
    ("redirect-handler-omitted", changed("build_opener(NoRedirect())", "build_opener()")),
    (
        "exception-detail-leaked",
        changed(
            'raise LogRefused("diagnostic download refused") from None',
            'raise LogRefused("REMOTE_SECRET signature") from None',
        ),
    ),
    (
        "normal-api-s3-expanded",
        changed(
            'parsed.hostname == "vast.ai"',
            '(parsed.hostname.endswith(".amazonaws.com") or parsed.hostname == "vast.ai")',
        ),
    ),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_diagnostic_logs.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run(root):
    p = subprocess.run(
        [sys.executable, "-c", COLLECT], cwd=root, capture_output=True, text=True, timeout=20
    )
    assert p.returncode == 0, p.stderr[-300:]
    return json.loads(p.stdout.splitlines()[-1])


def main():
    rows = []
    with tempfile.TemporaryDirectory(prefix="kf-v7-diagnostic-mutants-") as d:
        root = Path(d)
        ci = root / "scripts/ci"
        ci.mkdir(parents=True)
        shutil.copyfile(HERE / "test_vast_diagnostic_logs.py", ci / "test_vast_diagnostic_logs.py")
        (ci / "vast_diagnostic_logs.py").write_text(SOURCE)
        baseline = run(root)
        assert not baseline["killers"]
        for name, source in CASES:
            (ci / "vast_diagnostic_logs.py").write_text(source)
            result = run(root)
            killers = sorted(set(result["killers"]))
            assert len(killers) >= 2, (name, killers)
            rows.append({"mutant": name, "count": len(killers), "killers": killers})
    print(
        json.dumps(
            {"run": "v7-diagnostic-mutants-20261023", "tests": baseline["tests"], "cases": rows},
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
