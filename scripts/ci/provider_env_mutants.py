#!/usr/bin/env python3
"""Isolated mutation checks for VM environment recovery; two named killers each."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
SOURCE = (HERE / "vast_report.py").read_text()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


def provider_changed(old, new):
    start = SOURCE.index("def load_provider_env(")
    end = SOURCE.index("def run_check_env(", start)
    value = SOURCE[start:end]
    assert value.count(old) == 1
    return SOURCE[:start] + value.replace(old, new) + SOURCE[end:]


CHECK_CALL = next(
    line for line in SOURCE.splitlines() if line.strip().startswith("env = load_provider_env(")
)
ERROR_RETURN = next(
    line
    for line in SOURCE.splitlines()
    if 'sys.stderr.write("onstart: provider environment is invalid' in line
)
ERROR_RETURN += "\n        return 2"


CASES = [
    (
        "provider-file-ignored",
        changed(
            '        with open(path, "rb") as source:\n            raw = source.read(65537)',
            "        return result",
        ),
    ),
    (
        "unknown-secret-imported",
        changed("if not separator or name not in missing:", "if not separator:"),
    ),
    ("process-value-overridden", changed("if not result.get(name)}", "if True}")),
    ("duplicate-provider-field-admitted", changed("        if name in seen:", "        if False:")),
    (
        "quotes-left-unparsed",
        changed(
            "        words = shlex.split(value, comments=False, posix=True)",
            "        words = [value.strip()]",
        ),
    ),
    ("provider-file-size-unbounded", provider_changed("    if len(raw) > 65536:", "    if False:")),
    ("provider-file-not-used-at-check", changed(CHECK_CALL, "        env = env")),
    ("invalid-provider-file-success", changed(ERROR_RETURN, "        return 0")),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_provider_env.py')
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
    with tempfile.TemporaryDirectory(prefix="kf-v7-env-mutants-") as d:
        root = Path(d)
        ci = root / "scripts/ci"
        ci.mkdir(parents=True)
        for name in ["vast_report.py", "test_vast_provider_env.py", "test_vast_box.py"]:
            shutil.copyfile(HERE / name, ci / name)
        assert not run(root)["killers"]
        for name, source in CASES:
            (ci / "vast_report.py").write_text(source)
            result = run(root)
            killers = sorted(set(result["killers"]))
            assert len(killers) >= 2, (name, killers)
            rows.append({"mutant": name, "count": len(killers), "killers": killers})
    print(json.dumps({"run": "v7-provider-env-mutants-20261023", "cases": rows}, indent=2))


if __name__ == "__main__":
    main()
