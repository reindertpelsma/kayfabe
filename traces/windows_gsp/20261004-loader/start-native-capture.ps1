$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeGsp'
$trace="$root\captures\rtx3060-first-install"
if (Test-Path $trace) { throw 'Capture directory already exists; inspect the prior run' }
if (-not (Test-Path 'C:\ProgramData\VastWindows\defer-native-gpu.flag')) { throw 'Native install is not deferred' }
if (-not (Test-Path 'C:\ProgramData\VastWindows\hold-native-reboots.flag')) { throw 'Native reboots are not held' }
$gpu=@(Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'PCI\VEN_10DE&*' -and $_.Class -eq 'Display' })
if (-not $gpu.Count) { throw 'No NVIDIA display PCI device is present' }
New-Item -ItemType Directory -Path $trace | Out-Null
& "$root\metadata.ps1" -OutputPath "$trace\before.json"
Start-Service KayfabeGspTrace
& "$root\build\gsptrace.exe" --status
if ($LASTEXITCODE -ne 0) { throw 'Recorder initial status failed' }
$body=@'
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeGsp'
$trace="$root\captures\rtx3060-first-install"
try {
  $arguments='"'+$trace+'\gsp.kgwt" 7200 --stop-file "'+$trace+'\stop.flag"'
  $process=Start-Process -FilePath "$root\build\gsptrace.exe" -ArgumentList $arguments -NoNewWindow -PassThru -RedirectStandardOutput "$trace\collector.stdout" -RedirectStandardError "$trace\collector.stderr"
  while (-not $process.WaitForExit(1000)) {
    if (((Get-Item "$trace\gsp.kgwt" -ErrorAction SilentlyContinue).Length -gt 256MB) -or ((Get-PSDrive C).Free -lt 10GB)) {
      New-Item -ItemType File -Path "$trace\stop.flag" -Force | Out-Null
      'Capture stopped at disk/size bound' | Set-Content "$trace\storage-limit.txt"
    }
  }
  $process.WaitForExit()
  $process.Refresh()
  $code=$process.ExitCode
  [ordered]@{time=(Get-Date).ToUniversalTime().ToString('o');exit_code=$code} | ConvertTo-Json -Compress | Set-Content "$trace\collector-exit.json"
  exit $code
} catch {
  $failure=[string]$_
  $drained=$true
  if ($process -and -not $process.HasExited) {
    New-Item -ItemType File -Path "$trace\stop.flag" -Force | Out-Null
    $drained=$process.WaitForExit(30000)
  }
  [ordered]@{time=(Get-Date).ToUniversalTime().ToString('o');error=$failure;collector_drained=$drained;exit_code=1} | ConvertTo-Json -Compress | Set-Content "$trace\collector-exit.json"
  exit 1
}
'@
$body | Set-Content "$trace\collect-task.ps1" -Encoding UTF8
$principal=New-ScheduledTaskPrincipal -UserId SYSTEM -LogonType ServiceAccount -RunLevel Highest
$settings=New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Minutes 125)
$action=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "'+$trace+'\collect-task.ps1"')
Register-ScheduledTask -TaskName KayfabeGspCapture3060 -Action $action -Principal $principal -Settings $settings | Out-Null
Start-ScheduledTask -TaskName KayfabeGspCapture3060
$ready=$false
for ($i=0;$i -lt 30;$i++) {
  Start-Sleep -Seconds 1
  if (Test-Path "$trace\collector-exit.json") { throw ('Collector exited: '+[IO.File]::ReadAllText("$trace\collector-exit.json")) }
  if ((Test-Path "$trace\gsp.kgwt") -and (Get-Item "$trace\gsp.kgwt").Length -ge 64 -and (Get-ScheduledTask KayfabeGspCapture3060).State -eq 'Running') { $ready=$true; break }
}
if (-not $ready) { throw 'Collector did not produce its header before GPU installation' }
$action=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "C:\ProgramData\VastWindows\native-gpu.ps1" -Resume -ForceGsp -HoldReboots'
$settings=New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Hours 2)
Register-ScheduledTask -TaskName KayfabeNativeGpu3060 -Action $action -Principal $principal -Settings $settings | Out-Null
Start-ScheduledTask -TaskName KayfabeNativeGpu3060
'RECORDER_RUNNING_NATIVE_INSTALL_STARTED'
$gpu | Select-Object InstanceId,Status,Problem | ConvertTo-Json -Compress
