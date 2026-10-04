$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeGsp'
$trace="$root\captures\rtx4070-d3d11-resume-02"
foreach($name in @('gsp.kgwt','startup.json','collector-exit.json')) {
  if([IO.File]::Exists("$trace\$name")){throw 'A prior boot capture exists; refuse to overwrite its evidence'}
}
try {
  if ((Get-Service KayfabeGspTrace).Status -ne 'Running') { throw 'Observer is not running' }
  if ((Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\KayfabeGspTrace').Start -ne 3) { throw 'Observer is not configured for demand start' }
  if (-not [IO.File]::Exists('C:\ProgramData\VastWindows\defer-native-gpu.flag')) { throw 'Native setup is not deferred' }
  & "$root\build\gsptrace-candidates-0797f6ae.exe" --status | Set-Content -Encoding UTF8 "$trace\manual-observer-initial.json"
  if ($LASTEXITCODE) { throw 'Boot observer status failed' }
  [ordered]@{time=(Get-Date).ToUniversalTime().ToString('o');boot_tick_ms=[Environment]::TickCount;observer_system_start=$false;history_complete=$false} | ConvertTo-Json -Compress | Set-Content "$trace\startup.json"
} catch {
  [ordered]@{time=(Get-Date).ToUniversalTime().ToString('o');error=[string]$_} | ConvertTo-Json -Compress | Set-Content "$trace\startup-error.json"
  exit 1
}
$process=$null; $stdout=$null; $stderr=$null; $outCopy=$null; $errCopy=$null; $started=$false
function Complete-CollectorStreams {
  if ($outCopy) { $null=$outCopy.GetAwaiter().GetResult(); $stdout.Flush($true) }
  if ($errCopy) { $null=$errCopy.GetAwaiter().GetResult(); $stderr.Flush($true) }
}
try {
  $arguments='"'+$trace+'\gsp.kgwt" 300 --stop-file "'+$trace+'\stop.flag"'
  $process=[Diagnostics.Process]::new()
  $process.StartInfo.FileName="$root\build\gsptrace-candidates-0797f6ae.exe"
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
    if (((Get-Item "$trace\gsp.kgwt" -ErrorAction SilentlyContinue).Length -gt 64MB) -or ((Get-PSDrive C).Free -lt 10GB)) {
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
