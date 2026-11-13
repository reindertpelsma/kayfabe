param([string]$Name,[string]$Ps)
# Ps = PowerShell snippet (no double quotes) run hidden in the interactive session; output C:\kf\t\Name.txt, marker Name.done
$ErrorActionPreference="Stop"
New-Item -ItemType Directory -Force -Path C:\kf\t | Out-Null
$task="kfi_$Name"
try { Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue } catch {}
Remove-Item "C:\kf\t\$Name.txt","C:\kf\t\$Name.done" -ErrorAction SilentlyContinue
$user=(Get-CimInstance Win32_ComputerSystem).UserName
if(-not $user){ "KFRUNI fail no-interactive-user"; exit 3 }
$inner="& { $Ps } *> C:\kf\t\$Name.txt; 'done' | Out-File C:\kf\t\$Name.done"
$act=New-ScheduledTaskAction -Execute "powershell.exe" -Argument ("-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command `"" + $inner + "`"")
$pr=New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
$set=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Hours 1)
Register-ScheduledTask -TaskName $task -Action $act -Principal $pr -Settings $set -Force | Out-Null
Start-ScheduledTask -TaskName $task
"KFRUNI ok $Name"
