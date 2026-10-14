# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# shorts_probe.ps1 [-Url https://www.youtube.com/shorts] [-Seconds 90] [-Out C:\kf\out\shorts_probe.txt]
# Native-baseline reference for the kayfabe comparison: play a YouTube Short in the inbox Edge (policies HideFirstRunExperience +
# AutoplayAllowed set machine-wide first, as the TDR-hunt runner does) and log every 2 s the msedge processes and every non-zero GPU Engine
# counter row (per pid / engine type). Screenshots and consent clicks are taken from the controller (native_windows_apps.py shot / click).
param([string]$Url = 'https://www.youtube.com/shorts', [int]$Seconds = 90, [string]$Out = 'C:\kf\out\shorts_probe.txt')
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path (Split-Path $Out) | Out-Null
'' | Set-Content $Out
$pk = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge'
New-Item -Path $pk -Force | Out-Null
foreach ($kv in @(@('HideFirstRunExperience', 1), @('AutoplayAllowed', 1), @('SyncDisabled', 1), @('BrowserSignin', 0), @('DefaultBrowserSettingEnabled', 0), @('PromotionalTabsEnabled', 0), @('StartupBoostEnabled', 0))) {
    New-ItemProperty -Path $pk -Name $kv[0] -Value $kv[1] -PropertyType DWord -Force | Out-Null
}
$edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
$ver = (Get-Item $edge).VersionInfo.ProductVersion
"edge_version=$ver url=$Url" | Add-Content $Out
$prof = 'C:\kf\edge\shorts'; Remove-Item -Recurse -Force $prof -ErrorAction SilentlyContinue
$null = Start-Process $edge -ArgumentList @("--user-data-dir=$prof", '--no-first-run', '--no-default-browser-check', '--window-size=1024,700', '--window-position=0,0', $Url) -PassThru
$t0 = Get-Date
while (((Get-Date) - $t0).TotalSeconds -lt $Seconds) {
    Start-Sleep -Seconds 2
    $el = [int]((Get-Date) - $t0).TotalSeconds
    $pids = @{}
    foreach ($p in @(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue)) {
        $cl = [string]$p.CommandLine
        $ty = 'browser'; if ($cl -match '--type=([a-z\-]+)') { $ty = $Matches[1] }; if ($cl -match '--utility-sub-type=(\S+)') { $ty += ':' + $Matches[1] }
        $pids[[int]$p.ProcessId] = $ty
    }
    try { $cs = (Get-Counter -Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples } catch { continue }
    foreach ($c in $cs) {
        if ($c.CookedValue -le 0) { continue }
        if ($c.InstanceName -match '^pid_(\d+)_.*_engtype_(.+)$') {
            $q = [int]$Matches[1]
            if ($pids.ContainsKey($q)) { "t=$el eng pid=$q (msedge/$($pids[$q])) engtype=$($Matches[2]) util=$([math]::Round($c.CookedValue,2))" | Add-Content $Out }
        }
    }
}
'DONE' | Add-Content $Out
