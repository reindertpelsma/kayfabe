$ErrorActionPreference='Stop'
$trace='C:\ProgramData\KayfabeGsp\captures\rtx4070-b297-restart-01'
if(-not [IO.File]::Exists("$trace\before-probe-stats.json")){throw 'No observer-ready evidence'}
if([IO.File]::Exists("$trace\restart-started.json")){throw 'Restart already requested; inspect outcome'}
$gpu=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object {$_.InstanceId -like 'PCI\VEN_10DE&DEV_2786&*'})
if($gpu.Count -ne 1 -or $gpu[0].Status -ne 'OK' -or [int]$gpu[0].Problem -ne 0){throw 'Require exactly one healthy assigned RTX4070'}
[ordered]@{time=[DateTime]::UtcNow.ToString('o');instance_id=$gpu[0].InstanceId;before=$gpu[0].Status;operation='pnputil /restart-device';observer_stats=(Get-Content -Raw "$trace\before-probe-stats.json" | ConvertFrom-Json)} | ConvertTo-Json -Depth 8 | Set-Content "$trace\restart-started.json"
$p=Start-Process -FilePath "$env:SystemRoot\System32\pnputil.exe" -ArgumentList @('/restart-device',('"'+$gpu[0].InstanceId+'"')) -PassThru -NoNewWindow -RedirectStandardOutput "$trace\restart.stdout" -RedirectStandardError "$trace\restart.stderr"
if(-not $p.WaitForExit(60000)){throw 'Restart deadline; outcome unknown, inspect before retrying'}
$p.WaitForExit();$rc=$p.ExitCode
Start-Sleep -Seconds 20
$after=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object {$_.InstanceId -like 'PCI\VEN_10DE&DEV_2786&*'} | Select-Object Status,Problem,InstanceId)
& nvidia-smi --query-gpu=name,driver_version --format=csv,noheader | Set-Content "$trace\after-smi.stdout"
$smi=$LASTEXITCODE
[ordered]@{time=[DateTime]::UtcNow.ToString('o');restart_exit=$rc;smi_exit=$smi;after=$after} | ConvertTo-Json -Depth 6 | Set-Content "$trace\restart-result.json"
Get-Content -Raw "$trace\restart-result.json"
Get-Content -Raw "$trace\restart.stdout"
Get-Content -Raw "$trace\after-smi.stdout"
if($rc -ne 0 -or $smi -ne 0 -or $after.Count -ne 1 -or $after[0].Status -ne 'OK'){throw 'Native restart did not restore a healthy GPU'}
