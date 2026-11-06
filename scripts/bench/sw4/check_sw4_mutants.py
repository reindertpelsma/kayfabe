"""Each changed behavioral guard must have two distinct killing tests."""

import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
MUTANTS = {
    "network_observer_error_ignored": ("network_guard.py", "        stop()", "        pass"),
    "old_die_scope": (
        "die_binding.py",
        'SCOPE = {"GB206", "GB207"}',
        'SCOPE = {"GB206", "GB207", "GA104"}',
    ),
    "host_blacklist_removed": ("offer_selector.py", ' or offer.get("host_id") in blacklisted', ""),
    "candidate_pin_removed": (
        "offer_selector.py",
        'raise OfferRefused("candidate outside SW4 grant")',
        "pass",
    ),
    "country_removed": (
        "offer_selector.py",
        'raise OfferRefused("candidate country is not granted Canada")',
        "pass",
    ),
    "requested_storage_rate_limit_removed": (
        "offer_selector.py",
        'raise OfferRefused("hourly price outside lane limit")',
        "pass",
    ),
    "budget_hourly_limit_raised": ("round_budget.py", "MAX_RATE = 0.60", "MAX_RATE = 2.50"),
    "budget_time_ceiling_raised": (
        "round_budget.py",
        "MAX_SECONDS = 120 * 60",
        "MAX_SECONDS = 150 * 60",
    ),
    "budget_cap_raised": ("round_budget.py", "CAP_USD = 2.00", "CAP_USD = 3.00"),
    "transfer_runtime_unbudgeted": (
        "round_budget.py",
        "stop_usd - NONRUNTIME_MARGIN - transfer_usd",
        "stop_usd - NONRUNTIME_MARGIN",
    ),
    "retained_margin_removed": (
        "round_budget.py",
        "NONRUNTIME_MARGIN = 0.50",
        "NONRUNTIME_MARGIN = 0.00",
    ),
    "billed_allocation_stop_removed": (
        "precreate_watchdog.py",
        'return retire("billed allocation stop")',
        "pass",
    ),
    "source_lock_removed": (
        "lifecycle.py",
        "    for filename, expected in files.items():",
        "    return\n    for filename, expected in files.items():",
    ),
    "create_before_arm": (
        "lifecycle.py",
        "    watch = backend.arm(plan)",
        "    backend.create()\n    watch = backend.arm(plan)",
    ),
    "capture_hash_removed": (
        "lifecycle.py",
        'raise ValueError("private capture hash mismatch")',
        "pass",
    ),
    "cancel_before_absence": (
        "lifecycle.py",
        "    if not backend.absent(iid):",
        "    backend.cancel(watch)\n    if not backend.absent(iid):",
    ),
    "download_stop_removed": ("network_guard.py", "download >= DOWNLOAD_BYTES", "False"),
    "upload_stop_removed": ("network_guard.py", "upload >= UPLOAD_BYTES", "False"),
    "network_invalid_counter_accepted": (
        "network_guard.py",
        'raise ValueError("invalid network byte counter")',
        "pass",
    ),
}
DRIVER = """
import json,unittest
from pathlib import Path
suite=unittest.TestSuite()
for pattern in ('test_sw4.py','test_cost_watch.py','test_precreate_watchdog.py'):
    suite.addTests(unittest.defaultTestLoader.discover('.',pattern=pattern))
result=unittest.TextTestRunner(verbosity=0).run(suite)
Path('result.json').write_text(json.dumps({
    'run':result.testsRun,
    'killers':sorted({t.id() for t,_ in result.failures+result.errors})}))
"""


def main():
    rows = []
    for name, (filename, before, after) in MUTANTS.items():
        with tempfile.TemporaryDirectory(prefix="sw4-mutant-") as tmp:
            root = Path(tmp)
            for source in HERE.glob("*.py"):
                shutil.copyfile(source, root / source.name)
            path = root / filename
            source = path.read_text()
            if source.count(before) != 1:
                raise ValueError("nonunique mutation site: " + name)
            path.write_text(source.replace(before, after))
            subprocess.run(
                [sys.executable, "-c", DRIVER],
                cwd=root,
                capture_output=True,
                check=True,
                timeout=30,
            )
            result = json.loads((root / "result.json").read_text())
        rows.append({"mutant": name, **result})
        print(name, len(result["killers"]), flush=True)
        if len(result["killers"]) < 2:
            raise ValueError("fewer than two killing tests: " + name)
    Path(sys.argv[1]).write_text(json.dumps(rows, indent=2) + "\n")


if __name__ == "__main__":
    main()
