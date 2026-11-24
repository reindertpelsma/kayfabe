#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of scripts/windows/kayfabe-windows-setup.ps1 (STATUS: LIVE, 2026-11-20): every mutant needs two named killers.

Each mutant is one exact-once text replacement in a private copy of scripts/windows; the named tests of
scripts/ci/test_windows_guest_setup.py run against the copy (KF_WINSETUP_DIR), and a mutant is killed only when at least TWO of
its named tests fail. The unmutated copy must be green first. Needs pwsh (PowerShell 7; preinstalled on GitHub's ubuntu runners,
KF_PWSH names another binary); a missing pwsh is a failure, not a skip. GPU-free, no registry. Same shape as
open_rows_mutants.py.

    python3 scripts/ci/windows_guest_setup_mutants.py [--jobs N] [substring-of-mutant-name]
"""
import argparse
import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SRC = ROOT / "scripts" / "windows"
PS1 = (SRC / "kayfabe-windows-setup.ps1").read_text()
SUITE = "test_windows_guest_setup."
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""


def K(*names):
    return [SUITE + n.split(".")[0] + ".test_" + n.split(".")[1] for n in names]


MUTANTS = [
    # (name, text that occurs exactly once in the script, its replacement, named killers)
    ("wrong-value-service-key", "Path = 'SYSTEM\\CurrentControlSet\\Services\\nvlddmkm'; Name = 'EnableGpuFirmware'; Type = 'DWord'; Value = 1",
     "Path = 'SYSTEM\\CurrentControlSet\\Services\\nvlddmkm'; Name = 'EnableGpuFirmware'; Type = 'DWord'; Value = 0",
     K("Table.the_script_table_equals_the_json", "Pre.pre_sets_the_service_and_dwm_values_and_creates_the_missing_service_key", "Post.post_sets_the_class_key_at_the_discovered_index")),
    ("wrong-value-dwm", "Name = 'OverlayTestMode'; Type = 'DWord'; Value = 5", "Name = 'OverlayTestMode'; Type = 'DWord'; Value = 4",
     K("Table.the_script_table_equals_the_json", "Pre.pre_sets_the_service_and_dwm_values_and_creates_the_missing_service_key", "Post.post_sets_the_class_key_at_the_discovered_index")),
    ("wrong-value-class-key", "<index>'; Name = 'EnableGpuFirmware'; Type = 'DWord'; Value = 1", "<index>'; Name = 'EnableGpuFirmware'; Type = 'DWord'; Value = 0",
     K("Table.the_script_table_equals_the_json", "Post.post_sets_the_class_key_at_the_discovered_index", "Auto.auto_picks_post_when_the_nvidia_driver_is_bound")),
    ("service-key-moved-to-the-post-phase", "@{ Id = 2; Phase = 'pre';", "@{ Id = 2; Phase = 'post';",
     K("Table.the_script_table_equals_the_json", "Pre.pre_sets_the_service_and_dwm_values_and_creates_the_missing_service_key")),
    ("missing-readback", "    $now = Get-RegDword $Path $Name\n", "    $now = $Value\n",
     K("Readback.a_write_that_does_not_stick_fails_the_run", "Readback.every_write_is_followed_by_a_read_of_the_same_value")),
    ("readback-failure-ignored", "    Say \"FAILED   HKLM\\$Path $Name reads back $nowText, expected $Value\"\n    return $false",
     "    Say \"FAILED   HKLM\\$Path $Name reads back $nowText, expected $Value\"\n    return $true",
     K("Readback.a_write_that_does_not_stick_fails_the_run", "Install.a_failed_class_write_leaves_the_device_disabled")),
    ("accepts-unelevated", "    if (-not (Test-Elevated)) {", "    if ($false) {",
     K("Elevation.unelevated_pre_is_refused_and_nothing_is_touched", "Elevation.unelevated_post_is_refused_and_nothing_is_touched",
       "Elevation.unelevated_auto_and_install_driver_are_refused_and_nothing_is_touched")),
    ("elevation-checked-late",
     "    if (-not (Test-Elevated)) {\n        Show-Refusal 'this needs an elevated PowerShell (Run as administrator). Nothing was changed.'\n        return 2\n    }\n    if ($InstallDriver -and $Phase -ne 'auto') {",
     "    if ($InstallDriver -and $Phase -ne 'auto') {",
     K("Elevation.unelevated_pre_is_refused_and_nothing_is_touched", "Elevation.refusal_happens_before_the_device_list_or_registry_is_read")),
    ("auto-ignores-a-bound-driver", "if ($state.Bound -or $DriverKey) {", "if ($DriverKey) {",
     K("Auto.auto_picks_post_when_the_nvidia_driver_is_bound", "Auto.auto_on_a_bound_driver_asks_for_a_restart_and_sets_all_three")),
    ("basic-display-counts-as-nvidia", "$state.Bound = ($state.Service -eq 'nvlddmkm') -and ($state.DriverKey -ne '')", "$state.Bound = ($state.DriverKey -ne '')",
     K("Auto.a_basic_display_device_is_not_a_bound_nvidia_driver", "Post.post_refuses_when_the_device_is_on_the_basic_display_driver")),
    ("no-restart-notice", "RESTART REQUIRED: restart Windows now (shutdown /r /t 0) so the driver starts with the settings.", "finished.",
     K("Post.post_says_a_restart_is_needed", "Post.post_with_a_given_driver_key_says_a_restart_is_needed_and_skips_discovery")),
    ("always-writes", "    if ($null -ne $was -and $was -eq $Value) {", "    if ($false) {",
     K("Idempotent.the_second_run_writes_nothing_and_ends_in_the_same_state", "Idempotent.the_second_run_says_already_and_still_reads_back")),
    ("driver-key-shape-unchecked", "    return ($Key -match '^\\{4d36e968-e325-11ce-bfc1-08002be10318\\}\\\\[0-9]{4}$')", "    return $true",
     K("Post.post_refuses_a_driver_key_that_walks_out_of_the_class_key", "Post.post_refuses_a_driver_key_of_another_class_or_without_an_index")),
    ("install-does-not-disable-first", "    $rc = Invoke-Pnputil @('/disable-device', $state.InstanceId)", "    $rc = 0",
     K("Install.the_order_is_pre_disable_add_class_value_enable", "Install.the_device_is_disabled_before_the_driver_is_added")),
    ("install-enables-after-a-failed-write", "            return 1\n        }\n        Say 'step 5   enable the device'", "        }\n        Say 'step 5   enable the device'",
     K("Install.a_failed_class_write_leaves_the_device_disabled", "Install.a_failed_class_write_says_how_to_recover")),
    ("install-goes-on-after-a-failed-pre-value",
     "    if (-not (Set-PhaseSettings @('pre') '')) {\n        Say 'FAILED   a pre value did not read back; the device was not touched.'\n        return 1\n    }",
     "    Set-PhaseSettings @('pre') '' | Out-Null",
     K("Install.a_failed_pre_value_does_not_touch_the_device", "Install.a_failed_pre_value_ends_the_install_with_one_and_a_message")),
    ("install-skips-the-pre-values", "    Say 'step 1   settings that can be set before the install'\n    if (-not (Set-PhaseSettings @('pre') '')) {",
     "    Say 'step 1   settings that can be set before the install'\n    if (-not (Set-PhaseSettings @() '')) {",
     K("Install.the_order_is_pre_disable_add_class_value_enable", "Install.a_failed_pre_value_does_not_touch_the_device")),
    ("full-instance-id-printed", "        Say \"device   $($state.Short) bound to nvlddmkm, driver key $Key\"", "        Say \"device   $($state.InstanceId) bound to nvlddmkm, driver key $Key\"",
     K("Output.nothing_sensitive_is_printed", "Output.a_device_is_named_by_vendor_and_device_id_only")),
]
NAMES = [m[0] for m in MUTANTS]


def apply(text, old, new):
    if text.count(old) != 1:
        raise SystemExit("mutant target occurs %d times: %r" % (text.count(old), old[:90]))
    return text.replace(old, new, 1)


def run_tests(directory, tests):
    env = dict(os.environ, KF_WINSETUP_DIR=str(directory), KF_REQUIRE_PWSH="1")
    code = subprocess.run([sys.executable, "-B", "-c", COLLECT] + tests, cwd=str(HERE), env=env, capture_output=True, text=True, timeout=1800)
    lines = [l for l in code.stdout.splitlines() if l.startswith("KF_MUTATION_RESULT=")]
    if not lines:
        raise SystemExit("no result from the test run:\n%s\n%s" % (code.stdout[-2000:], code.stderr[-2000:]))
    return json.loads(lines[-1].split("=", 1)[1])


def run_one(mutant):
    name, old, new, tests = mutant
    tests = sorted(set(tests))
    with tempfile.TemporaryDirectory() as tmp:
        d = Path(tmp) / "windows"
        shutil.copytree(SRC, d)
        (d / "kayfabe-windows-setup.ps1").write_text(apply(PS1, old, new))
        result = run_tests(d, tests)
    killers = result["killers"]
    ok = len(killers) >= 2
    return name, ok, killers, tests


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("only", nargs="?")
    args = ap.parse_args()
    for sub in (m for m in MUTANTS):
        assert len(set(sub[3])) >= 2, "mutant %s names fewer than two killers" % sub[0]
    with tempfile.TemporaryDirectory() as tmp:
        d = Path(tmp) / "windows"
        shutil.copytree(SRC, d)
        every = sorted({t for m in MUTANTS for t in m[3]})
        base = run_tests(d, every)
        if base["killers"]:
            print("the unmutated script is not green: %s" % base["killers"])
            return 1
        print("baseline green: %d tests" % base["tests"])
    chosen = [m for m in MUTANTS if not args.only or args.only in m[0]]
    weak = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for name, ok, killers, tests in pool.map(run_one, chosen):
            print("%s %-52s killed by %d: %s" % ("ok  " if ok else "WEAK", name, len(killers), ", ".join(k.split(".")[-1] for k in killers)))
            if not ok:
                weak.append(name)
    if weak:
        print("WEAK mutants (fewer than two killers): %s" % ", ".join(weak))
        return 1
    print("every mutant has at least two named killers (%d mutants)" % len(chosen))
    return 0


if __name__ == "__main__":
    sys.exit(main())
