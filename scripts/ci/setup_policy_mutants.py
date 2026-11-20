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
VAST_SETUP = (HERE / "vast_setup.py").read_text()
# the files test_package_profiles.py reads besides provision_box.sh (2026-11-20: the cache policy list, the lock's digest, the private Python that names the profile)
EXTRA_FILES = [("scripts/ci/test_package_profiles.py",), ("scripts/ci/vast_setup.py",), ("scripts/ci/setup-cache.lock.json",), ("scripts/apps/mlrows/private_python.py",)]


def changed(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


CASES = [
    (
        "missing-ssh-tools-skip-the-apt-probe",
        changed("fuser ssh scp ssh-keygen)", "fuser)"),
    ),
    (
        "fresh-image-loses-guest-ssh-tools",
        changed("busybox-static openssh-client)", "busybox-static)"),
    ),
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
        "pybuild-loses-libssl",
        changed("local pybuild=(build-essential zlib1g-dev libssl-dev libffi-dev", "local pybuild=(build-essential zlib1g-dev libffi-dev"),
    ),
    (
        "pybuild-is-not-a-profile",
        changed("    pybuild) printf '%s\\n' \"${pybuild[@]}\" ;;\n", ""),
    ),
    (
        "pybuild-is-the-ci-list",
        changed("pybuild) printf '%s\\n' \"${pybuild[@]}\"", "pybuild) printf '%s\\n' \"${required[@]}\""),
    ),
    (
        "bench-uses-full-stack",
        changed(
            "bench) printf '%s\\n' \"${bench[@]}\"",
            'bench) printf \'%s\\n\' "${required[@]}" "${optional[@]}"',
        ),
    ),
]

def changed_vast_setup(old, new):
    assert VAST_SETUP.count(old) == 1, old
    return VAST_SETUP.replace(old, new)


# mutations of the cache policy list in vast_setup.py itself (a profile a stage installs must be in the cache's policy)
VAST_SETUP_CASES = [
    (
        "pybuild-left-out-of-the-cache-policy",
        changed_vast_setup('["ci", "bench", "windows", "apps", "pybuild"]', '["ci", "bench", "windows", "apps"]'),
    ),
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_*.py')
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
        for (rel,) in EXTRA_FILES:
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(HERE.parents[1] / rel, root / rel)
        target = bench / "provision_box.sh"
        target.write_text(SOURCE)
        assert not run(root)["killers"]
        for name, source in CASES:
            target.write_text(source)
            result = run(root)
            killers = sorted(set(result["killers"]))
            assert len(killers) >= 2, (name, killers)
            rows.append({"mutant": name, "count": len(killers), "killers": killers})
        target.write_text(SOURCE)
        for name, source in VAST_SETUP_CASES:
            (ci / "vast_setup.py").write_text(source)
            killers = sorted(set(run(root)["killers"]))
            assert len(killers) >= 2, (name, killers)
            rows.append({"mutant": name, "count": len(killers), "killers": killers})
        (ci / "vast_setup.py").write_text(VAST_SETUP)
    print(json.dumps({"run": "v8-package-policy-mutants-20261023", "cases": rows}, indent=2))


if __name__ == "__main__":
    main()
