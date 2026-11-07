#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline rule mutants: two independently named fixture tests per mutation."""
import argparse
import ast
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
WIN = ROOT / 'scripts/bench/windows'
CASES = []


def case(name, filename, old, new, *tests):
    source = (WIN / filename).read_text()
    assert source.count(old) == 1, (name, source.count(old))
    changed=source.replace(old,new)
    if filename.endswith('.py'):
        ast.parse(changed)
    CASES.append((name, filename, changed, tests))


C = 'app_autostart_common.ps1'
driver_source=(WIN / C).read_text()
for old in ("$s.service_type -match 'Driver'", "$Row.service_type -match 'Driver'"):
    assert driver_source.count(old)==1
    driver_source=driver_source.replace(old,'$false')
CASES.append(('all-driver-types-protected',C,driver_source,
              ('test_driver_guard_is_not_just_name_matching','test_second_protected_dependency_and_driver')))
case('new-entry-ownership', 'app_footprint.ps1', 'if (-not $old.ContainsKey($key))', 'if ($true)',
     'test_diff_does_not_claim_existing_entries', 'test_changed_existing_service_is_audit_only')
case('same-machine-snapshots', 'app_footprint.ps1', 'if ($before.machine_id -cne $after.machine_id)', 'if ($false)',
     'test_diff_rejects_cross_machine_snapshots', 'test_second_machine_mismatch_diff_rejected')
case('original-service-mode-copy', 'lab_prep.ps1', '$catalog.entries | ConvertTo-Json -Depth 24 | ConvertFrom-Json', '$catalog.entries',
     'test_lab_report_records_previous_types_and_hashes', 'test_start_selected_app_and_its_dependency_only')
case('post-mutation-audit', C, 'Assert-KfQuiet (Get-KfCatalog)', '$null=$Catalog',
     'test_silent_service_disable_failure_is_audited', 'test_silent_task_disable_failure_is_audited')
case('preserve-original-preparation-report', 'lab_prep.ps1',
     "if (Test-Path -LiteralPath $ReportPath) { throw 'report-already-exists' }", '$null=$ReportPath',
     'test_prep_cannot_replace_original_modes', 'test_prep_preserves_failed_report_for_inspection')
case('schema-version', C, "if (($Object.schema_version -isnot [int] -and $Object.schema_version -isnot [long]) -or\n        $Object.schema_version -ne 1 -or $Object.kind -cne $Kind)", 'if ($false)',
     'test_bad_footprint_refuses_before_mutation', 'test_boolean_schema_rejected')
case('unknown-fields', C, "if (@($Required + $Optional) -notcontains $n)", 'if ($false)',
     'test_unknown_footprint_field_rejected', 'test_second_unknown_footprint_field_rejected')
case('installer-and-snapshot-hashes', C, "if ($h -cnotmatch '^[0-9a-f]{64}$')", 'if ($false)',
     'test_corrupt_installer_hash_rejected', 'test_corrupt_snapshot_hash_rejected')
case('protected-service-floor', C, 'if ($s.name -like $pattern)', 'if ($false)',
     'test_protected_services_are_never_changed', 'test_second_protected_dependency_and_driver')
case('protected-dependency-closure', C, 'if (-not $protected.ContainsKey($dep))', 'if ($false)',
     'test_protection_follows_live_dependencies', 'test_second_protected_dependency_and_driver')
case('protected-task-policy', C, "if ($Kind -eq 'tasks') {\n        if", "if ($Kind -eq 'tasks') { return $false\n        if",
     'test_protected_tasks_untouched', 'test_protected_task_command_is_kept')
case('loaded-profile-required', C, 'if (@($script:KfFixture.unloaded_profiles).Count -ne 0)', 'if ($false)',
     'test_unloaded_profile_refuses_before_mutation', 'test_second_profile_refusal_and_failed_report')
case('global-service-disable', C, "Set-KfServiceMode $s 'Disabled'", '$null=$s',
     'test_lab_disables_services_missing_from_footprint', 'test_unowned_updater_and_task_cannot_escape_sweep')
