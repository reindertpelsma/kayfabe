#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Run each one-line coverage meter mutant against independently named tests."""
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CASES = (
    ("nonzero-treated-as-ok", "evidence.py", '("refused" if int(status, 16) else "serviced")',
     '"serviced"', {"test_trace_and_legacy_reply_preserve_nonzero_refusals",
                   "test_alloc_refusal_is_not_a_success"}),
    ("control-id-dropped", "evidence.py", 'if fn == "76":', 'if fn == "77":',
     {"test_real_census_keeps_rpc_control_class_and_host_escape_separate",
      "test_code_count_and_no_reply_do_not_invent_control_ids"}),
    ("control-owner-shifted", "interface.py", 'add("control", owner, value, name)',
     'add("control", owner + 1, value, name)',
     {"test_subdevice_control_with_alias", "test_root_control_has_root_class"}),
)


def main():
    clean = subprocess.run([sys.executable, str(HERE / "test_coverage_meter.py")], check=False)
    if clean.returncode:
        return clean.returncode
    failed = False
    for name, filename, old, new, expected in CASES:
        with tempfile.TemporaryDirectory() as job:
            package = Path(job) / "coverage_meter"
            shutil.copytree(HERE / "coverage_meter", package, ignore=shutil.ignore_patterns("__pycache__"))
            path = package / filename
            source = path.read_text()
            assert source.count(old) == 1, name
            path.write_text(source.replace(old, new))
            result = subprocess.run([sys.executable, str(HERE / "test_coverage_meter.py"), "-v"],
                                    env=dict(os.environ, KF_COVERAGE_METER_MODULES=job),
                                    capture_output=True, text=True, check=False)
        killers = {line.split()[0] for line in result.stderr.splitlines()
                   if line.startswith("test_") and line.endswith(("... FAIL", "... ERROR"))}
        print(f"{name} {'killed' if expected <= killers else 'survived'} ({', '.join(sorted(killers))})")
        failed |= not expected <= killers
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
