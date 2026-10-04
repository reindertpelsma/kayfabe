$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeGsp'
$trace="$root\captures\rtx4070-d3d11-resume-02"
$body="$root\manual-d3d11-collector4070.ps1"
$collector="$root\build\gsptrace-candidates-0797f6ae.exe"
if (Test-Path $trace) { throw 'Refuse to overwrite a previous capture' }
foreach($path in @('C:\ProgramData\VastWindows\defer-native-gpu.flag','C:\ProgramData\VastWindows\hold-native-reboots.flag')) { if(!(Test-Path $path)){throw "Required research hold absent: $path"} }
foreach($entry in @(
 @($body,'f1780570e4f94ee5020d78c3ec4503fea038bf9ede7903637ddec94b38a80f27'),
 @($collector,'9f1a69942dc52e9e8fded62069b1078a59db06fd2c522e1b1e90596d6fc97bb9'),
 @('C:\Windows\System32\drivers\kayfabe-gsptrace.sys','e311595276d7451504d7515ce4a592441bd8cdad20c0c15b6e08cce30b90b753'))) {
 if((Get-FileHash $entry[0] -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry[1]){throw "Artifact hash mismatch: $($entry[0])"}
}
if((Get-Service KayfabeGspTrace).Status -ne 'Stopped'){throw 'Prior observer not stopped'}
$original=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\KayfabeGspTrace').Start
if($original -ne 3){throw 'Expected original demand-start service'}
$gpu=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object {$_.InstanceId -like 'PCI\VEN_10DE&DEV_2786&*'})
if($gpu.Count -ne 1 -or $gpu[0].Status -ne 'OK' -or [int]$gpu[0].Problem -ne 0){throw 'Expected healthy assigned RTX4070'}
New-Item -ItemType Directory $trace | Out-Null
Copy-Item $body "$trace\collect-task.ps1"
[ordered]@{time=[DateTime]::UtcNow.ToString('o');original_observer_start=$original;gpu=$gpu[0].InstanceId;collector_sha256='9f1a69942dc52e9e8fded62069b1078a59db06fd2c522e1b1e90596d6fc97bb9';driver_signed_sha256='e311595276d7451504d7515ce4a592441bd8cdad20c0c15b6e08cce30b90b753';collector_source='0797f6ae20b760b283116514a993d19273d2fd9d';max_seconds=300;max_capture_mib_before_drain=64;history_complete=$false} | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 "$trace\capture-journal.json"
Start-Service KayfabeGspTrace
$principal=New-ScheduledTaskPrincipal -UserId SYSTEM -LogonType ServiceAccount -RunLevel Highest
$settings=New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Minutes 10)
$action=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "'+$trace+'\collect-task.ps1"')
Register-ScheduledTask -TaskName KayfabeGspCapture4070D3D11 -Action $action -Principal $principal -Settings $settings | Out-Null
Export-ScheduledTask -TaskName KayfabeGspCapture4070D3D11 | Set-Content -Encoding UTF8 "$trace\capture-task.xml"
Start-ScheduledTask -TaskName KayfabeGspCapture4070D3D11
$ready=$false
for($i=0;$i -lt 40;$i++) {
 Start-Sleep -Seconds 1
 if([IO.File]::Exists("$trace\collector-exit.json")){throw ('Collector exited before probe: '+[IO.File]::ReadAllText("$trace\collector-exit.json"))}
 if([IO.File]::Exists("$trace\startup-error.json")){throw ([IO.File]::ReadAllText("$trace\startup-error.json"))}
 if([IO.File]::Exists("$trace\gsp.kgwt") -and ([IO.FileInfo]::new("$trace\gsp.kgwt")).Length -ge 64 -and (Get-ScheduledTask KayfabeGspCapture4070D3D11).State -eq 'Running') {
  # The collector owns an exclusive device handle. A parallel --status open
  # fails; require a written record and validate all records after drain.
  if(([IO.FileInfo]::new("$trace\gsp.kgwt")).Length -ge 208 -and (Get-Service KayfabeGspTrace).Status -eq 'Running') {
   $ready=$true
   [ordered]@{time=[DateTime]::UtcNow.ToString('o');capture_bytes=([IO.FileInfo]::new("$trace\gsp.kgwt")).Length;collector_task='Running';observer='Running';parallel_status_unavailable='exclusive collector handle';validation='strict decode after drain'} | ConvertTo-Json | Set-Content -Encoding UTF8 "$trace\before-probe-readiness.json"
   break
  }
 }
}
if(-not $ready){New-Item -ItemType File "$trace\stop.flag" -Force | Out-Null; throw 'Recorder never attached a GSP table before probe'}
Get-Content -Raw "$trace\before-probe-readiness.json"
'CAPTURE_READY_FOR_PROBE'
