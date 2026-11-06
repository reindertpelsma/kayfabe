"""Check the actual-die correction and retained binding/identity refusals."""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
MUTANTS = {
    "forecast_reintroduced": (
        "    return {",
        '    if die != "GA106":\n        raise BindingRefused("forecast")\n' "    return {",
    ),
    "scope_guard_removed": (
        'raise BindingRefused("unknown or out-of-scope native tuple")',
        "pass",
    ),
    "binding_guard_removed": (
        'raise BindingRefused("inconsistent native binding")',
        "pass",
    ),
    "ssh_console_guard_removed": (
        'raise BindingRefused("real SSH/console identity failure")',
        "pass",
    ),
}
DRIVER = """
import json
from pathlib import Path
import unittest
suite = unittest.defaultTestLoader.discover('.', pattern='test_die_binding.py')
result = unittest.TextTestRunner(verbosity=0).run(suite)
Path('result.json').write_text(json.dumps({
    'run': result.testsRun,
    'killers': sorted({test.id() for test, _ in result.failures + result.errors})
}))
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    source = (HERE / "die_binding.py").read_text()
    tests = (
        (HERE / "test_die_binding.py")
        .read_text()
        .replace(
            "ROOT = Path(__file__).resolve().parents[3]", "ROOT = Path(" + repr(str(ROOT)) + ")"
        )
    )
    rows = []
    for name, (before, after) in MUTANTS.items():
        if source.count(before) != 1:
            raise ValueError("mutation site is not unique: " + name)
        with tempfile.TemporaryDirectory(prefix="sw3-binding-mutant-") as directory:
            root = Path(directory)
            (root / "die_binding.py").write_text(source.replace(before, after))
            (root / "test_die_binding.py").write_text(tests)
            subprocess.run(
                [sys.executable, "-c", DRIVER],
                cwd=root,
                capture_output=True,
                text=True,
                check=True,
                timeout=30,
            )
            result = json.loads((root / "result.json").read_text())
        if result["run"] != 17 or len(result["killers"]) < 2:
            raise ValueError("insufficient killing tests: " + name)
        rows.append({"mutant": name, **result})
    args.output.write_text(json.dumps(rows, indent=2) + "\n")
    print(json.dumps({row["mutant"]: len(row["killers"]) for row in rows}))


if __name__ == "__main__":
    main()
