# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param(
    [Parameter(Mandatory=$true)][Alias('-app')][string]$App,
    [ValidateSet('Start','Reset')][string]$Action='Start',
    [string]$FootprintsDir='C:\ProgramData\KayfabeLab\footprints',
    [string]$LabReportPath='C:\ProgramData\KayfabeLab\lab_prep.json',
    [ValidateRange(1,120)][int]$TimeoutSec=30,
    [string]$FixturePath, [string]$FixtureResultPath
)
. "$PSScriptRoot/app_autostart_common.ps1"
try {
    Initialize-KfBackend $FixturePath; Assert-KfId $App
    $catalog=Get-KfCatalog; $script:KfProtected=Get-KfProtectedNames $catalog.entries
    $report=Read-KfJson $LabReportPath
    Assert-KfFields $report @('schema_version','kind','machine_id','created_utc','complete','policy','footprints','services','tasks','removed_run','quarantined_startup','protected_services','actions','errors')
    Assert-KfVersion $report 'lab_prep' 2
    Initialize-KfPolicy $report.policy $catalog.entries
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
        if (-not $recorded.ContainsKey($Name) -or $script:KfDisablePolicy.services -contains $Name) { throw 'forbidden-or-unknown-dependency' }
        $selected[$Name]=$recorded[$Name]
        foreach ($d in @($recorded[$Name].dependencies)) { Select-KfService $d }
    }
    foreach ($s in @($foot.entries.services)) {
            if (-not (Test-KfProtectedEntry 'services' $s)) {
                if (-not $current.ContainsKey($s.name)) { throw 'app-service-missing' }
                Assert-KfServiceIdentity $current[$s.name] $s
                Select-KfService $s.name
            }
        }
    $clock=[Diagnostics.Stopwatch]::StartNew()
    $order=@(Get-KfOrder @($selected.Values))
    if ($Action -eq 'Start') {
        foreach ($name in $order) {
            if ($current[$name].start_type -ne 'Manual') { throw 'app-service-not-demand-start' }
            Start-KfService $current[$name] $clock $TimeoutSec
        }
    } else {
        for ($i=$order.Count-1; $i -ge 0; $i--) {
            $row=$current[$order[$i]]
            if ($row.status -ne 'Stopped') { Stop-KfService $row $clock $TimeoutSec }
        }
    }
    Write-Output "APP_PRESTART_DONE app=$App action=$Action"
    exit 0
} catch {
    $reason=Get-KfFailureReason $_
    Write-Output "APP_PRESTART_FAIL app=$App action=$Action reason=$reason"
    exit 1
} finally { Save-KfFixture $FixtureResultPath }
