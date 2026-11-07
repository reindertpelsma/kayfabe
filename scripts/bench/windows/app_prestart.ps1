# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param(
    [Parameter(Mandatory=$true)][Alias('-app')][string]$App,
    [ValidateSet('Start','Reset')][string]$Action='Start',
    [string]$FootprintsDir='C:\ProgramData\KayfabeLab\footprints',
    [string]$LabReportPath='C:\ProgramData\KayfabeLab\lab_prep.json',
    [string]$QuarantineDir='C:\ProgramData\KayfabeLab\startup-quarantine',
    [string[]]$TaskIds=@(), [string]$TaskIdsJson,
    [ValidateRange(1,120)][int]$TimeoutSec=30,
    [string]$FixturePath, [string]$FixtureResultPath
)
. "$PSScriptRoot/app_autostart_common.ps1"
try {
    Initialize-KfBackend $FixturePath; Assert-KfId $App
    if ($TaskIdsJson) { $TaskIds=@($TaskIdsJson | ConvertFrom-Json) }
    if ($TaskIds.Count -gt 32) { throw 'too-many-tasks' }
    $catalog=Get-KfCatalog; $script:KfProtected=Get-KfProtectedNames $catalog.entries
    $report=Read-KfJson $LabReportPath
    Assert-KfFields $report @('schema_version','kind','machine_id','created_utc','complete','footprints','services','tasks','removed_run','quarantined_startup','protected_services','actions','errors')
    Assert-KfVersion $report 'lab_prep'
    Assert-KfEntries ([pscustomobject]@{ services=$report.services; tasks=$report.tasks
        run=$report.removed_run; startup=$report.quarantined_startup })
    if ($report.complete -isnot [bool] -or -not $report.complete -or $report.machine_id -cne $catalog.machine_id) { throw 'lab-report-not-complete' }
    $current=Get-KfServiceMap $catalog.entries.services; $recorded=Get-KfServiceMap $report.services
    foreach ($name in $recorded.Keys) {
        if (-not $current.ContainsKey($name)) { throw 'service-missing' }
        Assert-KfServiceIdentity $current[$name] $recorded[$name]
    }
    foreach ($s in @($catalog.entries.services)) {
        if (-not (Test-KfProtectedEntry 'services' $s) -and -not $recorded.ContainsKey($s.name)) { throw 'new-unrecorded-service' }
    }
    $foot=Read-KfFootprint (Join-Path $FootprintsDir ($App + '.json'))
    $pin=@($report.footprints | Where-Object { $_.app_id -ceq $App })
    if ($foot.app_id -cne $App -or $pin.Count -ne 1 -or $foot.machine_id -cne $catalog.machine_id -or
        $pin[0].installer_sha256 -cne $foot.installer_sha256 -or
        $pin[0].sha256 -cne (Get-KfHash (Join-Path $FootprintsDir ($App + '.json')))) { throw 'footprint-pin-mismatch' }
    $selected=@{}
    function Select-KfService([string]$Name) {
        if ($script:KfProtected.ContainsKey($Name)) {
            if ($current.ContainsKey($Name) -and $current[$Name].status -ne 'Running') { throw 'protected-dependency-not-running' }
            return
        }
        if ($selected.ContainsKey($Name)) { return }
        if (-not $recorded.ContainsKey($Name) -or $script:KfAlwaysOff -contains $Name) { throw 'forbidden-or-unknown-dependency' }
        $selected[$Name]=$recorded[$Name]
        foreach ($d in @($recorded[$Name].dependencies)) { Select-KfService $d }
    }
    if ($Action -eq 'Start') {
        foreach ($s in @($foot.entries.services)) {
            if (-not (Test-KfProtectedEntry 'services' $s)) {
                if (-not $current.ContainsKey($s.name)) { throw 'app-service-missing' }
                Assert-KfServiceIdentity $current[$s.name] $s
                Select-KfService $s.name
            }
        }
    }
    $tasks=@{}; foreach ($t in @($foot.entries.tasks)) { $tasks[(Get-KfEntryKey 'tasks' $t)]=$t }
    $chosenTasks=@(); $taskCurrent=@{}
    foreach ($t in @($catalog.entries.tasks)) { $taskCurrent[(Get-KfEntryKey 'tasks' $t)]=$t }
    foreach ($id in $TaskIds) {
        $key=$id.ToLowerInvariant()
        if (-not $tasks.ContainsKey($key) -or -not $taskCurrent.ContainsKey($key) -or
            (Test-KfProtectedEntry 'tasks' $taskCurrent[$key])) { throw 'task-not-owned' }
        $null=Get-KfManualTaskXml $tasks[$key].xml
        $chosenTasks+=,$key
    }
    $clock=[Diagnostics.Stopwatch]::StartNew()
    # A new app never inherits a prior app's enabled services, even without cleanup.
    Reset-KfLab $catalog $QuarantineDir $clock $TimeoutSec
    if ($Action -eq 'Start') {
        $order=@(Get-KfOrder @($selected.Values))
        foreach ($name in $order) {
            $old=$selected[$name]
            if ($old.start_type -eq 'Disabled') { throw 'original-service-disabled' }
            $current[$name].delayed_auto=$old.delayed_auto
            Set-KfServiceMode $current[$name] $old.start_type
            Start-KfService $current[$name] $clock $TimeoutSec
        }
        foreach ($key in $chosenTasks) { Import-KfManualTask $taskCurrent[$key] $tasks[$key] }
        Assert-KfQuiet (Get-KfCatalog) @($selected.Keys) $chosenTasks
    }
    Write-Output "APP_PRESTART_DONE app=$App action=$Action"
    exit 0
} catch {
    $reason=Get-KfFailureReason $_
    Write-Output "APP_PRESTART_FAIL app=$App action=$Action reason=$reason"
    exit 1
} finally { Save-KfFixture $FixtureResultPath }
