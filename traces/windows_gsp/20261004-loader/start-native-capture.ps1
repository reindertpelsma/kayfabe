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
$process=$null; $stdout=$null; $stderr=$null; $outCopy=$null; $errCopy=$null; $started=$false
function Complete-CollectorStreams {
  if ($outCopy) { $null=$outCopy.GetAwaiter().GetResult(); $stdout.Flush($true) }
  if ($errCopy) { $null=$errCopy.GetAwaiter().GetResult(); $stderr.Flush($true) }
}
try {
  $arguments='"'+$trace+'\gsp.kgwt" 7200 --stop-file "'+$trace+'\stop.flag"'
  $process=[Diagnostics.Process]::new()
  $process.StartInfo.FileName="$root\build\gsptrace.exe"
  $process.StartInfo.Arguments=$arguments
  $process.StartInfo.UseShellExecute=$false
  $process.StartInfo.CreateNoWindow=$true
  $process.StartInfo.RedirectStandardOutput=$true
  $process.StartInfo.RedirectStandardError=$true
  $stdout=[IO.File]::Open("$trace\collector.stdout",[IO.FileMode]::Create,[IO.FileAccess]::Write,[IO.FileShare]::Read)
  $stderr=[IO.File]::Open("$trace\collector.stderr",[IO.FileMode]::Create,[IO.FileAccess]::Write,[IO.FileShare]::Read)
  if (-not $process.Start()) { throw 'Cannot start collector process' }
  $started=$true
  # Copy both pipes concurrently; avoid pipe-buffer deadlocks and unbounded strings.
  $outCopy=$process.StandardOutput.BaseStream.CopyToAsync($stdout)
  $errCopy=$process.StandardError.BaseStream.CopyToAsync($stderr)
  while (-not $process.WaitForExit(1000)) {
    if (((Get-Item "$trace\gsp.kgwt" -ErrorAction SilentlyContinue).Length -gt 256MB) -or ((Get-PSDrive C).Free -lt 10GB)) {
      New-Item -ItemType File -Path "$trace\stop.flag" -Force | Out-Null
      'Capture stopped at disk/size bound' | Set-Content "$trace\storage-limit.txt"
    }
  }
  $process.WaitForExit()
  Complete-CollectorStreams
  $code=$process.ExitCode
  if ($null -eq $code) { throw 'Collector exit code is unavailable' }
  [ordered]@{time=(Get-Date).ToUniversalTime().ToString('o');exit_code=[int]$code} | ConvertTo-Json -Compress | Set-Content "$trace\collector-exit.json"
  exit $code
} catch {
  $failure=[string]$_
  $drained=$true
  if ($started -and -not $process.HasExited) {
    New-Item -ItemType File -Path "$trace\stop.flag" -Force | Out-Null
    $drained=$process.WaitForExit(30000)
  }
  if ($started -and $drained) {
    try { $process.WaitForExit(); Complete-CollectorStreams }
    catch { $failure += '; stream drain: '+[string]$_ }
  }
  [ordered]@{time=(Get-Date).ToUniversalTime().ToString('o');error=$failure;collector_drained=$drained;exit_code=1} | ConvertTo-Json -Compress | Set-Content "$trace\collector-exit.json"
  exit 1
} finally {
  if ($stdout) { $stdout.Dispose() }
  if ($stderr) { $stderr.Dispose() }
  if ($process) { $process.Dispose() }
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
