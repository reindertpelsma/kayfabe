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
            'return re.fullmatch(r"HUBCLIENT_(?:CE|HSCE)[0-9]+", name) is not None', "return True"
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
            'return re.fullmatch(r"HUBCLIENT_(?:CE|HSCE)[0-9]+", name) is not None',
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

# wtier, 2026-11-05: the release tier's payload extras (KF_WINDOWS_RUNS, KF_STAGES).
CASES += [
    (
        "second-windows-run-reuses-row-name",
        replace("            return [row(name, 1)]", '            return [row("windows_gate", 1)]'),
    ),
    (
        "design-only-stage-runs",
        replace('        or any(STAGE_SLOTS[n] == "DESIGN_ONLY" for n in names)\n', ""),
    ),
]

# 2026-11-07, lane appslinux: the Linux apps stage (draft, disabled by default).
CASES += [
    ("apps-stage-enabled-by-default", replace("APPS_STAGE_ENABLED = False", "APPS_STAGE_ENABLED = True")),
    (
        "apps-run-for-small-tier",
        replace("    return bool(APPS_STAGE_ENABLED) and tier in APPS_TIERS", "    return bool(APPS_STAGE_ENABLED)"),
    ),
    (
        "apps-missing-app-passes",
        replace("            and sorted(seen) == sorted(names)\n", ""),
    ),
    (
        "apps-failing-app-passes",
        replace("            and passed == len(names)\n", ""),
    ),
    (
        "apps-nonzero-exit-passes",
        replace('            and exits[0] == "0"\n            and listed', "            and listed"),
    ),
    (
        "apps-list-size-unchecked",
        replace("            and listed == [str(len(names))]\n", ""),
    ),
    (
        "apps-retry-passes",
        replace('        if kind == "RETRY":\n            name = "apps_retry_"', '        if kind == "RETRY":\n            return [row("apps", len(names))]\n            name = "apps_retry_"'),
    ),
    (
        "apps-stage-without-time-guard",
        replace("APPS_MIN_REMAINING_MS = 60 * 60000", "APPS_MIN_REMAINING_MS = 0"),
    ),
    (
        "apps-stage-selected-by-kf-stages",
        replace('STAGE_SLOTS = {"specviewperf": "PLUMBING",', 'STAGE_SLOTS = {"apps": "PLUMBING", "specviewperf": "PLUMBING",'),
    ),
    (
        "apps-middle-reads-the-full-list",
        replace('"middle": ("gate_list_middle.txt", "middle")', '"middle": ("gate_list.txt", "middle")'),
    ),
    (
        "apps-middle-runs-the-full-lane-list",
        replace('"middle": ("gate_list_middle.txt", "middle")', '"middle": ("gate_list_middle.txt", "full")'),
    ),
    (
        "apps-tier-ignored-when-reading-the-list",
        replace("            names = apps_gate_list(root, tier)", '            names = apps_gate_list(root, "large")'),
    ),
    (
        "apps-stage-never-appended",
        replace(
            '        if apps_requested(tier):\n',
            '        if False:\n',
        ),
    ),
]

# 2026-11-07: the SPECviewperf stage (draft, disabled by default; docs/design/V3_SPEC_GATE.md).
CASES += [
    (
        "spec-slot-without-a-lane-runs",
        replace('        if wanted:\n            raise EvidenceError("preflight")\n        return None', "        return None"),
    ),
    ("spec-lane-without-the-slot-runs", replace("    if value not in SPEC_LANES or not wanted:", "    if value not in SPEC_LANES:")),
    (
        "spec-without-the-windows-gate-runs",
        replace('    if wanted and not windows:\n        raise EvidenceError("preflight")\n', ""),
    ),
    ("spec-lane-unvalidated", replace("    if value not in SPEC_LANES or not wanted:", "    if not wanted:")),
    (
        "spec-pass-with-a-missing-row",
        replace(
            "[(lane, str(len(names)))]\n        and sorted(seen) == sorted(names)\n",
            "[(lane, str(len(names)))]\n",
        ),
    ),
    (
        "spec-pass-with-a-failing-row",
        replace(
            "        and sorted(seen) == sorted(names)\n        and passed == len(names)\n    ):\n        out = [",
            "        and sorted(seen) == sorted(names)\n    ):\n        out = [",
        ),
    ),
    ("spec-pass-with-a-nonzero-exit-line", replace('        and exits == ["0"]\n', "")),
    ("spec-pass-for-the-wrong-lane", replace("        and SPEC_LIST.findall(text) == [(lane, str(len(names)))]\n", "")),
    ("spec-pass-with-a-nonzero-rc", replace("        and rc == 0\n        and exits", "        and exits")),
    (
        "spec-retry-passes",
        replace(
            '    if kind == "RETRY":\n        name = "specviewperf_retry_"',
            '    if kind == "RETRY":\n        return [row("specviewperf", len(names))]\n        name = "specviewperf_retry_"',
        ),
    ),
    (
        "spec-fail-is-a-retry",
        replace('    if kind == "FAIL":\n        raise EvidenceError("specviewperf", partial)', '    if kind == "FAIL":\n        raise EvidenceError("specviewperf_retry_operator", partial)'),
    ),
    ("spec-unknown-retry-class-trusted", replace('cls.lower() if cls in SPEC_RETRY_CLASSES else "operator"', "cls.lower()")),
    ("spec-stage-without-time-guard", replace("min_remaining_ms=spec_min_remaining_ms(spec_tbl, spec), short_row", "min_remaining_ms=0, short_row")),
    ("spec-min-remaining-ignores-the-native-walls", replace('table["factor"] * native', "native")),
    ("spec-windows-disk-not-sized", replace('            os.environ.setdefault("WINVM_DISK_GB", SPEC_DISK_GB)', "            pass")),
    ("spec-command-always-lane-both", replace('"KF_SPEC_LANE=" + spec,', '"KF_SPEC_LANE=both",')),
    ("spec-deadline-without-margin", replace("SPEC_DEADLINE_MARGIN_S = 120", "SPEC_DEADLINE_MARGIN_S = 0")),
    (
        "spec-both-is-one-lane",
        replace('        return list(table["lanes"]["lanea"]) + list(table["lanes"]["laneb"])', '        return list(table["lanes"]["laneb"])'),
    ),
    ("spec-pass-without-lane-rows", replace('        out = [row("specviewperf", len(names))]\n', '        return [row("specviewperf", len(names))]\n        out = []\n')),
]

