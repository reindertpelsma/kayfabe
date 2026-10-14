# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_launch.ps1 -Id APP [-Mode service|interactive] -- start the supervisor for one app DETACHED (a one-shot
# scheduled task), so the QGA guest-exec that calls this returns at once and a hung or TDR-stalled app
# cannot hold the agent. `service` = SYSTEM in session 0 (compute, headless D3D/DXGI, CLI tools);
# `interactive` = the signed-in console user's desktop session (OpenGL/Vulkan windows, Edge).
# Output: `KFLAUNCH ok mode=... user=...` or `KFLAUNCH fail <why>` (exit 3 = no interactive user).
param([Parameter(Mandatory = $true)][string]$Id, [string]$Mode = 'service')
$ErrorActionPreference = 'Stop'
$task = "kfapp_$Id"
try { Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue } catch { }
$act = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ('-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\kf\kf_run_app.ps1 -Id ' + $Id)
$user = 'SYSTEM'
if ($Mode -eq 'interactive') {
    $user = (Get-CimInstance Win32_ComputerSystem).UserName
    if (-not $user) { Write-Output 'KFLAUNCH fail no-interactive-user'; exit 3 }
    $pr = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
} else {
    $pr = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
}
$set = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Hours 4) -StartWhenAvailable
try {
    Register-ScheduledTask -TaskName $task -Action $act -Principal $pr -Settings $set -Force | Out-Null
    Start-ScheduledTask -TaskName $task
} catch { Write-Output "KFLAUNCH fail $_"; exit 4 }
Write-Output "KFLAUNCH ok mode=$Mode user=$user"
exit 0
