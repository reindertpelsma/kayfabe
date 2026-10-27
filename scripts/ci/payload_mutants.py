#!/usr/bin/env python3
"""Isolated, GPU-free mutation checks for the thin payload adapter."""

import ast
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
SOURCE = (HERE / "vast_payload.py").read_text()
HOOK = (HERE / "vast_payload_hooks.sh").read_text()
GUARD = (HERE.parent / "bench/native_xid_guard.py").read_text()
REPORT = (HERE / "vast_report.py").read_text()


def report_replace(old, new):
    assert REPORT.count(old) == 1, old
    return REPORT.replace(old, new)


def guard_replace(old, new):
    assert GUARD.count(old) == 1, old
    return GUARD.replace(old, new)


def guard_scoped(name, old, new):
    node = next(
        n for n in ast.parse(GUARD).body if isinstance(n, ast.FunctionDef) and n.name == name
    )
    lines = GUARD.splitlines(keepends=True)
    value = "".join(lines[node.lineno - 1 : node.end_lineno])
    assert value.count(old) == 1, (name, old)
    return (
        "".join(lines[: node.lineno - 1])
        + value.replace(old, new)
        + "".join(lines[node.end_lineno :])
    )


def function(name, body):
    nodes = [n for n in ast.parse(SOURCE).body if isinstance(n, ast.FunctionDef) and n.name == name]
    assert len(nodes) == 1, name
    node = nodes[0]
    lines = SOURCE.splitlines(keepends=True)
    header = "".join(lines[node.lineno - 1 : node.body[0].lineno - 1])
    return (
        "".join(lines[: node.lineno - 1]) + header + body + "\n" + "".join(lines[node.end_lineno :])
    )


def replace(old, new):
    assert SOURCE.count(old) == 1, old
    return SOURCE.replace(old, new)


def scoped(name, old, new):
    node = next(
        n for n in ast.parse(SOURCE).body if isinstance(n, ast.FunctionDef) and n.name == name
    )
    lines = SOURCE.splitlines(keepends=True)
    value = "".join(lines[node.lineno - 1 : node.end_lineno])
    assert old in value
    value = value.replace(old, new)
    return "".join(lines[: node.lineno - 1]) + value + "".join(lines[node.end_lineno :])


CASES = [
    (
        "minimal-profile-not-forced",
        replace(
            '            KF_PROVISION_PROFILE="ci",', '            KF_PROVISION_PROFILE="full",'
        ),
    ),
    (
        "inherited-expensive-profile-admitted",
        replace(
            '            KF_PROVISION_PROFILE="ci",',
            '            KF_PROVISION_PROFILE=os.environ.get("KF_PROVISION_PROFILE", "full"),',
        ),
    ),
    (
        "failed-provision-hardware-stale",
        replace('            result["hardware"] = hardware()', "            pass"),
    ),
    (
        "hardware-refreshed-only-after-success",
        replace(
            '            result["hardware"] = hardware()\n'
            "            rows = provision_rows(log, rc)",
            "            rows = provision_rows(log, rc)\n"
            '            result["hardware"] = hardware()',
        ),
    ),
    (
        "existing-client-target-redirected",
        replace(
            '            CARGO_TARGET_DIR=str(root / "target"),',
            '            CARGO_TARGET_DIR="/workspace/bench/kf-verify-target",',
        ),
    ),
    (
        "existing-client-target-nested",
        replace(
            '            CARGO_TARGET_DIR=str(root / "target"),',
            '            CARGO_TARGET_DIR=str(root / "target" / "misplaced"),',
        ),
    ),
    ("provision-exit-only", function("provision_rows", '    return [row("provision", 1)]')),
    ("merge-evidence-bypass", function("merge_rows", '    return [row("merge_bar", 1)]')),
    ("ladder-exit-only", function("ladder_rows", '    return [row("cuda_host", 4)]')),
    ("red-record-ignored", function("red_line", "    return False")),
    ("deadline-status-forged", scoped("run_bounded", "return 124", "return 0")),
    (
        "finished-prefix-lost",
        scoped(
            "execute_stages", '            result["suites"].extend(error.rows)', "            pass"
        ),
    ),
    (
        "first-red-continues",
        scoped("execute_stages", "            return 1", "            continue"),
    ),
    ("result-never-published", replace("    temporary.replace(destination)", "    pass")),
    ("approved-source-unpinned", function("prepare_checkout", '    return "kf-ci-approved"')),
    ("xid-unconditionally-admitted", function("xid_ok", "    return True")),
    (
        "hardware-unvalidated",
        function(
            "hardware",
            '    return {"gpu_name":"RTX 3060","compute_cap":860,"driver_version":"580.159.04"}',
        ),
    ),
    ("failed-stage-attribution-lost", replace("    if rc not in (0, 6):", "    if rc == 124:")),
    (
        "unattributed-stage-xid-admitted",
        replace(
            "        if collections.Counter(delta) != collections.Counter(allowed):",
            "        if False:",
        ),
    ),
    (
        "duplicate-cuda-rung-admitted",
        replace("        if not match or match[1] in seen:", "        if not match:"),
    ),
    (
        "merge-exit-marker-ignored",
        replace(
            '    if rc or red_line(text) or len(re.findall(r"^EXIT rc=0 .+$", text, re.M)) != 1:',
            "    if rc or red_line(text):",
        ),
    ),
    (
        "wrong-control-va",
        guard_scoped(
            "expected_defer_faults", "va = int(marker[2], 16)", "va = int(marker[2], 16) + 4096"
        ),
        "native_xid_guard.py",
    ),
    ("unreadable-watch-admitted", function("watch_delta", "    return []")),
    (
        "other-page-type-admitted",
        guard_replace(
            "Counter(actual) != Counter(expected)",
            "Counter(x[:3]+x[4:] for x in actual) != Counter(x[:3]+x[4:] for x in expected)",
        ),
        "native_xid_guard.py",
    ),
    (
        "non-ce-client-admitted",
        guard_replace(
            'return re.fullmatch(r"HUBCLIENT_CE[0-9]+", name) is not None', "return True"
        ),
        "native_xid_guard.py",
    ),
    (
        "wrong-arm-admitted",
        replace("            return not delta", '            arm = "--defer-liveness"'),
    ),
    ("xid-refusal-status-forged", replace("    return 0 if ok else 6", "    return 0")),
    (
        "arm-watch-not-installed",
        replace(
            '            BASH_ENV=str(root / "scripts/ci/vast_payload_hooks.sh"),',
            '            BASH_ENV="/nonexistent-v7-hook",',
        ),
    ),
]

