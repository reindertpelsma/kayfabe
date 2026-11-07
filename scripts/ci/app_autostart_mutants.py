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
case('primitive-never-disables-ordinary-service',C,
     "if ($Mode -eq 'Disabled' -and -not (Test-KfDisabledService $Row)) { throw 'non-updater-disable-refused' }",'$null=$Mode',
     'test_primitive_rejects_disabling_app_service','test_primitive_rejects_disabling_manual_dependency')
case('ordinary-services-demand-start',C,"Set-KfServiceMode $s 'Manual'","Set-KfServiceMode $s 'Disabled'",
     'test_app_automatic_services_become_manual','test_delayed_automatic_service_is_demand_start')
case('ordinary-tasks-preserved',C,"-and (Test-KfDisabledTask $t) -and",'-and $true -and',
     'test_ordinary_scheduled_tasks_are_untouched','test_startup_entries_are_removed_without_disabling_ordinary_tasks')
case('record-original-start-types','lab_prep.ps1','$catalog.entries | ConvertTo-Json -Depth 24 | ConvertFrom-Json','$catalog.entries',
     'test_previous_start_types_are_recorded','test_delayed_automatic_service_is_demand_start')
case('protected-service-floor',C,'if ($s.name -like $pattern)','if ($false)',
     'test_protected_services_are_never_changed','test_second_protected_dependency_and_driver')
case('protected-dependency-closure',C,'if (-not $protected.ContainsKey($dep))','if ($false)',
     'test_protection_follows_live_dependencies','test_second_protected_dependency_and_driver')
driver=(WIN/C).read_text()
for old in ("$s.service_type -match 'Driver'","$Row.service_type -match 'Driver'"):
    assert driver.count(old)==1
    driver=driver.replace(old,'$false')
CASES.append(('driver-type-protection',C,driver,('test_driver_guard_is_not_just_name_matching','test_second_protected_dependency_and_driver')))
case('policy-protected-entries',C,"throw 'protected-policy-service'","$null=$row",
     'test_policy_cannot_disable_protected_service','test_second_policy_protected_service_rejected')
case('post-write-audit',C,'Assert-KfQuiet (Get-KfCatalog)','$null=$Catalog',
     'test_silent_service_write_failure_is_audited','test_silent_update_task_failure_is_audited')
case('original-report-immutable','lab_prep.ps1',"if (Test-Path -LiteralPath $ReportPath) { throw 'report-already-exists' }",'$null=$ReportPath',
     'test_prep_cannot_replace_original_modes','test_prep_preserves_failed_report_for_inspection')
case('strict-fields',C,'if (@($Required + $Optional) -notcontains $n)','if ($false)',
     'test_unknown_footprint_field_rejected','test_second_unknown_footprint_field_rejected')
case('strict-sha256',C,"if ($h -cnotmatch '^[0-9a-f]{64}$')",'if ($false)',
     'test_corrupt_installer_hash_rejected','test_corrupt_snapshot_hash_rejected')
case('loaded-profiles',C,'if (@($script:KfFixture.unloaded_profiles).Count -ne 0)','if ($false)',
     'test_unloaded_profile_refuses_before_mutation','test_second_profile_refusal_and_failed_report')
case('new-entry-ownership','app_footprint.ps1','if (-not $old.ContainsKey($key))','if ($true)',
     'test_diff_does_not_claim_existing_entries','test_changed_existing_service_is_audit_only')
case('same-machine-snapshots','app_footprint.ps1','if ($before.machine_id -cne $after.machine_id)','if ($false)',
     'test_diff_rejects_cross_machine_snapshots','test_second_machine_mismatch_diff_rejected')
case('pinned-footprint','app_prestart.ps1',"$pin[0].sha256 -cne (Get-KfHash (Join-Path $FootprintsDir ($App + '.json')))",'$false',
     'test_changed_footprint_hash_is_refused','test_second_report_hash_drift_rejected')
case('complete-report-machine','app_prestart.ps1',"if ($report.complete -isnot [bool] -or -not $report.complete -or $report.machine_id -cne $catalog.machine_id)",'if ($false)',
     'test_failed_lab_report_is_refused','test_report_machine_mismatch_rejected')
case('service-identity',C,'if ($Current.$k -cne $Recorded.$k)','if ($false)',
     'test_changed_service_identity_refuses','test_service_account_drift_rejected')
case('no-updater-prestart','app_prestart.ps1','$script:KfDisablePolicy.services -contains $Name','$false',
     'test_updater_dependency_cannot_be_prestarted','test_indexer_dependency_cannot_be_prestarted')
case('bounded-service-wait',C,"Wait-KfService $Row 'Running' $Clock $TimeoutSec",'# omitted wait',
     'test_start_wait_failure_is_clear','test_dependency_wait_failure_is_clear')
case('prestart-never-restores-automatic','app_prestart.ps1','Start-KfService $current[$name] $clock $TimeoutSec',
     "Set-KfServiceMode $current[$name] 'Automatic'; Start-KfService $current[$name] $clock $TimeoutSec",
     'test_prestart_is_only_a_manual_service_escape_hatch','test_prestart_does_not_restore_automatic_start')
case('host-zero-exit','appmatrix/prestart.py','result.exitcode == 0 and ','',
     'test_nonzero_with_success_marker_never_launches','test_cleanup_nonzero_with_marker_forces_fresh_guest')
case('host-exact-marker','appmatrix/prestart.py','marker in result.text().splitlines()','True',
     'test_missing_marker_never_launches_workload','test_wrong_app_marker_never_launches')
case('host-failed-cleanup','appmatrix/winapps.py','if not reset_ok:','if False:',
     'test_cleanup_failure_forces_fresh_guest','test_cleanup_missing_marker_forces_fresh_guest')
case('host-unknown-fields','appmatrix/prestart.py','set(raw) - {"timeout_s"}','False',
     'test_unknown_prestart_field_is_rejected','test_second_unknown_field_rejected')
case('host-bounded-timeout','appmatrix/prestart.py','type(seconds) is not int or not 1 <= seconds <= 120','False',
     'test_timeout_bounds_and_boolean_rejected','test_negative_and_string_timeout_rejected')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--json', type=Path)
    parser.add_argument('--only', action='append', choices=[c[0] for c in CASES],
                        help='local focused rerun; CI omits this and exercises every mutant')
    args = parser.parse_args()
    cases=[c for c in CASES if not args.only or c[0] in args.only]
    if args.only:
        print('SCOPED run: '+str(len(cases))+'/'+str(len(CASES))+' mutants',flush=True)
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
                                   *['AutostartFixtures.' + t for t in sorted({t for c in cases if not c[1].startswith('appmatrix/') for t in c[3]})]],
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
        for name, filename, changed, tests in cases:
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
