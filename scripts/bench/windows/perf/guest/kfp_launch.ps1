# kfp_launch.ps1 -Tag T [-Mode service|interactive] : one-shot scheduled task running kfp_body.ps1 for job C:\kfperf\jobs\T.ps1; returns at once.
param([string]$Tag, [string]$Mode = "service")
$ErrorActionPreference = "Stop"
$task = "kfperf_$Tag"
try { Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue } catch {}
$act = New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\kfperf\kfp_body.ps1 -Tag $Tag -Job C:\kfperf\jobs\$Tag.ps1"
if ($Mode -eq "interactive") {
  $user = (Get-CimInstance Win32_ComputerSystem).UserName
  if (-not $user) { "KFPLAUNCH fail no-interactive-user"; exit 3 }
  $pr = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
} else { $user = "SYSTEM"; $pr = New-ScheduledTaskPrincipal -UserId "SYSTEM" -LogonType ServiceAccount -RunLevel Highest }
$set = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Hours 2) -StartWhenAvailable
Register-ScheduledTask -TaskName $task -Action $act -Principal $pr -Settings $set -Force | Out-Null
Start-ScheduledTask -TaskName $task
"KFPLAUNCH ok mode=$Mode user=$user"
