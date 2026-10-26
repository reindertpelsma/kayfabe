#!/usr/bin/env python3
"""Mutation checks for the merge-bar change (2026-10-26): every mutant needs two killing tests.

Targets (the payload adapter has its own mutants in payload_mutants.py):
  - ci/vast-broker/src/sanitize.js   killed by test/log-identifiers.test.js    (KF_SANITIZE_PATH)
  - ci/vast-broker/runner.py scrub   killed by test/runner-identifiers.test.js (KF_RUNNER_PATH)
  - scripts/bench/box/merge_check.sh and kvm_tests.sh
                                     killed by scripts/ci/test_vast_merge_bar.py (KF_MERGE_CHECK, KF_KVM_TESTS)
Stdlib only; GPU-free. A baseline run must be green first.

    python3 scripts/ci/merge_bar_mutants.py [substring-of-mutant-name]
"""

import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
BROKER = ROOT / "ci/vast-broker"
SANITIZE = BROKER / "src/sanitize.js"
RUNNER = BROKER / "runner.py"
MERGE_CHECK = ROOT / "scripts/bench/box/merge_check.sh"
KVM_TESTS = ROOT / "scripts/bench/box/kvm_tests.sh"

# (name, target, old text (exactly once), new text)
MUTANTS = [
    # --- the scrubbers: a long test name survives, a token-shaped string does not -------------------
    (
        "sanitize-keeps-every-long-run",
        SANITIZE,
        "  if (value.length > 128 || !SNAKE_NAME.test(value)) return false;",
        "  return true;",
    ),
    (
        "sanitize-keeps-no-long-run",
        SANITIZE,
        "  return value.replace(/[^0-9]/g, '').length * 5 <= value.length;",
        "  return false;",
    ),
    (
        "sanitize-digit-rule-dropped",
        SANITIZE,
        "  return value.replace(/[^0-9]/g, '').length * 5 <= value.length;",
        "  return true;",
    ),
    ("sanitize-length-cap-dropped", SANITIZE, "value.length > 128 || ", ""),
    (
        "sanitize-two-words-allowed",
        SANITIZE,
        "(?:_[a-z0-9]{1,20}){2,}$",
        "(?:_[a-z0-9]{1,20}){1,}$",
    ),
    (
        "sanitize-uppercase-allowed",
        SANITIZE,
        "/^[a-z][a-z0-9]{0,19}",
        "/^[A-Za-z][A-Za-z0-9]{0,19}",
    ),
    ("sanitize-first-word-unbounded", SANITIZE, "[a-z0-9]{0,19}", "[a-z0-9]{0,99}"),
    ("sanitize-later-words-unbounded", SANITIZE, "{1,20}", "{1,99}"),
    (
        "runner-keeps-every-long-run",
        RUNNER,
        '    return run if keep else "[redacted]"',
        "    return run",
    ),
    (
        "runner-keeps-no-long-run",
        RUNNER,
        '    return run if keep else "[redacted]"',
        '    return "[redacted]"',
    ),
    (
        "runner-digit-rule-dropped",
        RUNNER,
        "\n        and sum(c.isdigit() for c in run) * 5 <= len(run)",
        "",
    ),
    ("runner-length-cap-dropped", RUNNER, "len(run) <= 128\n        and ", ""),
    ("runner-two-words-allowed", RUNNER, "(?:_[a-z0-9]{1,20}){2,}", "(?:_[a-z0-9]{1,20}){1,}"),
    ("runner-uppercase-allowed", RUNNER, 'r"[a-z][a-z0-9]{0,19}', 'r"[A-Za-z][A-Za-z0-9]{0,19}'),
    ("runner-first-word-unbounded", RUNNER, "[a-z0-9]{0,19}", "[a-z0-9]{0,99}"),
    ("runner-later-words-unbounded", RUNNER, "{1,20}", "{1,99}"),
    # --- merge_check.sh: light bar, census stays, KVM stage opt-in --------------------------------
    (
        "bar-runs-crate-tests-again",
        MERGE_CHECK,
        'if [ "${KF_MERGE_KVM_TESTS:-0}" = 1 ]; then',
        'cargo test -q --no-fail-fast "${pk[@]}" >"$PROV/${T}_tests.log" 2>&1\nif [ "${KF_MERGE_KVM_TESTS:-0}" = 1 ]; then',
    ),
    (
        "kvm-stage-always-on",
        MERGE_CHECK,
        'if [ "${KF_MERGE_KVM_TESTS:-0}" = 1 ]; then',
        "if true; then",
    ),
    (
        "kvm-stage-on-for-any-value",
        MERGE_CHECK,
        'if [ "${KF_MERGE_KVM_TESTS:-0}" = 1 ]; then',
        'if [ -n "${KF_MERGE_KVM_TESTS:-}" ]; then',
    ),
    (
        "census-dropped",
        MERGE_CHECK,
        '[ "${#pk[@]}" -ge 36 ] || { echo "v3 crate census truncated"; exit 2; }',
        "true",
    ),
    (
        "census-exit-code-changed",
        MERGE_CHECK,
        'echo "v3 crate census truncated"; exit 2;',
        'echo "v3 crate census truncated"; exit 1;',
    ),
    (
        "legacy-totals-line-back",
        MERGE_CHECK,
        'if [ "${KF_MERGE_KVM_TESTS:-0}" = 1 ]; then',
        'echo "TESTS passed 1 failed 0"\nif [ "${KF_MERGE_KVM_TESTS:-0}" = 1 ]; then',
    ),
    # --- kvm_tests.sh --------------------------------------------------------------------------
    ("kvm-tests-whole-workspace", KVM_TESTS, "-p kf-linux-raw -p kf-chan", "--workspace"),
    ("kvm-tests-fail-fast", KVM_TESTS, "cargo test -q --no-fail-fast", "cargo test -q"),
    ("kvm-skips-tolerated", KVM_TESTS, "kvm_skipped=0$ ]]", "kvm_skipped=[0-9]+$ ]]"),
    ("kvm-unreached-tolerated", KVM_TESTS, "kvm_ran=[1-9][0-9]*", "kvm_ran=[0-9]+"),
    (
        "kvm-zero-tests-tolerated",
        KVM_TESTS,
        "passed\\ [1-9][0-9]*\\ failed",
        "passed\\ [0-9]+\\ failed",
    ),
    (
        "kvm-cargo-rc-ignored",
        KVM_TESTS,
        '[ "$rc" -eq 0 ] && [[',
        "[[",
    ),
    ("kvm-rc-line-dropped", KVM_TESTS, '[ "$rc" -eq 0 ] || echo "KVM_TESTS_RC=$rc"', "true"),
    ("kvm-log-moved", KVM_TESTS, 'log="$PROV/${T}_tests.log"', 'log="$PROV/${T}_kvm.log"'),
    ("kvm-request-line-dropped", KVM_TESTS, 'echo "KVM_TESTS_REQUESTED=1"', "true"),
]