CASES += [
    ("diagnostics-not-posted", replace("    arm_diagnostic(arm, text, rc, ok)", "    pass")),
    ("diagnostics-head-lost", replace("raw[:16384]", "b''")),
    ("diagnostics-tail-lost", replace("raw[-16384:]", "b''")),
    ("diagnostics-unbounded", replace("if len(raw) > 32768:", "if False:")),
    ("diagnostics-no-fault-reserve", replace("1048576 - (65536 if admitted else 0)", "1048576")),
    ("failed-arm-row-lost", replace("            append_arm_failure(result)", "            pass")),
    (
        "guest-supervisor-bypassed",
        replace(
            "result = xid_guard.supervise_guest(binary, argv, receipt, revision, root, serial, qlog)",
            "result = 0",
        ),
    ),
    (
        "guest-refusal-status-forged",
        scoped("guest_command", "    rc = 6", "    rc = 0"),
    ),
    (
        "guest-allowed-records-lost",
        scoped("guest_command", 'approved.write(json.dumps(xid_record_key(line)) + "\\n")', "pass"),
    ),
    (
        "native-source-channel-ignored",
        guard_replace(
            "Counter(actual) != Counter(expected)",
            "Counter(x[:1]+x[2:] for x in actual) != Counter(x[:1]+x[2:] for x in expected)",
        ),
        "native_xid_guard.py",
    ),
    (
        "native-source-va-ignored",
        guard_replace(
            "Counter(actual) != Counter(expected)",
            "Counter(x[:2]+x[3:] for x in actual) != Counter(x[:2]+x[3:] for x in expected)",
        ),
        "native_xid_guard.py",
    ),
    (
        "native-extra-record-ignored",
        guard_replace("Counter(actual) != Counter(expected)", "set(actual) != set(expected)"),
        "native_xid_guard.py",
    ),
    (
        "native-ce-instance-hardcoded",
        guard_replace(
            'return re.fullmatch(r"HUBCLIENT_CE[0-9]+", name) is not None',
            'return name == "HUBCLIENT_CE1"',
        ),
        "native_xid_guard.py",
    ),
    (
        "guest-ce-instance-hardcoded",
        guard_replace(
            "not copy_engine_client(record['client'])", "record['client'] != 'HUBCLIENT_CE1'"
        ),
        "native_xid_guard.py",
    ),
    (
        "native-entire-proof-bypassed",
        guard_replace("Counter(actual) != Counter(expected)", "False"),
        "native_xid_guard.py",
    ),
    (
        "native-receipt-not-created",
        scoped("guard_command", "            retain_native_control(arm, text)", "            pass"),
    ),
    (
        "native-receipt-refusal-ignored",
        scoped("guard_command", "            ok = False", "            ok = True"),
    ),
    (
        "guest-diagnostics-not-posted",
        replace(
            '        arm_diagnostic(arm, "\\n".join(parts), rc, admitted, mode="guest")',
            "        pass",
        ),
    ),
    (
        "guest-stop-flag-lost",
        scoped(
            "guest_command",
            '        Path(os.environ["KF_PAYLOAD_XID_FAILED"]).touch()',
            "        pass",
        ),
    ),
    (
        "timeout-options-mistaken-for-arm",
        HOOK.replace("--missing-page-fault|--defer-liveness)", "--missing-page-fault)").replace(
            '[ "$kind" != raw ] || [ -n "$arm" ] || arm=$arg', '[ -n "$arm" ] || arm=$arg'
        ),
        "vast_payload_hooks.sh",
    ),
    (
        "guest-log-descriptor-becomes-subshell-pipe",
        HOOK.replace(
            'qlog=$(readlink "/proc/$caller_pid/fd/1")', 'qlog=$(readlink "/proc/$BASHPID/fd/1")'
        ),
        "vast_payload_hooks.sh",
    ),
]

