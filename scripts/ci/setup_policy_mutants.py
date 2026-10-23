#!/usr/bin/env python3
"""Policy mutations require at least two independently named failing contracts."""

import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
SOURCE = (HERE.parent / "bench/provision_box.sh").read_text()


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    (
        "ci-installs-optional-stack",
        changed(
            "ci) printf '%s\\n' \"${required[@]}\"",
            'ci) printf \'%s\\n\' "${required[@]}" "${optional[@]}"',
        ),
    ),
    (
        "ci-required-tools-missing",
        changed("ci) printf '%s\\n' \"${required[@]}\"", "ci) printf '%s\\n' empty"),
    ),
    (
        "full-profile-loses-tools",
        changed(
            'full) printf \'%s\\n\' "${required[@]}" "${optional[@]}"',
            "full) printf '%s\\n' \"${required[@]}\"",
        ),
    ),
    (
        "bench-uses-full-stack",
        changed(
            "bench) printf '%s\\n' \"${bench[@]}\"",
            'bench) printf \'%s\\n\' "${required[@]}" "${optional[@]}"',
        ),
    ),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_setup_packages.py')
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
    with tempfile.TemporaryDirectory(prefix="kf-v8-package-mutants-") as d:
        root = Path(d)
        ci = root / "scripts/ci"
        ci.mkdir(parents=True)
        bench = root / "scripts/bench"
        bench.mkdir()
        shutil.copyfile(HERE / "test_vast_setup_packages.py", ci / "test_vast_setup_packages.py")
        target = bench / "provision_box.sh"
        target.write_text(SOURCE)
        assert not run(root)["killers"]
        for name, source in CASES:
            target.write_text(source)
            result = run(root)
            killers = sorted(set(result["killers"]))
            assert len(killers) >= 2, (name, killers)
            rows.append({"mutant": name, "count": len(killers), "killers": killers})
    print(json.dumps({"run": "v8-package-policy-mutants-20261023", "cases": rows}, indent=2))


if __name__ == "__main__":
    main()