case('global-task-disable', C, 'Disable-KfTask $t }', '# omitted task disable\n        }',
     'test_lab_disables_tasks_and_removes_all_startup', 'test_unowned_updater_and_task_cannot_escape_sweep')
case('global-run-removal', C, 'Remove-KfRun $r }', '# omitted run removal\n        }',
     'test_lab_disables_tasks_and_removes_all_startup', 'test_unowned_updater_and_task_cannot_escape_sweep')
case('global-startup-quarantine', C, 'Remove-KfStartup $s $QuarantineDir }', '# omitted quarantine\n        }',
     'test_lab_disables_tasks_and_removes_all_startup', 'test_unowned_updater_and_task_cannot_escape_sweep')
case('prestart-footprint-binding', 'app_prestart.ps1',
     '$pin[0].sha256 -cne (Get-KfHash (Join-Path $FootprintsDir ($App + \'.json\')))', '$false',
     'test_changed_footprint_hash_is_refused', 'test_second_report_hash_drift_rejected')
case('complete-report-machine', 'app_prestart.ps1',
     "if ($report.complete -isnot [bool] -or -not $report.complete -or $report.machine_id -cne $catalog.machine_id)", 'if ($false)',
     'test_failed_lab_report_is_refused', 'test_report_machine_mismatch_rejected')
case('service-identity-binding', C, "if ($Current.$k -cne $Recorded.$k)", 'if ($false)',
     'test_changed_service_identity_refuses', 'test_service_account_drift_rejected')
case('always-off-dependencies', 'app_prestart.ps1', '$script:KfAlwaysOff -contains $Name', '$false',
     'test_always_off_dependency_is_refused', 'test_search_dependency_stays_off')
case('service-bounded-wait', C, "Wait-KfService $Row 'Running' $Clock $TimeoutSec", '# omitted wait',
     'test_start_wait_failure_is_clear', 'test_dependency_wait_failure_is_clear')
case('task-trigger-removal', C, '$triggers.RemoveAll()', '$null=$triggers',
     'test_explicit_task_import_has_no_triggers', 'test_task_json_explicit_import_has_no_boot_trigger')
case('task-credentials', C, "$logon.InnerText -notin @('ServiceAccount','InteractiveToken','S4U')", '$false',
     'test_task_credentials_are_not_recreated', 'test_interactive_password_task_is_not_recreated')
case('per-app-reset', 'app_prestart.ps1', 'Reset-KfLab $catalog $QuarantineDir $clock $TimeoutSec', '# omitted reset',
     'test_reset_removes_previous_apps_service_state', 'test_app_b_never_inherits_app_a_background')
case('host-zero-exit-required', 'appmatrix/prestart.py', 'result.exitcode == 0 and ', '',
     'test_nonzero_with_success_marker_never_launches', 'test_cleanup_nonzero_with_marker_forces_fresh_guest')
case('host-exact-marker-required', 'appmatrix/prestart.py', 'marker in result.text().splitlines()', 'True',
     'test_missing_marker_never_launches_workload', 'test_wrong_app_marker_never_launches')
case('host-cleanup-failure-restarts', 'appmatrix/winapps.py', 'if not reset_ok:', 'if False:',
     'test_cleanup_failure_forces_fresh_guest', 'test_cleanup_missing_marker_forces_fresh_guest')
case('host-unknown-prestart-fields', 'appmatrix/prestart.py', 'set(raw) - {"tasks", "timeout_s"}', 'False',
     'test_unknown_prestart_field_is_rejected', 'test_second_unknown_field_rejected')
