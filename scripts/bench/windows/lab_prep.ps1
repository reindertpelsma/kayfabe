# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Run elevated before capture. A failed report is never permission to capture.
param(
    [string]$FootprintsDir='C:\ProgramData\KayfabeLab\footprints',
    [string]$ReportPath='C:\ProgramData\KayfabeLab\lab_prep.json',
    [string]$QuarantineDir='C:\ProgramData\KayfabeLab\startup-quarantine',
    [ValidateRange(1,300)][int]$TimeoutSec=120,
    [string]$FixturePath, [string]$FixtureResultPath
)
. "$PSScriptRoot/app_autostart_common.ps1"
$report=[ordered]@{ schema_version=1; kind='lab_prep'; machine_id=''; created_utc=[DateTime]::UtcNow.ToString('o')
    complete=$false; footprints=@(); services=@(); tasks=@(); removed_run=@(); quarantined_startup=@()
    protected_services=@(); actions=@(); errors=@() }
$writeReport=$false
try {
    Initialize-KfBackend $FixturePath
    if (Test-Path -LiteralPath $ReportPath) { throw 'report-already-exists' }
    $writeReport=$true
    $catalog=Get-KfCatalog
    $report.machine_id=$catalog.machine_id
    $script:KfProtected=Get-KfProtectedNames $catalog.entries
    $report.protected_services=@($script:KfProtected.Keys | Sort-Object)
    $pins=New-Object 'System.Collections.Generic.List[object]'; $ids=@{}
    foreach ($file in @(Get-ChildItem -LiteralPath $FootprintsDir -Filter '*.json' -File | Sort-Object Name)) {
        $f=Read-KfFootprint $file.FullName
        if ($f.machine_id -cne $catalog.machine_id -or $ids.ContainsKey($f.app_id)) { throw 'footprint-machine-or-id' }
        if ($file.Name -cne ($f.app_id + '.json')) { throw 'footprint-filename' }
        $ids[$f.app_id]=$true
        $pins.Add([ordered]@{ app_id=$f.app_id; installer_sha256=$f.installer_sha256; sha256=(Get-KfHash $file.FullName) })
    }
    $report.footprints=@($pins.ToArray())
    # Copy original modes before the provider mutates live service objects.
    $original=$catalog.entries | ConvertTo-Json -Depth 24 | ConvertFrom-Json
    $report.services=@($original.services | Where-Object { -not (Test-KfProtectedEntry 'services' $_) })
    $report.tasks=@($original.tasks | Where-Object { -not (Test-KfProtectedEntry 'tasks' $_) })
    $report.removed_run=@($original.run | Where-Object { -not (Test-KfProtectedEntry 'run' $_) })
    $report.quarantined_startup=@($original.startup | Where-Object { -not (Test-KfProtectedEntry 'startup' $_) })
    $clock=[Diagnostics.Stopwatch]::StartNew()
    Reset-KfLab $catalog $QuarantineDir $clock $TimeoutSec
    $report.complete=$true
    Write-Output 'LAB_PREP_DONE'
} catch {
    $reason=Get-KfFailureReason $_
    $report.errors=@($reason)
    Write-Output "LAB_PREP_FAIL reason=$reason"
} finally {
    $report.actions=@($script:KfActions.ToArray())
    if ($writeReport) { Write-KfJson $report $ReportPath }
    Save-KfFixture $FixtureResultPath
}
if ($report.complete) { exit 0 }; exit 1
