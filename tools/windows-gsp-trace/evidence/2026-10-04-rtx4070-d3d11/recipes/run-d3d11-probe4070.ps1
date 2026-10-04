$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeGsp'
$trace="$root\captures\rtx4070-d3d11-resume-02"
$probe="$root\build\d3d11_probe-572411c.exe"
$hash='c41e1479cd216bc9c76c0f65eada43ffeed9e49894fa9923a589b91ac9c1cea2'
if((Get-FileHash $probe -Algorithm SHA256).Hash.ToLowerInvariant() -ne $hash){throw 'Probe artifact hash mismatch'}
foreach($path in @('C:\ProgramData\VastWindows\defer-native-gpu.flag','C:\ProgramData\VastWindows\hold-native-reboots.flag')){if(-not [IO.File]::Exists($path)){throw "Missing research hold: $path"}}
if([IO.File]::Exists("$trace\probe-result.json") -or [IO.File]::Exists("$trace\probe-start.json")){throw 'Probe already attempted; refuse automatic duplicate'}
if([IO.File]::Exists("$trace\collector-exit.json") -or (Get-ScheduledTask KayfabeGspCapture4070D3D11).State -ne 'Running'){throw 'Expected active collector'}
if((Get-Service KayfabeGspTrace).Status -ne 'Running' -or ([IO.FileInfo]::new("$trace\gsp.kgwt")).Length -lt 208){throw 'No active observer with recorded traffic'}
[ordered]@{time=[DateTime]::UtcNow.ToString('o');capture_bytes=([IO.FileInfo]::new("$trace\gsp.kgwt")).Length;collector_task='Running';observer='Running';parallel_status_unavailable='exclusive collector handle';validation='strict decode after drain'} | ConvertTo-Json | Set-Content -Encoding UTF8 "$trace\probe-immediate-before-readiness.json"
$start=[DateTime]::UtcNow
[ordered]@{start_time=$start.ToString('o');probe_sha256=$hash;probe_source='572411c10001383e7c46e219c4596194c1f8e5ee';source_sha256='bd6dfeae2c96fde43e000613eaf2747192ae8cfc88702d70248da8d645424dae';compiler='trusted controller MinGW13';timeout_seconds=60;arguments=@()} | ConvertTo-Json | Set-Content -Encoding UTF8 "$trace\probe-start.json"
$process=[Diagnostics.Process]::new()
$process.StartInfo.FileName=$probe
$process.StartInfo.UseShellExecute=$false
$process.StartInfo.CreateNoWindow=$true
$process.StartInfo.RedirectStandardOutput=$true
$process.StartInfo.RedirectStandardError=$true
$timeout=$false;$code=$null;$errorText=$null
try {
 if(-not $process.Start()){throw 'Unable to start probe'}
 $stdout=$process.StandardOutput.ReadToEndAsync()
 $stderr=$process.StandardError.ReadToEndAsync()
 if(-not $process.WaitForExit(60000)) {
  $timeout=$true
  & taskkill.exe /PID $process.Id /T /F | Out-Null
  if(-not $process.WaitForExit(5000)){throw 'Timed-out probe did not terminate'}
 }
 $process.WaitForExit()
 [IO.File]::WriteAllText("$trace\probe.stdout",$stdout.GetAwaiter().GetResult())
 [IO.File]::WriteAllText("$trace\probe.stderr",$stderr.GetAwaiter().GetResult())
 $code=$process.ExitCode
 if($null -eq $code){throw 'Probe exitcode unavailable'}
} catch { $errorText=[string]$_ } finally { $process.Dispose() }
[ordered]@{start_time=$start.ToString('o');end_time=[DateTime]::UtcNow.ToString('o');exit_code=$code;timed_out=$timeout;error=$errorText;probe_sha256=$hash} | ConvertTo-Json | Set-Content -Encoding UTF8 "$trace\probe-result.json"
Get-Content -Raw "$trace\probe-result.json"
if([IO.File]::Exists("$trace\probe.stdout")){Get-Content -Raw "$trace\probe.stdout"}
if([IO.File]::Exists("$trace\probe.stderr")){Get-Content -Raw "$trace\probe.stderr"}
[ordered]@{time=[DateTime]::UtcNow.ToString('o');capture_bytes=([IO.FileInfo]::new("$trace\gsp.kgwt")).Length;observer=[string](Get-Service KayfabeGspTrace).Status} | ConvertTo-Json | Set-Content -Encoding UTF8 "$trace\probe-immediate-after-readiness.json"
if($timeout -or $errorText -or $null -eq $code -or $code -ne 0){throw 'D3D11 probe failed; preserve evidence and drain capture'}
