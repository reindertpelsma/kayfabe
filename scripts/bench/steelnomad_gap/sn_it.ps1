# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_it.ps1 -Script C:\kf\sn\sn_ui.ps1 -Args "-Action run" [-Wait 60]: run a script as the logged-in user in the interactive
# session (one-shot scheduled task /it, pattern of appmatrix/native/run_overlay_it.ps1), delete the task, print its log.
param([string]$Script = 'C:\kf\sn\sn_ui.ps1', [string]$ScriptArgs = '-Action dump', [int]$Wait = 60, [string]$Log = 'C:\kf\sn\ui.txt')
New-Item -ItemType Directory -Force C:\kf\sn | Out-Null
Remove-Item $Log -ErrorAction SilentlyContinue
$cmd = 'C:\kf\sn\it_run.cmd'
Set-Content -Path $cmd -Encoding ASCII -Value ("@echo off`r`npowershell.exe -NoProfile -ExecutionPolicy Bypass -File $Script $ScriptArgs`r`n")
$user = (Get-CimInstance Win32_ComputerSystem).UserName
if (-not $user) { $user = "$env:COMPUTERNAME\vast" }
schtasks /delete /tn sn_it /f 2>&1 | Out-Null
schtasks /create /tn sn_it /tr $cmd /sc once /st 00:00 /it /ru $user /rl highest /f 2>&1 | Out-Null
schtasks /run /tn sn_it 2>&1 | Out-Null
$end = (Get-Date).AddSeconds($Wait)
while ((Get-Date) -lt $end -and -not (Test-Path $Log)) { Start-Sleep -Milliseconds 300 }
Start-Sleep -Milliseconds 500
schtasks /delete /tn sn_it /f 2>&1 | Out-Null
"IT user=$user"
if (Test-Path $Log) { Get-Content $Log } else { 'NO LOG' }