# 2026-11-06: the setup picker's host claims (optional, bounded, never a verdict).
CASES += [
    (
        "claims-sent-to-a-worker-that-did-not-ask",
        report_replace(
            '**({"host_claims": claims} if claims and env.get("KF_HOST_CLAIMS") == "1" else {}),',
            '**({"host_claims": claims} if claims else {}),',
        ),
        "vast_report.py",
    ),
    (
        "claims-forwarded-uncleaned",
        report_replace(
            'claims = {k: v for k, v in clean_claims(doc["host_claims"]).items() if v is not None}',
            'claims = dict(doc["host_claims"]) if isinstance(doc["host_claims"], dict) else {}',
        ),
        "vast_report.py",
    ),
    (
        "claims-variable-not-in-the-env-contract",
        report_replace('    ("KF_HOST_CLAIMS", False, _match(r"1")),\n', ""),
        "vast_report.py",
    ),
    (
        "claims-driver-pattern-loose",
        report_replace(
            'CLAIM_DRIVER = re.compile(r"[0-9]{3,4}\\.[0-9]{1,5}(?:\\.[0-9]{1,5})?")',
            'CLAIM_DRIVER = re.compile(r".{1,16}")',
        ),
        "vast_report.py",
    ),
    (
        "claims-identifier-check-removed",
        report_replace(
            "    return None if CLAIM_IDENTIFIER.search(value) or CLAIM_TOKEN.search(value) else value",
            "    return value",
        ),
        "vast_report.py",
    ),
    (
        "claims-windows-guest-without-the-stage",
        replace('    if windows:\n        raw["windows_os_version"]', '    if True:\n        raw["windows_os_version"]'),
    ),
    (
        "claims-model-not-sanitised",
        replace('            model = re.sub(r"[^A-Za-z0-9 ().@+/_-]", " ", value)', "            model = value"),
    ),
    (
        "claims-error-fails-the-run",
        replace(
            "    except (OSError, ValueError, UnicodeError, EvidenceError):\n        return\n    if claims:",
            "    except ZeroDivisionError:\n        return\n    if claims:",
        ),
    ),
    (
        "claims-numeric-model-line-is-the-model",
        replace('        elif key == "model name" and model is None and value:', '        elif key in ("model name", "model") and model is None and value:'),
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

CASES += [
    ("winapps-all-output-is-pass", function("winapps_rows", '    return [row("windows_apps", 8)]')),
    ("winapps-never-passes", function("winapps_rows", '    raise EvidenceError("windows_apps_retry_operator")')),
    ("winapps-enabled-by-default", scoped("winapps_requested", '        return None', '        return "small"')),
    ("winapps-request-ignored", function("winapps_requested", '    environ.pop("KF_WINAPPS", None)\n    return None')),
    ("winapps-incomplete-output-is-pass", scoped("winapps_rows", 'if (len(grade) == 1 and count and grade[0][0] == grade[0][1] == count[1]\n                    and len(app_lines) == len(passed) == len(set(passed)) == int(count[1])):', 'if True:')),
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
        [sys.executable, "-c", COLLECT], cwd=root, text=True, capture_output=True, timeout=180
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
            "test_vast_payload_claims.py",
            "test_vast_payload_apps.py",
            "test_vast_payload_winapps.py",

            "test_vast_payload_spec.py",
            "host_claims_vectors.json",
            "test_vast_box.py",
            "vast_report.py",
        ]:
            shutil.copyfile(HERE / name, ci / name)
        bench = root / "scripts/bench"
        bench.mkdir()
        # The claims test checks its guest constants against the scripts that install the guests.
        for relative in (
            "provision_bench_tree.sh",
            "windows/win_vm.sh",
            "box/windows_gate.sh",
        ):
            (bench / relative).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(HERE.parent / "bench" / relative, bench / relative)
        # The apps stage tests run the lane script and ask run_apps.sh for the app names.
        (bench / "box").mkdir(exist_ok=True)
        shutil.copyfile(HERE.parent / "bench/box/apps_gate.sh", bench / "box/apps_gate.sh")
        (root / "scripts/apps").mkdir()
        for name in ("run_apps.sh", "gate_list.txt", "gate_list_middle.txt"):
            shutil.copyfile(HERE.parent / "apps" / name, root / "scripts/apps" / name)
        for relative in ("g1/gate_full.out", "g2/gate_mid.out", "g3/gate_watched.out", "g4/gate_fail.out"):  # real lane output, replayed by the tests
            target = root / "traces/apps_linux_20261107" / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(HERE.parents[1] / "traces/apps_linux_20261107" / relative, target)
        # The SPECviewperf stage tests read the lane table and the Worker's stage slots.
        (bench / "specviewperf").mkdir()
        shutil.copyfile(HERE.parent / "bench/specviewperf/lanes.json", bench / "specviewperf/lanes.json")
        (root / "ci/vast-broker").mkdir(parents=True)
        shutil.copyfile(HERE.parents[1] / "ci/vast-broker/defaults.json", root / "ci/vast-broker/defaults.json")
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