case('host-bounded-timeouts', 'appmatrix/prestart.py', 'type(seconds) is not int or not 1 <= seconds <= 120', 'False',
     'test_timeout_bounds_and_boolean_rejected', 'test_negative_and_string_timeout_rejected')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--json', type=Path)
    args = parser.parse_args()
    pw = os.environ.get('KF_PWSH') or shutil.which('pwsh') or '/root/am/pwsh/pwsh'
    if not Path(pw).is_file():
        parser.error('PowerShell is required; skipping mutants would give no evidence')
    table = []
    with tempfile.TemporaryDirectory(prefix='y1-autostart-mutants-') as tmp:
        tree = Path(tmp)
        env={**os.environ, 'KF_PWSH':pw, 'HOME':tmp, 'XDG_CONFIG_HOME':tmp+'/config',
             'XDG_CACHE_HOME':tmp+'/cache', 'DOTNET_CLI_HOME':tmp,
             'POWERSHELL_TELEMETRY_OPTOUT':'1', 'DOTNET_SKIP_FIRST_TIME_EXPERIENCE':'1'}
        parse=tree/'parse.ps1'
        parse.write_text('param([string]$Path)\n$t=$null; $e=$null\n[System.Management.Automation.Language.Parser]::ParseFile($Path,[ref]$t,[ref]$e) | Out-Null\nif ($e.Count) { exit 1 }\n')
        win = tree / 'scripts/bench/windows'; win.mkdir(parents=True)
        ci = tree / 'scripts/ci'; ci.mkdir(parents=True)
        for file in WIN.glob('app_*.ps1'):
            shutil.copy2(file, win / file.name)
        shutil.copy2(WIN / 'lab_prep.ps1', win / 'lab_prep.ps1')
        shutil.copytree(WIN / 'appmatrix', win / 'appmatrix', ignore=shutil.ignore_patterns('__pycache__'))
        test = ci / 'test_app_autostart.py'
        shutil.copy2(ROOT / 'scripts/ci/test_app_autostart.py', test)
        baseline = subprocess.run([sys.executable, '-B', str(test),
                                   *['AutostartFixtures.' + t for t in sorted({t for c in CASES if not c[1].startswith('appmatrix/') for t in c[3]})]],
                                  text=True, capture_output=True, timeout=1200,
                                  env=env)
        if baseline.returncode:
            print('FAIL unmutated baseline\n' + baseline.stdout + baseline.stderr)
            return 2
        print('PASS unmutated fixture baseline', flush=True)
        host_test=win / 'appmatrix/tests/test_prestart.py'
        host_baseline=subprocess.run([sys.executable,'-B',str(host_test)],text=True,capture_output=True,timeout=60,env=env)
        if host_baseline.returncode:
            print('FAIL unmutated host baseline\n' + host_baseline.stdout + host_baseline.stderr)
            return 2
        for name, filename, changed, tests in CASES:
            target = win / filename; original = target.read_text()
            target.write_text(changed)
            try:
                if filename.endswith('.ps1'):
                    syntax=subprocess.run([pw,'-NoProfile','-NonInteractive','-File',str(parse),str(target)],
                                          text=True,capture_output=True,env=env,timeout=30)
                    if syntax.returncode:
                        raise RuntimeError('invalid mutant syntax: '+name)
                host=filename.startswith('appmatrix/')
                run = subprocess.run([sys.executable, '-B', str(host_test if host else test), '-v',
                                      *[('Prestart.' if host else 'AutostartFixtures.') + t for t in tests]],
                                     text=True, capture_output=True, timeout=400,
                                     env=env)
            finally:
                target.write_text(original)
            killers = re.findall(r'^(?:FAIL|ERROR): (test_\w+)', run.stdout + run.stderr, re.M)
            killed = run.returncode != 0 and len(set(killers)) >= 2
            table.append(dict(mutant=name, killed=killed, killers=sorted(set(killers))))
            print(f'{"PASS" if killed else "WEAK"} {name}: {len(set(killers))} killers', flush=True)
            if not killed:
                print((run.stdout + run.stderr)[-2500:])
    if args.json:
        args.json.write_text(json.dumps(table, indent=2) + '\n')
    return 0 if all(row['killed'] for row in table) else 1


if __name__ == '__main__':
    sys.exit(main())
