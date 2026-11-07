# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
param(
    [Parameter(Mandatory=$true,Position=0)][ValidateSet('snapshot','diff')][string]$Mode,
    [Parameter(Mandatory=$true)][string]$OutputPath,
    [string]$BeforePath, [string]$AfterPath,
    [Alias('-app')][string]$App, [string]$InstallerSha256,
    [string]$FixturePath, [string]$FixtureResultPath
)
. "$PSScriptRoot/app_autostart_common.ps1"
try {
    Initialize-KfBackend $FixturePath
    if (Test-Path -LiteralPath $OutputPath) { throw 'output-exists' }
    if ($Mode -eq 'snapshot') {
        $catalog=Get-KfCatalog
        $out=[ordered]@{ schema_version=1; kind='autostart_snapshot'; machine_id=$catalog.machine_id
            created_utc=[DateTime]::UtcNow.ToString('o'); entries=$catalog.entries }
    } else {
        Assert-KfId $App
        if ($InstallerSha256 -cnotmatch '^[0-9a-f]{64}$') { throw 'installer-hash-required' }
        $before=Read-KfJson $BeforePath; $after=Read-KfJson $AfterPath
        foreach ($snap in @($before,$after)) {
            Assert-KfFields $snap @('schema_version','kind','machine_id','created_utc','entries')
            Assert-KfVersion $snap 'autostart_snapshot'; Assert-KfEntries $snap.entries
        }
        if ($before.machine_id -cne $after.machine_id) { throw 'machine-mismatch' }
        $new=[ordered]@{}; $changed=New-Object 'System.Collections.Generic.List[object]'
        foreach ($kind in @('services','tasks','run','startup')) {
            $old=@{}; foreach ($r in @($before.entries.$kind)) { $old[(Get-KfEntryKey $kind $r)]=$r }
            $added=New-Object 'System.Collections.Generic.List[object]'
            foreach ($r in @($after.entries.$kind)) {
                $key=Get-KfEntryKey $kind $r
                if (-not $old.ContainsKey($key)) { $added.Add($r) }
                elseif (($old[$key] | ConvertTo-Json -Depth 12 -Compress) -cne ($r | ConvertTo-Json -Depth 12 -Compress)) {
                    $changed.Add([ordered]@{ kind=$kind; key=$key; before=$old[$key]; after=$r })
                }
            }
            $new[$kind]=@($added.ToArray())
        }
        $out=[ordered]@{ schema_version=1; kind='app_footprint'; machine_id=$before.machine_id
            created_utc=[DateTime]::UtcNow.ToString('o'); app_id=$App; installer_sha256=$InstallerSha256
            before_sha256=(Get-KfHash $BeforePath); after_sha256=(Get-KfHash $AfterPath)
            entries=$new; changed_existing=@($changed.ToArray()) }
    }
    Write-KfJson $out $OutputPath
    Write-Output "APP_FOOTPRINT_DONE mode=$Mode"
    exit 0
} catch {
    Write-Output "APP_FOOTPRINT_FAIL mode=$Mode"
    exit 1
} finally { Save-KfFixture $FixtureResultPath }
