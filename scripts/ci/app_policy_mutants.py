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
GRADE = 'scripts/bench/windows/appmatrix/grade.py'
GEN = 'scripts/bench/windows/appmatrix/gen_apps.py'
CATALOG = 'scripts/bench/windows/appmatrix/apps.json'
LEDGER = 'docs/RELEASE_CLAIMS.tsv'
MATRIX = 'docs/design/V3_WINDOWS_APP_MATRIX.md'
MUTANTS = [
    ('manual-apps-run', GP, 'set(prof["manual"]["apps"]) | set(prof.get("known_unsupported", {}))\n    return [', 'set(prof.get("known_unsupported", {}))\n    return ['),
    ('unsupported-apps-run', GP, 'set(prof["manual"]["apps"]) | set(prof.get("known_unsupported", {}))\n    return [', 'set(prof["manual"]["apps"])\n    return ['),
    ('explicit-policy-app-runs', GP, 'x not in known or x in non_gating', 'x not in known'),
    ('release-policy-app-runs', GP, 'or set(ids) & non_gating', ''),
    ('manual-evidence-unchecked', GP, 'if name.startswith("overlay_yuv_") and (item.get("reason") != "needs real display" or not item.get("evidence_template")):', 'if False:'),
    ('unsupported-reason-unchecked', GP, 'if name not in known or not isinstance(reason, str) or len(reason) < 20:', 'if False:'),
    ('unsupported-overlap-unchecked', GP, 'if unsupported & (opn | owner | manual):', 'if False:'),
    ('linux-gate-allows-uvm-override', 'scripts/kfci/apps_gate.py', '                or app in non_gating\n', ''),
    ('linux-gate-policy-output-omitted', 'scripts/kfci/apps_gate.py',
     '        for line in policy_lines:\n            ctx.emit(line)\n', ''),
    ('linux-gate-failed-policy-admitted', 'scripts/kfci/apps_gate.py',
     'if policy.returncode or policy.timed_out or policy.truncated:', 'if False:'),
    ('linux-gate-empty-policy-admitted', 'scripts/kfci/apps_gate.py',
     '        if not policy_lines:\n            return StepResult.retry(RetryClass.BAD_LIST)\n', ''),
    ('linux-gate-policy-namespace-widened', 'scripts/kfci/apps_gate.py',
     'allow_env=BASE_ENV,', 'allow_env=legacy_allowlist(env),'),
    ('linux-gate-unknown-app-admitted', 'scripts/kfci/apps_gate.py',
     '                or " " + app + " " not in " " + known.output.rstrip("\\n") + " "\n', ''),
    ('generated-gate-policy-omitted', MT, '            handle.write("# known_unsupported (non-gating; listed in reports):\\n")', '            pass'),
    ('grade-policy-omitted', 'scripts/bench/windows/appmatrix/grade.py', '    app_policy.main(["app_policy.py", "windows"])\n', ''),
    ('report-policy-absent', RR, '    app_policy.apply(c)\n', ''),
    ('policy-truncated-after-observed-apps', RR, '            for p in policy\n', '            for p in []\n'),
    ('policy-test-still-fails', AP, "collector.tests[key]['result'] = 'SKIP'", "collector.tests[key]['result'] = 'FAIL'"),
    ('historical-observation-lost', AP, "note += '; observed %s: %s' % (observed[0], observed[2])", 'pass'),
    ('linux-unsupported-class-removed', MT, 'if app in KNOWN_UNSUPPORTED:', 'if False:'),
    ('linux-unsupported-omitted', MT, 'list(POLICY_SKIP) + list(KNOWN_UNSUPPORTED)', 'list(POLICY_SKIP)'),
    ('grade-manual-gets-pass-credit', GRADE,
     'non_gating = set(profiles["manual"]["apps"]) | set(profiles["known_unsupported"])',
     'non_gating = set(profiles["known_unsupported"])'),
    ('grade-unsupported-gets-pass-credit', GRADE,
     'non_gating = set(profiles["manual"]["apps"]) | set(profiles["known_unsupported"])',
     'non_gating = set(profiles["manual"]["apps"])'),
    ('grade-policy-guard-removed', GRADE, 'if app in non_gating:', 'if False:'),
    ('grade-policy-increments-pass', GRADE,
     'per[app] = dict(cls="harness", verdict="NOTRUN",', 'per[app] = dict(cls="pass", verdict="NOTRUN",'),
    ('grade-policy-publishes-pass', GRADE,
     'per[app] = dict(cls="harness", verdict="NOTRUN",', 'per[app] = dict(cls="harness", verdict="PASS",'),
    ('grade-policy-overwritten-by-observation', GRADE,
     'note="non-gating app in gate expectation")\n            continue',
     'note="non-gating app in gate expectation")'),
    ('grade-policy-raw-observations-lost', GRADE,
     '                            batched=(batched.get(app) or {}).get("verdict", "-"),\n'
     '                            alone=(alone.get(app) or {}).get("verdict", "-"),',
     '                            batched="-", alone="-",'),
    ('native-generator-setup-becomes-fail', GEN,
     'Virtual-display native probe: SETUP/refused (code 5), no DIRECT overlay, no scenario frames.',
     'Virtual-display native probe is FAIL (no DIRECT overlay).'),
    ('native-ledger-setup-becomes-fail', LEDGER,
     'Native Vast simulated-display probe SETUP/refused (code 5): no DIRECT overlay; no scenario frames',
     'Native Vast simulated-display probe FAIL: no DIRECT overlay'),
]