CASES += [
    ("structured-arm-logs-omitted", replace('snapshot["diagnostics"] = events', "pass")),
    (
        "local-reader-rejects-arm-logs",
        report_replace('            "diagnostics",\n', ""),
        "vast_report.py",
    ),
    (
        "local-reader-skips-log-validation",
        report_replace('            validate_diagnostics(doc["diagnostics"])', "            pass"),
        "vast_report.py",
    ),
    (
        "local-log-preview-loses-head",
        report_replace('head = raw[:half].decode("utf-8", errors="ignore")', 'head = ""'),
        "vast_report.py",
    ),
    (
        "local-log-preview-loses-tail",
        report_replace('tail = raw[-half:].decode("utf-8", errors="ignore")', 'tail = ""'),
        "vast_report.py",
    ),
    (
        "local-log-hash-covers-preview-only",
        report_replace(
            "sha256=hashlib.sha256(raw).hexdigest()",
            "sha256=hashlib.sha256(preview.encode()).hexdigest()",
        ),
        "vast_report.py",
    ),
    (
        "local-log-manifest-budget-disabled",
        report_replace(
            'if budget and len(json.dumps(records, separators=(",", ":")).encode()) > 49152:',
            "if False:",
        ),
        "vast_report.py",
    ),
    (
        "local-log-first-refusal-can-be-evicted",
        report_replace(
            'first_fault = next((i for i, item in enumerate(result) if not item["admitted"]), None)',
            "first_fault = None",
        ),
        "vast_report.py",
    ),
    (
        "local-logs-leak-into-worker-wire-schema",
        report_replace(
            "        **reference,\n    }",
            '        **reference,\n        "diagnostics": [],\n    }',
        ),
        "vast_report.py",
    ),
    (
        "local-log-boolean-rc-admitted",
        report_replace('type(item["rc"]) is not int', 'not isinstance(item["rc"], int)'),
        "vast_report.py",
    ),
    (
        "local-log-invalid-source-hash-admitted",
        report_replace(
            're.fullmatch(r"[0-9a-f]{64}", item["sha256"])', 're.fullmatch(r".*", item["sha256"])'
        ),
        "vast_report.py",
    ),
    (
        "summary-not-printed-on-failure",
        scoped(
            "execute_stages",
            '            print("KF_PAYLOAD_FAIL suite=" + failure, flush=True)\n            print_final_summary(result)',
            '            print("KF_PAYLOAD_FAIL suite=" + failure, flush=True)',
        ),
    ),
    (
        "summary-not-printed-on-pass",
        scoped(
            "execute_stages",
            "    persist_result(destination, result)\n    print_final_summary(result)\n    return 0",
            "    persist_result(destination, result)\n    return 0",
        ),
    ),
    (
        "summary-printed-before-the-failure-line",
        scoped(
            "execute_stages",
            '            print("KF_PAYLOAD_FAIL suite=" + failure, flush=True)\n            print_final_summary(result)',
            '            print_final_summary(result)\n            print("KF_PAYLOAD_FAIL suite=" + failure, flush=True)',
        ),
    ),
    (
        "summary-missing-after-preflight-failure",
        replace(
            '        print("KF_PAYLOAD_FAIL suite=" + error.name, flush=True)\n        print_final_summary(result)',
            '        print("KF_PAYLOAD_FAIL suite=" + error.name, flush=True)',
        ),
    ),
    (
        "summary-ignores-the-progress-file",
        replace(
            'files = [*stage["progress"], stage["output"], *stage.get("detail", [])]',
            'files = [stage["output"]]',
        ),
    ),
    (
        "summary-budget-removed",
        scoped(
            "final_summary", "budget = SUMMARY_LIMIT - len(begin)", "budget = 10**9 - len(begin)"
        ),
    ),
    (
        "summary-failure-queued-behind-passes",
        scoped(
            "final_summary",
            '[("stage", i, s) for i, s in enumerate(stage_rows) if i in failed]\n        + [("fail"',
            '[]\n        + [("fail"',
        ),
    ),
    (
        "summary-line-not-cut",
        scoped(
            "printable",
            'return line if len(line) <= limit else line[: limit - 3] + "..."',
            "return line",
        ),
    ),
    (
        "summary-keeps-control-characters",
        scoped("printable", 'c if " " <= c <= "~" else', "c if True else"),
    ),
    (
        "summary-first-red-line-is-the-last",
        scoped(
            "first_failing_line",
            "            if is_failing(line):\n                return printable(line)",
            "            if is_failing(line):\n                last = line",
        ),
    ),
    (
        "summary-cause-from-the-wrong-end",
        scoped("step_cause", "lines[-3:]", "lines[:3]"),
    ),
    (
        "summary-timeout-not-marked",
        scoped("execute_stages", 'result="TIMEOUT" if rc == 124 else verdict', "result=verdict"),
    ),
    (
        "kvm-totals-not-checked",
        replace(
            "        if passed < 1 or failed or ran < 1 or skipped:",
            "        if False:",
        ),
    ),
    (
        "kvm-rc-ignored",
        replace(
            '    if re.search(r"^KVM_TESTS_RC=[1-9]", text, re.M):',
            "    if False:",
        ),
    ),
    (
        "kvm-red-lines-not-red",
        replace("|CARGO|KVM_TESTS)_RC=", "|CARGO)_RC="),
    ),
    (
        "crate-tests-row-required-again",
        replace(
            '    requested = len(re.findall(r"^KVM_TESTS_REQUESTED=1$", text, re.M))',
            '    only(r"^TESTS passed ([1-9][0-9]*) failed 0$", text, "kf_tests", rows)\n'
            '    requested = len(re.findall(r"^KVM_TESTS_REQUESTED=1$", text, re.M))',
        ),
    ),
    (
        "failed-tests-not-listed",
        replace("            found = FAILED_TEST.match(line) or item", "            found = None"),
    ),
    (
        "failed-test-cap-removed",
        replace("    return seen[:limit], len(seen)", "    return seen, len(seen)"),
    ),
    (
        "panic-dropped",
        replace(
            '            record["panic"] = first_panic(files)', '            record["panic"] = ""'
        ),
    ),
    (
        "fold-launders-tokens",
        replace("            if is_test_identifier(m[0])", "            if True"),
    ),
    (
        "fold-pieces-too-long",
        replace("FOLD = 30", "FOLD = 40"),
    ),
    (
        "fold-digit-rule-dropped",
        replace("        and sum(c.isdigit() for c in run) * 5 <= len(run)\n", ""),
    ),
    (
        "summary-names-not-folded",
        replace('" name=" + fold_identifiers(printable(name))', '" name=" + printable(name)'),
    ),
    (
        "summary-panic-not-folded",
        replace('" line=" + fold_identifiers(record["panic"])', '" line=" + record["panic"]'),
    ),
    (
        "cargo-quiet-not-exported",
        replace('            CARGO_TERM_QUIET="true",\n', ""),
    ),
    (
        "cargo-colour-not-disabled",
        replace('            CARGO_TERM_COLOR="never",\n', ""),
    ),
    (
        "cargo-progress-not-disabled",
        replace('            CARGO_TERM_PROGRESS_WHEN="never",\n', ""),
    ),
    (
        "rustflags-weakened-by-the-payload",
        replace(
            '            CARGO_TERM_QUIET="true",\n',
            '            CARGO_TERM_QUIET="true",\n            RUSTFLAGS="-A warnings",\n',
        ),
    ),
]

