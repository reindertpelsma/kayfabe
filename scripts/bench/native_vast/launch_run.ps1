# launch_run.ps1 -Api X -Label L : start heaven_job.ps1 in the interactive session of `vast` via a scheduled task, wait for DONE, print events.
param([Parameter(Mandatory)][string]$Api, [Parameter(Mandatory)][string]$Label, [int]$WaitS = 700)
$ErrorActionPreference = 'Continue'
$run = "C:\bench\runs\$Label"; Remove-Item $run -Recurse -Force -ErrorAction SilentlyContinue
schtasks /Delete /TN kfhv /F 2>$null | Out-Null
$tr = "powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File C:\bench\heaven_job.ps1 -Api $Api -Label $Label"
schtasks /Create /TN kfhv /TR $tr /SC ONCE /ST 23:59 /RU vast /IT /RL HIGHEST /F | Out-Null
schtasks /Run /TN kfhv | Out-Null
$t0 = Get-Date
while (((Get-Date) - $t0).TotalSeconds -lt $WaitS -and -not (Test-Path "$run\DONE")) { Start-Sleep 3 }
"LAUNCH_RUN done=$(Test-Path "$run\DONE") waited=$([int]((Get-Date)-$t0).TotalSeconds)s"
Get-Content "$run\events.txt" -ErrorAction SilentlyContinue
schtasks /Delete /TN kfhv /F 2>$null | Out-Null
