#!/usr/bin/env python3
"""J3 isolated mutation checks. Each mutant needs two distinct failing methods.

Only fake QEMU/SSH and fixture roots run. The baseline must be green first.
Optional arguments are mutant-name substrings (for review/debugging).
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
CI = 'scripts/ci/'
BOX = 'scripts/bench/box/'
# name, file, exact-once old/new source, tests exercised
MUTANTS = [
    ('default-enabled', CI+'vast_payload.py', 'environ.pop("KF_NESTED", "0")', 'environ.pop("KF_NESTED", "1")', ['Selection', 'PayloadStage']),
    ('non-large-enabled', CI+'vast_payload.py', 'or (flag == "1" and tier != "large")', '', ['Selection', 'PayloadStage']),
    ('invalid-flag-enabled', CI+'vast_payload.py', 'flag not in ("0", "1")', 'False', ['Selection', 'PayloadStage']),
    ('flag-leaked', CI+'vast_payload.py', 'environ.pop("KF_NESTED", "0")', 'environ.get("KF_NESTED", "0")', ['Selection', 'ExtraRules', 'PayloadStage']),
    ('environment-unchecked', CI+'vast_report.py', '("KF_NESTED", False, _match(r"[01]"))', '("KF_NESTED", False, _match(r".*"))', ['Selection', 'ExtraRules']),
    ('rows-unchecked', CI+'vast_payload.py', 'and rows_ok and all(m[2] == "PASS" for m in parsed) and ending', 'and True', ['Grade', 'ExtraRules']),
    ('row-results-unchecked', CI+'vast_payload.py', 'all(m[2] == "PASS" for m in parsed)', 'True', ['Grade']),
    ('process-exit-unchecked', CI+'vast_payload.py', 'v == "NESTED_PASS" and rc == 0', 'v == "NESTED_PASS"', ['Grade']),
    ('exit-marker-unchecked', CI+'vast_payload.py', 'and exits == ["NESTED_EXIT rc=0"]', '', ['Grade', 'ExtraRules']),
    ('verdicts-unchecked', CI+'vast_payload.py', 'len(verdicts) == 1 and len(exits) == 1:\n        v = verdicts[0]', 'len(verdicts) >= 1 and len(exits) == 1:\n        v = verdicts[0]', ['Grade', 'ExtraRules']),
    ('verdict-order-unchecked', CI+'vast_payload.py', 'and ending):\n            return [row("nested", 1)]', '):\n            return [row("nested", 1)]', ['ExtraRules']),
    ('failure-with-host-fault', CI+'vast_payload.py', 'and all(m[2] == "PASS" for i, m in enumerate(parsed) if i != 3)', '', ['Grade']),
    ('retry-is-pass', CI+'vast_payload.py', 'raise EvidenceError("nested_retry_" + cls.lower())', 'return [row("nested", 1)]', ['Grade', 'PayloadStage']),
    ('budget-guard-removed', CI+'vast_payload.py', 'min_remaining_ms=NESTED_MIN_REMAINING_MS', 'min_remaining_ms=0', ['PayloadStage']),
    ('report-rows-omitted', CI+'vast_run_report.py', 'elif head == "NESTED_ROW":', 'elif head == "NEVER_NESTED_ROW":', ['NestedReport']),
    ('report-notrun-pass', CI+'vast_run_report.py', '"nested", m[1], "SKIP" if m[2] == "NOT_RUN" else m[2]', '"nested", m[1], "PASS"', ['NestedReport']),
    ('report-suite-unattributed', CI+'vast_run_report.py', 'linux_group += tuple(r["name"] for r in rows if r["name"] == "nested" or r["name"].startswith("nested_retry_"))', 'linux_group += ()', ['NestedReport']),
    ('kvm-always', BOX+'nested_gate.py', "if probe.get('kvm_start') == 'yes':", 'if True:', ['Inner']),
    ('tcg-assumed', BOX+'nested_gate.py', "if probe.get('tcg') == 'yes':", 'if True:', ['Inner']),
    ('qemu-exit-unchecked', BOX+'nested_gate.py', 'if rc != 0 or len(ready) != 1', 'if len(ready) != 1', ['Inner', 'StageIntegration.test_qemu_failure_retries_even_with_passing_guest']),
    ('serial-evidence-unchecked', BOX+'nested_gate.py', "if rc != 0 or len(ready) != 1 or len(clients) != 1 or len(done) != 1 or len(checks) != 1:", 'if False:', ['Inner']),
    ('client-verdict-unchecked', BOX+'nested_gate.py', "if clients != ['0'] or checks != [CHECK_PASS]:", 'if False:', ['Inner', 'StageIntegration.test_raw_check_failure']),
    ('serial-order-unchecked', BOX+'nested_gate.py', 'if positions != sorted(positions):', 'if False:', ['ExtraRules']),
    ('workload-expanded', BOX+'nested_gate.py', 'KF_ARMS=--ce-client KF_IOCTL_TRACE=ring', 'KF_ARMS=--timer,--engines,--ce-client KF_IOCTL_TRACE=ring', ['StageIntegration.test_full_tcg_path', 'StageIntegration.test_small_workload_command']),
    ('tcg-host-cpu', BOX+'nested_gate.py', "'host' if accel == 'kvm' else 'max'", "'host'", ['StageIntegration.test_full_tcg_path', 'StageIntegration.test_small_workload_command']),
    ('inner-budget-guard-removed', BOX+'nested_gate.py', 'if deadline-time.monotonic() < budget:', 'if False:', ['StageIntegration.test_insufficient_budget_never_starts_vm', 'StageIntegration.test_tcg_budget_not_shortened_to_fit']),
    ('timeout-can-pass', BOX+'nested_gate.py', "raise Retry(reason) from None", 'return 0', ['StageIntegration.test_timeout_retries_and_reaps', 'StageIntegration.test_timeout_cannot_accept_preexisting_passing_serial']),
    ('logs-unbounded', BOX+'nested_gate.py', 'MAX_LOG = 16 * 1024 * 1024', 'MAX_LOG = 64 * 1024 * 1024', ['StageIntegration.test_serial_log_limit_retries', 'StageIntegration.test_qemu_log_limit_retries']),
    ('tcg-unchecked', BOX+'nested_probe.sh', 'tcg=no', 'tcg=yes', ['Probe']),
    ('dry-check-unchecked', BOX+'nested_probe.sh', '>/dev/null 2>&1; then start=yes; fi', '>/dev/null 2>&1; then start=yes; else start=yes; fi', ['Probe']),
]
FILES = [CI+f for f in ('test_nested.py', 'test_vast_payload.py', 'test_vast_payload_v7.py',
                        'vast_payload.py', 'vast_report.py', 'vast_run_report.py')]
FILES += [BOX+f for f in ('nested_gate.py', 'nested_gate.sh', 'nested_probe.sh')]
FILES += ['scripts/bench/native_xid_guard.py']
COLLECT = '''import contextlib,io,json,unittest,sys
import test_nested
names=json.loads(sys.argv[1])
s=unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromName('test_nested.'+n) for n in names)
with contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
 r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors], 'details':{t.id():trace for t,trace in r.failures+r.errors}}))
'''


def run(root, names):
    env = {'PATH': os.environ['PATH'], 'HOME': os.environ.get('HOME', '/tmp'), 'PYTHONDONTWRITEBYTECODE': '1'}
    done = subprocess.run([sys.executable, '-c', COLLECT, json.dumps(names)], cwd=root/CI,
                          env=env, capture_output=True, text=True, timeout=300)
    if done.returncode:
        raise RuntimeError('mutation harness error: '+done.stderr[-400:])
    return json.loads(done.stdout.splitlines()[-1])


def main():
    chosen = [m for m in MUTANTS if not sys.argv[1:] or any(a in m[0] for a in sys.argv[1:])]
    if not chosen:
        raise SystemExit('no matching mutants')
    rows = []
    with tempfile.TemporaryDirectory(prefix='kf-nested-mutants-') as directory:
        root = Path(directory)
        for relative in FILES:
            target = root/relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT/relative, target)
        names = ['Probe', 'Selection', 'Grade', 'Inner', 'StageIntegration', 'PayloadStage', 'NestedReport', 'ExtraRules']
        baseline = run(root, names)
        if baseline['killers']:
            print(json.dumps({'baseline': baseline}, indent=2))
            return 1
        for name, relative, old, new, tests in chosen:
            target = root/relative
            original = target.read_text()
            if original.count(old) != 1:
                raise RuntimeError(name+': anchor count '+str(original.count(old)))
            try:
                target.write_text(original.replace(old, new))
                result = run(root, tests)
            finally:
                target.write_text(original)
            killers = sorted(set(result['killers']))
            rows.append({'mutant': name, 'count': len(killers), 'killers': killers})
            print(name+': '+str(len(killers))+' killers', file=sys.stderr, flush=True)
    print(json.dumps({'tests': baseline['tests'], 'cases': rows}, indent=2))
    return int(any(r['count'] < 2 for r in rows))


if __name__ == '__main__':
    sys.exit(main())
