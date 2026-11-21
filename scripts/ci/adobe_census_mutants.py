#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Apply exact-once mutants in temporary copies and require two named killers."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
MUTANTS = [
    ("reference-only-omitted", "if ref_enabled != kf_enabled:",
     "if kf_enabled and not ref_enabled:", ("test_reference_only", "test_both_directions")),
    ("reasons-dropped", '"reference": {"version": ref_version, **a}',
     '"reference": {"version": ref_version, **a, "reason": ""}', ("test_reasons_and_sources_preserved", "test_table_contains_both_reasons")),
    ("unknown-is-enabled",
     'ref_enabled = a["state"] == "enabled"\n            kf_enabled = b["state"] == "enabled"',
     'ref_enabled = a["state"] in ("enabled", "unknown")\n'
     '            kf_enabled = b["state"] in ("enabled", "unknown")',
     ("test_unknown_is_not_enabled", "test_unknown_and_disabled_do_not_differ")),
]


def run(folder, *tests):
    return subprocess.run([sys.executable, "-B", "-m", "unittest", *tests],
                          cwd=folder, capture_output=True, text=True)


def main():
    for name, old, new, tests in MUTANTS:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            shutil.copytree(HERE / "adobe_census", path / "adobe_census", ignore=shutil.ignore_patterns("__pycache__"))
            shutil.copy(HERE / "test_adobe_census.py", path)
            target = path / "adobe_census" / "__init__.py"
            original = target.read_text()
            killers = [f"test_adobe_census.Diff.{test}" for test in tests]
            baseline = run(path, *killers)
            if baseline.returncode:
                raise SystemExit(baseline.stderr)
            if original.count(old) != 1 or old == new:
                raise SystemExit(f"{name}: invalid mutation anchor")
            target.write_text(original.replace(old, new))
            for test in killers:
                result = run(path, test)
                if result.returncode != 1 or "FAIL:" not in result.stderr:
                    raise SystemExit(f"{name}: {test} did not kill the mutant\n{result.stderr}")
            print(f"{name}: {', '.join(tests)}")


if __name__ == "__main__":
    main()
