$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\ProgramData\KayfabeGsp'
$trace="$root\captures\rtx3060-gsp-policy-boot"
if (Get-Process 'gsptrace*' -ErrorAction SilentlyContinue) { throw 'Collector still running' }
if ((Get-FileHash "$root\export-trace.ps1" -Algorithm SHA256).Hash.ToLowerInvariant() -ne '280038c4025f57c90557035d9eac7a3cbf7e404f1b321e020e8945baac8ab4c3') { throw 'Exporter hash mismatch' }
Get-Content -Raw "$trace\retirement-before.json"
Get-Content -Raw "$trace\retirement-drain.kgwt.stats.json"
foreach ($name in @('gsp','retirement-drain')) {
  if ((Get-Item "$trace\$name.kgwt").Length -gt 64MB) { throw 'Trace exceeds expected bound' }
  & "$root\export-trace.ps1" -Path "$trace\$name.kgwt" -OutputPath "$trace\$name.jsonl" -MaxInputMiB 64
}
Disable-ScheduledTask -TaskName 'KayfabeGspCapture3060Boot' | Out-Null
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\KayfabeGspTrace' -Name Start -Value 3
Stop-Service KayfabeGspTrace
& nvidia-smi --query-gpu=name,driver_version,pci.device_id --format=csv,noheader | Set-Content -Encoding UTF8 "$trace\retirement-gpu.txt"
[ordered]@{time=[DateTime]::UtcNow.ToString('o');drain_exit_unavailable=$true;original_collector_exit_missing=(-not [IO.File]::Exists("$trace\collector-exit.json"));observer_status=[string](Get-Service KayfabeGspTrace).Status;files=@(Get-ChildItem $trace -File | ForEach-Object {[ordered]@{name=$_.Name;length=$_.Length;sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}})} | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 "$trace\retirement-manifest.json"
Get-Content -Raw "$trace\retirement-manifest.json"