# Mutate one named catalog/template/doc entry, so every anchor is unambiguous.
catalog = json.loads((ROOT / CATALOG).read_text())
probe_note = next(a['note'] for a in catalog['apps'] if a['id'] == 'overlay_yuv_probe')
MUTANTS.append(('native-catalog-setup-becomes-fail', CATALOG,
                json.dumps(probe_note), json.dumps(probe_note.replace('SETUP/refused (code 5)', 'FAIL'))))
profile_text = (ROOT / 'scripts/bench/windows/appmatrix/gate_profiles.json').read_text()
template_start = profile_text.index('"id": "overlay_yuv_probe"')
template_end = profile_text.index('\n', profile_text.index('"evidence_template":', template_start))
template = profile_text[template_start:template_end]
MUTANTS.append(('native-template-setup-becomes-fail', 'scripts/bench/windows/appmatrix/gate_profiles.json',
                template, template.replace('SETUP/refused (code 5)', 'FAIL')))
readme_overlay = next(line for line in (ROOT / 'README.md').read_text().splitlines()
                      if line.startswith('**Manual: needs real display.**'))
MUTANTS.append(('native-readme-setup-becomes-fail', 'README.md', readme_overlay,
                readme_overlay.replace('SETUP/refused (code 5)', 'FAIL')))
matrix_probe = next(line for line in (ROOT / MATRIX).read_text().splitlines()
                    if line.startswith('| `overlay_yuv_probe` |'))
MUTANTS.append(('native-generated-doc-setup-becomes-fail', MATRIX, matrix_probe,
                matrix_probe.replace('SETUP/refused (code 5)', 'FAIL')))
FILES = [GP, AP, MT, RR,
         'scripts/ci/vast_report.py', 'scripts/ci/test_app_policy.py', 'scripts/ci/test_apps_matrix_table.py',
         'scripts/apps/known_unsupported.json', 'scripts/apps/summarize.py',
         'scripts/apps/run_apps.sh', 'scripts/bench/box/apps_gate.sh',
         'scripts/bench/windows/appmatrix/summarize.py', 'scripts/bench/windows/appmatrix/grade.py', 'scripts/bench/windows/appmatrix/gate_profiles.json',
         'scripts/bench/windows/appmatrix/apps.json', 'scripts/bench/windows/appmatrix/manifest.json',
         'scripts/bench/windows/appmatrix/release_software.json', 'scripts/bench/box/specviewperf.pin',
         GEN, LEDGER, MATRIX, 'README.md',
         'scripts/ci/test_kfci_app_policy.py',
         'traces/windows_overlay_native_20261010/README.md', 'traces/windows_overlay_native_20261010/raw/t3.txt',
         'traces/specviewperf_vast_20261105/spec/CHECKSUMS_pins.txt', 'docs/DOWNLOADS.md']
FILES += [str(path.relative_to(ROOT)) for path in (ROOT / 'scripts/kfci').rglob('*')
          if path.is_file() and path.suffix in ('.py', '.json')]
COLLECT = '''import io,json,unittest
suite=unittest.TestSuite()
for pattern in ('test_app_policy.py','test_apps_matrix_table.py','test_kfci_app_policy.py'):
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
            methods = sorted({killer.split(' (', 1)[0] for killer in result['killers']})
            row = dict(mutant=name, count=len(result['killers']), killers=result['killers'],
                       method_count=len(methods), killer_methods=methods)
            results.append(row)
            print(name + ': ' + str(row['count']) + ' killers', file=sys.stderr, flush=True)
        print(json.dumps(dict(tests=baseline['tests'], cases=results), indent=2))
        return int(any(r['method_count'] < 2 for r in results))


if __name__ == '__main__':
    sys.exit(main())
