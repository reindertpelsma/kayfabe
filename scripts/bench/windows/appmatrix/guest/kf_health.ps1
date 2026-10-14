# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_health.ps1 [-Since ISO8601] -- one `KFHEALTH {json}` line the driver reads between apps: boot time (a change
# = the guest rebooted = bugcheck), uptime, the NVIDIA display device's problem code (43 = failed driver), a
# 15 s-bounded nvidia-smi probe, the interactive user, and TDR-ish event counts since -Since.
param([string]$Since = '')
$ErrorActionPreference = 'Continue'
. "$PSScriptRoot\kf_common.ps1"
$h = @{ utc = (Get-Date).ToUniversalTime().ToString('o'); boot_utc = Get-BootTimeUtc }
$h.uptime_s = [int]((Get-Date) - (Get-CimInstance Win32_OperatingSystem).LastBootUpTime).TotalSeconds
$d = @(Get-PnpDevice -Class Display -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName -match 'NVIDIA' })
$h.display = @($d | ForEach-Object { @{ name = $_.FriendlyName; status = [string]$_.Status; problem = [string]$_.Problem } })
$smi = Get-SmiPath
$h.smi_ok = $false
if ($smi) {
    $j = Start-Job -ScriptBlock { param($s) & $s --query-gpu=name,utilization.gpu,memory.used --format=csv,noheader 2>&1 } -ArgumentList $smi
    if (Wait-Job $j -Timeout 15) { $h.smi = ((Receive-Job $j) | Select-Object -First 1) -as [string]; $h.smi_ok = ($h.smi -match 'NVIDIA') } else { $h.smi = 'TIMEOUT'; Stop-Job $j }
    Remove-Job $j -Force -ErrorAction SilentlyContinue
}
$h.user = (Get-CimInstance Win32_ComputerSystem).UserName
$h.explorer = [bool](Get-Process explorer -ErrorAction SilentlyContinue)
if ($Since) {
    try { $s = [datetime]::Parse($Since).ToUniversalTime(); $ev = Get-GuestEventSummary -Since $s.ToLocalTime(); $ev.Remove('lines'); $h.events = $ev } catch { $h.events_error = "$_" }
}
$h.lingering = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ($_.Path -like 'C:\kfapps\*' -or $_.Path -like '?:\py\*') } | ForEach-Object { $_.Name })
Write-Output ('KFHEALTH ' + (ConvertTo-Json -InputObject $h -Depth 5 -Compress))
exit 0
