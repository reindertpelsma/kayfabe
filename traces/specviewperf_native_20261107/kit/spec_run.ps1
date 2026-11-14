# spec_run.ps1 -Workload NAME [-Out C:\spec\out] -- one SPECviewperf-CLI run of one viewset (1 iteration, FullHD), meant to be
# started in the INTERACTIVE session (scheduled task /it). Writes <Out>\<Workload>\{start.txt, cli.log, smi.csv, tdr.txt,
# display.txt, END.txt}. END.txt is written last (`SPECRUN_END rc=<n>`). Reads no secrets. Results (scores) stay in the vendor result dir.
param([Parameter(Mandatory = $true)][string]$Workload, [string]$Out = 'C:\spec\out', [string]$Root = 'C:\Program Files\SPECviewperf 15')
$ErrorActionPreference = 'Continue'
$d = Join-Path $Out $Workload
New-Item -ItemType Directory -Force $d | Out-Null
Remove-Item (Join-Path $d '*') -Force -ErrorAction SilentlyContinue
$t0 = (Get-Date).ToUniversalTime()
"SPECRUN_START workload=$Workload utc=$($t0.ToString('yyyy-MM-ddTHH:mm:ssZ'))" | Set-Content (Join-Path $d 'start.txt')
$smi = "$env:windir\System32\nvidia-smi.exe"
$sampler = Start-Process -FilePath $smi -ArgumentList '--query-gpu=timestamp,memory.used,memory.total,utilization.gpu,pstate,temperature.gpu,power.draw','--format=csv,noheader','-l','5' -RedirectStandardOutput (Join-Path $d 'smi.csv') -NoNewWindow -PassThru
$res = "C:\spec\results\$Workload"
New-Item -ItemType Directory -Force $res | Out-Null
$cli = Join-Path $Root 'SPECviewperf-CLI.exe'
Set-Location $Root
$p = Start-Process -FilePath $cli -ArgumentList '--workload', $Workload, '--iterations', '1', '--resolution', 'FullHD', '--resultDirectory', $res -WorkingDirectory $Root -RedirectStandardOutput (Join-Path $d 'cli.log') -RedirectStandardError (Join-Path $d 'cli.err') -NoNewWindow -PassThru
$h = $p.Handle   # keep the handle so ExitCode survives
$p.WaitForExit()
$rc = $p.ExitCode
try { Stop-Process -Id $sampler.Id -Force } catch { }
$t1 = (Get-Date).ToUniversalTime()
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\gate_state.ps1 -Mode tdr -Since $t0.ToString('yyyy-MM-ddTHH:mm:ssZ') *> (Join-Path $d 'tdr.txt')
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\gate_state.ps1 -Mode display *> (Join-Path $d 'display.txt')
"SPECRUN_END workload=$Workload rc=$rc start=$($t0.ToString('yyyy-MM-ddTHH:mm:ssZ')) end=$($t1.ToString('yyyy-MM-ddTHH:mm:ssZ')) secs=$([int]($t1-$t0).TotalSeconds)" | Set-Content (Join-Path $d 'END.txt')
