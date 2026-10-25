"""Run source mutants in isolated temporary copies; synthetic checks only."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
MUTANTS = {
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