def tap_failures(output):
    return [m[1] for m in re.finditer(r"^not ok \d+ - (.*)$", output, re.M)]


def run_node(test_file, env_name, mutated):
    p = subprocess.run(
        ["node", "--test", "--test-reporter=tap", str(BROKER / "test" / test_file)],
        env=dict(os.environ, **{env_name: str(mutated)}),
        capture_output=True,
        text=True,
        timeout=300,
    )
    return tap_failures(p.stdout)


def run_unittest(env_name, mutated):
    p = subprocess.run(
        [sys.executable, "-m", "unittest", "-v", "test_vast_merge_bar"],
        cwd=HERE,
        env=dict(os.environ, **{env_name: str(mutated)}),
        capture_output=True,
        text=True,
        timeout=300,
    )
    return [
        line.split()[1]
        for line in (p.stdout + p.stderr).splitlines()
        if line.startswith(("FAIL: ", "ERROR: "))
    ]


RUNNERS = {
    SANITIZE: lambda m: run_node("log-identifiers.test.js", "KF_SANITIZE_PATH", m),
    RUNNER: lambda m: run_node("runner-identifiers.test.js", "KF_RUNNER_PATH", m),
    MERGE_CHECK: lambda m: run_unittest("KF_MERGE_CHECK", m),
    KVM_TESTS: lambda m: run_unittest("KF_KVM_TESTS", m),
}


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    rows, bad = [], 0
    with tempfile.TemporaryDirectory(prefix="kf-merge-bar-") as directory:
        for target, runner in RUNNERS.items():
            work = Path(directory) / ("base-" + target.name)
            work.write_text(target.read_text())
            baseline = runner(work)
            if baseline:
                print("BASELINE RED for %s: %s" % (target.name, baseline), file=sys.stderr)
                return 1
        for name, target, old, new in MUTANTS:
            if only not in name:
                continue
            source = target.read_text()
            assert source.count(old) == 1, (name, source.count(old))
            work = Path(directory) / ("mut-" + name + "-" + target.name)
            work.write_text(source.replace(old, new))
            work.chmod(0o755)
            killers = RUNNERS[target](work)
            rows.append({"mutant": name, "killers": killers, "count": len(killers)})
            print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
            bad += len(killers) < 2
    print(json.dumps({"cases": rows}, indent=2))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
