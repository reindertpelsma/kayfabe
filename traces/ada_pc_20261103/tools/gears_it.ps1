# gears_it.ps1 -- run kf_glgears.exe in the signed-in console user's INTERACTIVE session (a windowed GL app
# presenting through DWM, so the display engine keeps flipping), via a one-shot scheduled task; print its output.
param([int]$Seconds = 60, [string]$Tag = 'g')
$ErrorActionPreference = 'Continue'
$user = (Get-CimInstance Win32_ComputerSystem).UserName
if (-not $user) { Write-Output 'GEARS REFUSED no console user'; exit 2 }
$out = "C:\kf\probes\gears_$Tag.out.txt"
Remove-Item $out -ErrorAction SilentlyContinue
$arg = '/c "C:\kf\probes\kf_glgears.exe --seconds ' + $Seconds + ' > ' + $out + ' 2>&1"'
$a = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument $arg
$p = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
$s = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName KF-ADA-GEARS -Action $a -Principal $p -Settings $s -Force | Out-Null
Write-Output ("GEARS_START user=$user utc=" + (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))
Start-ScheduledTask -TaskName KF-ADA-GEARS
$end = (Get-Date).AddSeconds($Seconds + 60)
while ((Get-Date) -lt $end) {
    if ((Test-Path $out) -and ((Get-Content $out -Raw -ErrorAction SilentlyContinue) -match 'KFGL RESULT')) { break }
    Start-Sleep -Milliseconds 500
}
Start-Sleep -Milliseconds 300
if (Test-Path $out) { Get-Content $out } else { Write-Output 'GEARS NO OUTPUT' }
$i = Get-ScheduledTaskInfo -TaskName KF-ADA-GEARS
Write-Output ("GEARS_END utc=" + (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') + " task_result=$($i.LastTaskResult)")
Get-Process kf_glgears -ErrorAction SilentlyContinue | Stop-Process -Force
Unregister-ScheduledTask -TaskName KF-ADA-GEARS -Confirm:$false
