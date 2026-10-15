# bl_run.ps1 -Tag <t> -BlArgs "<blender args>" [-TimeoutSec 1800] [-Priority Normal] [-Gui 0]
# Runs one Blender process we start ourselves, with the nvidia-smi sampler beside it; writes C:\kf\apps\blender\out\<Tag>\{run.txt,stdout.txt,stderr.txt,smi.csv}
# and PUTs them to the host file server (10.0.2.2:8790). Kills only the Blender pid it started.
param([string]$Tag, [string]$BlArgs, [int]$TimeoutSec = 1800, [string]$Priority = 'Normal')
$d = "C:\kf\apps\blender\out\$Tag"; New-Item -ItemType Directory -Force $d | Out-Null; Remove-Item "$d\stop.flag" -ErrorAction SilentlyContinue
$bl = 'C:\Program Files\Blender Foundation\Blender 5.2\blender.exe'
"START $Tag $((Get-Date).ToString('HH:mm:ss.fff')) args=$BlArgs" | Set-Content "$d\run.txt"
$sm = Start-Process powershell -ArgumentList '-NoProfile','-File','C:\kf\apps\blender\smi_sampler.ps1','-Dir',$d -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 4
$p = Start-Process $bl -ArgumentList ($BlArgs -replace '#d', $d) -PassThru -WindowStyle Hidden -RedirectStandardOutput "$d\stdout.txt" -RedirectStandardError "$d\stderr.txt"
try { $p.PriorityClass = $Priority } catch {}
$null = $p.Handle
"PID $($p.Id)" | Add-Content "$d\run.txt"
if (-not $p.WaitForExit($TimeoutSec * 1000)) { "TIMEOUT after ${TimeoutSec}s, killing own pid $($p.Id)" | Add-Content "$d\run.txt"; $p.Kill() }
$p.WaitForExit()
"END rc=$($p.ExitCode) $((Get-Date).ToString('HH:mm:ss.fff'))" | Add-Content "$d\run.txt"
Start-Sleep -Seconds 3; "" | Set-Content "$d\stop.flag"; Start-Sleep -Seconds 3
foreach ($n in 'run.txt','stdout.txt','stderr.txt','smi.csv','result.json') {
  if (Test-Path "$d\$n") { try { Invoke-RestMethod -Method Put -Uri "http://10.0.2.2:8790/${Tag}__$n" -InFile "$d\$n" -UseBasicParsing | Out-Null } catch {} }
}
"UPLOADED" | Add-Content "$d\run.txt"
try { Invoke-RestMethod -Method Put -Uri "http://10.0.2.2:8790/${Tag}__run.txt" -InFile "$d\run.txt" -UseBasicParsing | Out-Null } catch {}