COLLECT = """import io,json,unittest,importlib.util
from pathlib import Path
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_payload*.py')
for filename,names in [
 ('test_native_xid_guard',['NativeXidGuardTests']),
 ('test_native_xid_route_round2',['Round2NativeRouteReplay']),
 ('test_guest_xid_guard',['GuestGuardTests','NativeExactFiveTests']),
 ('test_owner_copy_family_rule',['OwnerCopyFamilyRule'])]:
 p=Path('scripts/bench/tests')/(filename+'.py')
 spec=importlib.util.spec_from_file_location(filename,p)
 m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
 for name in names:s.addTests(unittest.defaultTestLoader.loadTestsFromTestCase(getattr(m,name)))
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print('KF_MUTATION_RESULT='+json.dumps({'tests':r.testsRun,
'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run(root):
    p = subprocess.run(
        [sys.executable, "-c", COLLECT], cwd=root, text=True, capture_output=True, timeout=45
    )
    assert p.returncode == 0, p.stderr[-500:]
    line = next(s for s in p.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT="))
    return json.loads(line.split("=", 1)[1])


def main():
    rows = []
    with tempfile.TemporaryDirectory(prefix="kf-v7-mutants-") as directory:
        root = Path(directory)
        ci = root / "scripts/ci"
        ci.mkdir(parents=True)
        for name in [
            "vast_payload.py",
            "vast_payload.sh",
            "vast_payload_hooks.sh",
            "test_vast_payload.py",
            "test_vast_payload_v7.py",
            "test_vast_payload_admission.py",
            "test_vast_payload_diagnostics.py",
            "test_vast_payload_failures.py",
            "test_vast_payload_guest_guard.py",
            "test_vast_payload_result_logs.py",
            "test_vast_payload_summary.py",
            "test_vast_box.py",
            "vast_report.py",
        ]:
            shutil.copyfile(HERE / name, ci / name)
        bench = root / "scripts/bench"
        bench.mkdir()
        shutil.copyfile(HERE.parent / "bench/host_xid_watch.sh", bench / "host_xid_watch.sh")
        shutil.copyfile(HERE.parent / "bench/native_xid_guard.py", bench / "native_xid_guard.py")
        (bench / "tests").mkdir()
        for name in (
            "test_native_xid_guard.py",
            "test_native_xid_route_round2.py",
            "test_guest_xid_guard.py",
            "test_owner_copy_family_rule.py",
        ):
            shutil.copyfile(HERE.parent / "bench/tests" / name, bench / "tests" / name)
        fixtures = root / "ci/vast-broker/test/fixtures"
        fixtures.mkdir(parents=True)
        shutil.copyfile(
            HERE.parents[1] / "ci/vast-broker/test/fixtures/scrub-vectors.json",
            fixtures / "scrub-vectors.json",
        )
        for name in ("sweep_ada_20261018", "sweep_dies_20261018", "sweep_windows_20261021"):
            shutil.copytree(HERE.parents[1] / "traces" / name, root / "traces" / name)
        baseline = run(root)
        assert not baseline["killers"], baseline
        for case in CASES:
            name, source = case[:2]
            if len(sys.argv) > 1 and sys.argv[1] not in name:
                continue
            shared = len(case) == 3 and case[2] == "native_xid_guard.py"
            shell = len(case) == 3 and case[2] == "vast_payload_hooks.sh"
            reporter = len(case) == 3 and case[2] == "vast_report.py"
            assert source != (
                GUARD if shared else HOOK if shell else REPORT if reporter else SOURCE
            ), name
            if not shell:
                ast.parse(source)
            (ci / "vast_payload.py").write_text(SOURCE)
            (ci / "vast_payload_hooks.sh").write_text(HOOK)
            (bench / "native_xid_guard.py").write_text(GUARD)
            (ci / "vast_report.py").write_text(REPORT)
            target = (
                bench / case[2]
                if shared
                else ci / case[2] if shell or reporter else ci / "vast_payload.py"
            )
            target.write_text(source)
            result = run(root)
            killers = sorted(set(result["killers"]))
            assert len(killers) >= 2, (name, killers)
            rows.append({"mutant": name, "killers": killers, "count": len(killers)})
            print(name + ": " + str(len(killers)) + " killers", file=sys.stderr, flush=True)
        (ci / "vast_payload.py").write_text(SOURCE)
        (bench / "native_xid_guard.py").write_text(GUARD)
        (ci / "vast_report.py").write_text(REPORT)
        (ci / "vast_payload_hooks.sh").write_text(
            '#!/bin/bash\ntimeout() { command timeout "$@"; }\n'
        )
        result = run(root)
        killers = sorted(set(result["killers"]))
        assert len(killers) >= 2, ("arm-hook-bypassed", killers)
        rows.append({"mutant": "arm-hook-bypassed", "killers": killers, "count": len(killers)})
    print(json.dumps({"run": "v7-payload-mutants-20261023", "cases": rows}, indent=2))


if __name__ == "__main__":
    main()
