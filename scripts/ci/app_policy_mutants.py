#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""DX1 GPU-free app policy mutants; each must fail two distinct named tests.

Falsifier: an unmutated baseline fails, a mutation anchor is ambiguous, or any
mutant has fewer than two killers. Each run uses a disposable copy; no network.
"""
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
GP = 'scripts/bench/windows/appmatrix/gate_profiles.py'
AP = 'scripts/ci/app_policy.py'
MT = 'scripts/apps/matrix_table.py'
RR = 'scripts/ci/vast_run_report.py'
MUTANTS = [
    ('manual-apps-run', GP, 'set(prof["manual"]["apps"]) | set(prof.get("known_unsupported", {}))\n    return [', 'set(prof.get("known_unsupported", {}))\n    return ['),
    ('unsupported-apps-run', GP, 'set(prof["manual"]["apps"]) | set(prof.get("known_unsupported", {}))\n    return [', 'set(prof["manual"]["apps"])\n    return ['),
    ('explicit-policy-app-runs', GP, 'x not in known or x in non_gating', 'x not in known'),
    ('release-policy-app-runs', GP, 'or set(ids) & non_gating', ''),
    ('manual-evidence-unchecked', GP, 'if name.startswith("overlay_yuv_") and (item.get("reason") != "needs real display" or not item.get("evidence_template")):', 'if False:'),
    ('unsupported-reason-unchecked', GP, 'if name not in known or not isinstance(reason, str) or len(reason) < 20:', 'if False:'),
    ('unsupported-overlap-unchecked', GP, 'if unsupported & (opn | owner | manual):', 'if False:'),
    ('linux-gate-allows-uvm-override', 'scripts/bench/box/apps_gate.sh', 'if grep -Fq "APP_POLICY name=$app " <<< "$policy"; then', 'if false; then'),
    ('generated-gate-policy-omitted', MT, '            handle.write("# known_unsupported (non-gating; listed in reports):\\n")', '            pass'),
    ('grade-policy-omitted', 'scripts/bench/windows/appmatrix/grade.py', '    app_policy.main(["app_policy.py", "windows"])\n', ''),
    ('report-policy-absent', RR, '    app_policy.apply(c)\n', ''),
    ('policy-truncated-after-observed-apps', RR, '            for p in policy\n', '            for p in []\n'),
    ('policy-test-still-fails', AP, "collector.tests[key]['result'] = 'SKIP'", "collector.tests[key]['result'] = 'FAIL'"),
    ('historical-observation-lost', AP, "note += '; observed %s: %s' % (observed[0], observed[2])", 'pass'),
    ('linux-unsupported-class-removed', MT, 'if app in KNOWN_UNSUPPORTED:', 'if False:'),
    ('linux-unsupported-omitted', MT, 'list(POLICY_SKIP) + list(KNOWN_UNSUPPORTED)', 'list(POLICY_SKIP)'),
]
FILES = [GP, AP, MT, RR,
         'scripts/ci/vast_report.py', 'scripts/ci/test_app_policy.py', 'scripts/ci/test_apps_matrix_table.py',
         'scripts/apps/known_unsupported.json', 'scripts/apps/summarize.py',
         'scripts/apps/run_apps.sh', 'scripts/bench/box/apps_gate.sh',
         'scripts/bench/windows/appmatrix/summarize.py', 'scripts/bench/windows/appmatrix/grade.py', 'scripts/bench/windows/appmatrix/gate_profiles.json',
         'scripts/bench/windows/appmatrix/apps.json', 'scripts/bench/windows/appmatrix/manifest.json',
         'scripts/bench/windows/appmatrix/release_software.json', 'scripts/bench/box/specviewperf.pin',
         'traces/specviewperf_vast_20261105/spec/CHECKSUMS_pins.txt', 'docs/DOWNLOADS.md']
COLLECT = '''import io,json,unittest
suite=unittest.TestSuite()
for pattern in ('test_app_policy.py','test_apps_matrix_table.py'):
    suite.addTests(unittest.defaultTestLoader.discover('.',pattern=pattern))
r=unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps({'tests':r.testsRun,'killers':sorted(set(t.id() for t,_ in r.failures+r.errors))}))
'''


def run(root):
    result = subprocess.run([sys.executable, '-c', COLLECT], cwd=root / 'scripts/ci',
                            capture_output=True, text=True, timeout=120,
                            env={'PATH': os.environ['PATH'], 'PYTHONDONTWRITEBYTECODE': '1'})
    if result.returncode:
        raise RuntimeError('harness failure: ' + result.stderr[-500:])
    return json.loads(result.stdout.splitlines()[-1])


def main():
    with tempfile.TemporaryDirectory(prefix='dx1-app-policy-') as tmp:
        root = Path(tmp)
        for name in FILES:
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        baseline = run(root)
        if baseline['killers']:
            print('BASELINE RED: ' + str(baseline))
            return 1
        results = []
        for name, file, old, new in MUTANTS:
            path = root / file
            source = path.read_text()
            if source.count(old) != 1:
                raise RuntimeError(name + ': anchor count ' + str(source.count(old)))
            path.write_text(source.replace(old, new))
            result = run(root)
            path.write_text(source)
            row = dict(mutant=name, count=len(result['killers']), killers=result['killers'])
            results.append(row)
            print(name + ': ' + str(row['count']) + ' killers', file=sys.stderr, flush=True)
        print(json.dumps(dict(tests=baseline['tests'], cases=results), indent=2))
        return int(any(r['count'] < 2 for r in results))


if __name__ == '__main__':
    sys.exit(main())
