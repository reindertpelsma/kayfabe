param([string]$Name,[string]$Cmd)
# run a command line in the signed-in user desktop session as a one-shot task; output to C:\kf\t\$Name.txt
$ErrorActionPreference="Stop"
New-Item -ItemType Directory -Force -Path C:\kf\t | Out-Null
$task="kfi_$Name"
try { Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue } catch {}
Remove-Item "C:\kf\t\$Name.txt","C:\kf\t\$Name.done" -ErrorAction SilentlyContinue
$user=(Get-CimInstance Win32_ComputerSystem).UserName
if(-not $user){ "KFRUNI fail no-interactive-user"; exit 3 }
$act=New-ScheduledTaskAction -Execute "cmd.exe" -Argument ("/c " + $Cmd + " > C:\kf\t\$Name.txt 2>&1 & echo %errorlevel% > C:\kf\t\$Name.done")
$pr=New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
$set=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Hours 1)
Register-ScheduledTask -TaskName $task -Action $act -Principal $pr -Settings $set -Force | Out-Null
Start-ScheduledTask -TaskName $task
"KFRUNI ok user=$user name=$Name"
