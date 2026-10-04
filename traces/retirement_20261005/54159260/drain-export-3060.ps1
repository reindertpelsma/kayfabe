$ErrorActionPreference='Stop'
$root='C:\ProgramData\KayfabeGsp'
$trace="$root\captures\rtx3060-gsp-policy-boot"
$collector="$root\build\gsptrace-candidates.exe"
if ((Get-FileHash $collector -Algorithm SHA256).Hash.ToLowerInvariant() -ne '9f1a69942dc52e9e8fded62069b1078a59db06fd2c522e1b1e90596d6fc97bb9') { throw 'Collector hash mismatch' }
if ((Get-FileHash "$root\export-trace.ps1" -Algorithm SHA256).Hash.ToLowerInvariant() -ne '280038c4025f57c90557035d9eac7a3cbf7e404f1b321e020e8945baac8ab4c3') { throw 'Exporter hash mismatch' }
if (Get-Process 'gsptrace*' -ErrorAction SilentlyContinue) { throw 'Collector already running' }
if ([IO.File]::Exists("$trace\retirement-drain.kgwt")) { throw 'Prior retirement drain exists' }
& $collector --status | Set-Content -Encoding UTF8 "$trace\retirement-before.json"
if ($LASTEXITCODE) { throw 'Observer status failed' }
& $collector "$trace\retirement-drain.kgwt" 1 2> "$trace\retirement-drain.stderr" | Set-Content -Encoding UTF8 "$trace\retirement-drain.stdout"
$code=$LASTEXITCODE
[ordered]@{time=[DateTime]::UtcNow.ToString('o');exit_code=$code} | ConvertTo-Json -Compress | Set-Content -Encoding UTF8 "$trace\retirement-drain-exit.json"
if ($code -ne 0) { throw "Retirement drain failed: $code" }
foreach ($name in @('gsp','retirement-drain')) {
  if ((Get-Item "$trace\$name.kgwt").Length -gt 64MB) { throw 'Trace exceeds expected bound' }
  & "$root\export-trace.ps1" -Path "$trace\$name.kgwt" -OutputPath "$trace\$name.jsonl" -MaxInputMiB 64
}
Disable-ScheduledTask -TaskName 'KayfabeGspCapture3060Boot' | Out-Null
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\KayfabeGspTrace' -Name Start -Value 3
Stop-Service KayfabeGspTrace
& nvidia-smi --query-gpu=name,driver_version,pci.device_id --format=csv,noheader | Set-Content -Encoding UTF8 "$trace\retirement-gpu.txt"
[ordered]@{time=[DateTime]::UtcNow.ToString('o');drain_exit=$code;original_collector_exit_missing=(-not [IO.File]::Exists("$trace\collector-exit.json"));observer_status=[string](Get-Service KayfabeGspTrace).Status;files=@(Get-ChildItem $trace -File | ForEach-Object {[ordered]@{name=$_.Name;length=$_.Length;sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}})} | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 "$trace\retirement-manifest.json"
Get-Content -Raw "$trace\retirement-before.json"
Get-Content -Raw "$trace\retirement-drain.kgwt.stats.json"
Get-Content -Raw "$trace\retirement-manifest.json"
