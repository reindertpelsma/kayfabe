"""Run source mutants in isolated temporary copies; synthetic checks only."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
MUTANTS = {
    "preupload_console_capture_lost": (
        "backend.py",
        'if not row.get("work_started"):',
        "if False:",
    ),
    "preupload_marked_remote_work": (
        "backend.py",
        'for name in ("upload.py", "parallel_eight.py"):',
        'row["work_started"] = True\n            provider.save(ledger, [row])\n'
        '            for name in ("upload.py", "parallel_eight.py"):',
    ),
    "upload_relative_paths_accepted": (
        "upload_wiring.py",
        "if not path.is_absolute():",
        "if False:",
    ),
    "upload_current_ledger_lost": (
        "upload_wiring.py",
        "read.func.value = path_value(ledger)",
        "pass",
    ),
    "upload_current_kit_lost": (
        "upload_wiring.py",
        "single(matches, name).value = path_value(value)",
        'single(matches, name).value = path_value(value) if name != "K" '
        "else single(matches, name).value",
    ),
    "upload_extras_lost": (
        "upload_wiring.py",
        "tree.body.insert(tree.body.index(bundle), update)",
        "pass",
    ),
    "upload_anchor_guard_lost": ("upload_wiring.py", "if len(nodes) != 1:", "if False:"),
    "grant5_limits_ignored": ("execution.py", "return 1.4, 3.0, 3.5", "return 1, 2, 2.5"),
    "unknown_grant_accepted": (
        "execution.py",
        'raise ValueError("unknown lease grant")',
        "return 1.4, 3, 3.5",
    ),
    "fresh_grant5_ignored": (
        "execution.py",
        'if grant in {"grant5", "grant6"} and not any(',
        "if False and not any(",
    ),
    "actual_rate_guard_removed": (
        "execution.py",
        'not 0 < rate <= min(plan.get("rate_ceiling", 1.0), plan["rate_bound"])',
        "False",
    ),
    "ready_amendment_lost": (
        "execution.py",
        'SimpleNamespace(STOP_USD=plan["stop_usd"], MAX_RATE=plan.get("rate_ceiling", 1.0))',
        "SimpleNamespace(STOP_USD=2, MAX_RATE=1)",
    ),
    "parser_windows_eol_rejected": (
        "execution.py",
        "BW8_PARSER_GREEN=1[[:space:]]*$",
        "BW8_PARSER_GREEN=1$",
    ),
    "parser_marker_prefix_accepted": (
        "execution.py",
        "BW8_PARSER_GREEN=1[[:space:]]*$",
        "BW8_PARSER_GREEN=1",
    ),
    "network_arm_removed": ("execution.py", "anchor + launch", "anchor"),
    "offer_verification_removed": (
        "execution.py",
        'offer.get("verification") != "verified"',
        "False",
    ),
    "offer_vm_removed": (
        "execution.py",
        'offer.get("vms_enabled") is not True or offer.get("is_vm_deverified") is not False',
        "False",
    ),
    "offer_gpu_removed": (
        "execution.py",
        'offer.get("gpu_name") != "RTX 5090" or offer.get("num_gpus") != 1',
        "False",
    ),
    "offer_country_removed": (
        "execution.py",
        'not country or country[1] in {"CU", "IR", "KP", "SY", "CN", "HK", "MO"}',
        "False",
    ),
    "offer_blacklist_removed": (
        "execution.py",
        'offer.get("machine_id") in blacklisted or offer.get("host_id") in blacklisted',
        "False",
    ),
    "offer_resources_removed": ("execution.py", "number(offer.get(field)) < floor", "False"),
    "offer_storage_rate_removed": ("execution.py", "not 0 < rate <= ceiling", "False"),
    "invalid_numbers_accepted": (
        "execution.py",
        "type(value) not in (int, float) or not math.isfinite(value) or value < 0",
        "False",
    ),
    "plan_rate_removed": ("execution.py", "not 0 < hourly <= ceiling", "False"),
    "plan_clock_removed": (
        "execution.py",
        "now.utcoffset() is None or deadline.utcoffset() is None",
        "False",
    ),
    "plan_shared_liability_removed": ("execution.py", "not stop < cap", "False"),
    "plan_fit_removed": ("execution.py", "seconds < estimated_seconds or seconds <= 0", "False"),
    "plan_reserve_removed": (
        "execution.py",
        "reservation >= stop or credit - reservation < 8",
        "False",
    ),
    "plan_runtime_ceiling_removed": (
        "execution.py",
        "min(6300, cost_seconds, wall_seconds)",
        "min(99999, cost_seconds, wall_seconds)",
    ),
    "plan_cap_ceiling_raised": (
        "execution.py",
        "min(grant_cap, number(shared_remaining))",
        "min(99, number(shared_remaining))",
    ),
    "parser_not_staged": (
        "execution.py",
        '"bw8_enable_observe.ps1 bw8_parser.ps1 diag_min.ps1"',
        '"bw8_enable_observe.ps1 diag_min.ps1"',
    ),
    "parser_gate_removed": ("execution.py", "preflight + anchors[0]", "anchors[0]"),
    "execution_anchor_removed": ("execution.py", "if text.count(old) != 1:", "if False:"),
    "qemu_admission_removed": ("execution.py", "pin + plans[0]", "plans[0]"),
    "settled_pin_requirement_removed": (
        "execution.py",
        'if "C.settle_pin(" not in body:',
        "if False:",
    ),
    "execution_source_hash_ignored": (
        "backend.py",
        "hashlib.sha256(Path(name).read_bytes()).hexdigest() != expected",
        "False",
    ),
    "create_retry_allowed": (
        "backend.py",
        'with attempt.open("x") as stream:',
        'with attempt.open("w") as stream:',
    ),
    "create_without_watchdog": (
        "backend.py",
        (
            "if self.watch.poll() is not None:\n                "
            'raise ValueError("precreate watchdog absent")'
        ),
        'if False:\n                raise ValueError("precreate watchdog absent")',
    ),
    "owned_id_not_reported": ("backend.py", 'if receipt.get("id"):', "if False:"),
    "capture_recheck_removed": ("capture.py", 'actual != item["sha256"]', "False"),
    "partial_pull_accepted": (
        "capture.py",
        'row["failed"] or row["not_pulled"] or row["verified"] != len(row["files"])',
        "False",
    ),
    "archive_hash_ignored": ("capture.py", "actual != expected", "False"),
    "archive_size_bounds_removed": (
        "capture.py",
        "len(files) > max_files or sum(p.stat().st_size for p, _ in files) > max_bytes",
        "False",
    ),
    "archive_member_path_ignored": (
        "capture.py",
        'not member.isfile() or name.is_absolute() or ".." in name.parts',
        "False",
    ),
    "archive_root_ignored": (
        "capture.py",
        'not name.parts or name.parts[0] not in ("evidence", "vm_logs", "bench")',
        "False",
    ),
    "static_failures_ignored": (
        "rehearse.py",
        'body.get("static") == []',
        "True",
    ),
    "budget_timer_witness_ignored": (
        "rehearse.py",
        'budget.get("absence_receipt") is True\n        and budget.get("destroy_called") is True',
        "True",
    ),
    "precreate_timer_witness_ignored": (
        "rehearse.py",
        'precreate.get("state") == "ARMED"\n'
        '        and precreate.get("result") == "no contract before deadline"',
        "True",
    ),
    "rehearsal_count_unchecked": (
        "rehearse.py",
        "len(rows) != len(SUITES)",
        "False",
    ),
    "rehearsal_suite_set_unchecked": (
        "rehearse.py",
        '{r["suite"] for r in rows} != set(SUITES)',
        "False",
    ),
    "rehearsal_failures_ignored": (
        "rehearse.py",
        'r["ok"] is True',
        "True",
    ),
    "rehearsal_zero_tests_accepted": (
        "rehearse.py",
        'r["ran"] > 0',
        "True",
    ),
    "git_context_leaks_to_temporary_repositories": (
        "rehearse.py",
        'if not key.startswith("GIT_")',
        "if True",
    ),
    "overlay_hashes_binary_instead_of_operator": (
        "prepare.py",
        "pathlib.Path(sys.argv[5]).read_bytes()",
        "pathlib.Path(sys.argv[4]).read_bytes()",
    ),
    "baseline_hash_overwrites_overlay_hash": (
        "prepare.py",
        "\"'baseline_harness_sha256':\"",
        "\"'harness_sha256':\"",
    ),
    "new_fatal_dump_ignored": (
        "observer.py",
        'require(dump["code"] not in FATAL_DUMPS,',
        "require(True,",
    ),
    "dump_generation_ignored": (
        "observer.py",
        'dump["id"] not in baseline',
        'dump["id"].split("|")[0] not in {item.split("|")[0] for item in baseline}',
    ),
    "system_bugcheck_reboot_ignored": (
        "observer.py",
        'require(False, "System bugcheck/reboot/TDR since enable")',
        'require(True, "System bugcheck/reboot/TDR since enable")',
    ),
    "application_provider_unchecked": (
        "observer.py",
        'provider == "Windows Error Reporting" and event["id"] == 1001',
        "True",
    ),
    "boot_identity_unchecked": (
        "observer.py",
        'require(utc(row["boot"]) == boot,',
        "require(True,",
    ),
    "wall_clock_unchecked": ("observer.py", "abs(delta - (mono - initial_mono)) <= 2", "True"),
    "coverage_clock_unchecked": ("observer.py", "index == 0 or 0 <= mono - previous <= 10", "True"),
    "sixty_second_hold_removed": ("observer.py", "previous - first_code0 >= 60", "True"),
    "readiness_deadline_removed": (
        "observer.py",
        "first_code0 - enable <= readiness_seconds",
        "True",
    ),
    "driver_identity_unchecked": (
        "observer.py",
        'row["driver"] == DRIVER and type(row["code"]) is int',
        'type(row["code"]) is int',
    ),
    "code0_loss_ignored": ("observer.py", 'require(row["code"] == 0,', "require(True,"),
    "code0_invented": (
        "observer.py",
        'require(first_code0 is not None, "no Code0")',
        "first_code0 = enable if first_code0 is None else first_code0",
    ),
    "stream_topology_unchecked": (
        "observer.py",
        'require(row["kind"] == expected_kind,',
        "require(True,",
    ),
    "partial_queries_accepted": (
        "observer.py",
        'isinstance(row, dict) and row["complete"] is True',
        "isinstance(row, dict)",
    ),
    "unclassified_dump_accepted": (
        "observer.py",
        'require(dump["code"] in KNOWN_DUMPS,',
        "require(True,",
    ),
    "event_cursor_unchecked": (
        "observer.py",
        'and event["record_id"] > cursors[channel]',
        "and True",
    ),
    "disabled_start_unchecked": (
        "observer.py",
        'start["kind"] == "start" and start["code"] == 22',
        'start["kind"] == "start"',
    ),
    "baseline_channel_cursors_unchecked": (
        "observer.py",
        'set(cursors) == {"System", "Application"}',
        "True",
    ),
    "partial_jsonl_accepted": ("observer.py", 'and text.endswith("\\n")', "and True"),
    "inventory_inputs_unchecked": ("prepare.py", "return plan == inventory()", "return True"),
    "effective_flags_unchecked": ("prepare.py", 'actual["flags"] == expected and', "True and"),
    "binary_hash_unchecked": (
        "prepare.py",
        'baseline["sha256"] != QEMU or actual["sha256"] != QEMU',
        "False",
    ),
    "device_harness_configuration_unchecked": (
        "prepare.py",
        "actual[key] != baseline[key]",
        "False",
    ),
    "overlay_anchor_check_removed": ("prepare.py", "if text.count(old) != 1:", "if False:"),
    "inherited_readiness_wait_removed": ("prepare.py", "'wait \"$init_observer_pid\"\\n'", "''"),
}

RUNNER = """
import json, unittest
class Results(unittest.TextTestResult):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.killed = []
    def addFailure(self, test, err):
        self.killed.append(test.id())
        super().addFailure(test, err)
    def addError(self, test, err):
        self.killed.append(test.id())
        super().addError(test, err)
suite = unittest.defaultTestLoader.discover('.', pattern='test_*.py')
result = unittest.TextTestRunner(resultclass=Results).run(suite)
print(json.dumps({'tests': result.testsRun, 'killers': sorted(set(result.killed))}))
"""


def main():
    result = {}
    for name, (filename, old, new) in MUTANTS.items():
        with tempfile.TemporaryDirectory(prefix="bw8-mutant-") as folder:
            root = Path(folder)
            for source in HERE.iterdir():
                if source.suffix in (".py", ".ps1"):
                    (root / source.name).write_bytes(source.read_bytes())
            path = root / filename
            text = path.read_text()
            if text.count(old) != 1:
                raise ValueError("mutant anchor missing/duplicated: " + name)
            path.write_text(text.replace(old, new))
            process = subprocess.run(
                [sys.executable, "-B", "-c", RUNNER],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=30,
            )
            row = json.loads(process.stdout)
            if len(row["killers"]) < 2:
                raise ValueError("fewer than two killing tests: " + name + " " + str(row))
            result[name] = row
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
