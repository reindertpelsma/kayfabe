$ErrorActionPreference='Stop'
$root='C:\ProgramData\KayfabeGsp'
$trace="$root\captures\rtx4070-b297-restart-01"
if (-not [IO.File]::Exists("$trace\startup.json")) { throw 'No boot capture startup evidence' }
New-Item -ItemType File -Path "$trace\stop.flag" -Force | Out-Null
$deadline=[DateTime]::UtcNow.AddSeconds(60)
while (-not [IO.File]::Exists("$trace\collector-exit.json")) {
  if ([DateTime]::UtcNow -ge $deadline) { throw 'Collector drain timed out; preserve service and source without exporting' }
  Start-Sleep -Milliseconds 250
}
$exit=Get-Content -Raw "$trace\collector-exit.json" | ConvertFrom-Json
if ($null -eq $exit.exit_code -or [int]$exit.exit_code -ne 0) { throw "Collector exit is not zero: $($exit|ConvertTo-Json -Compress)" }
Get-ChildItem $trace | Select-Object Name,Length | ConvertTo-Json
$stats=Get-Content -Raw "$trace\gsp.kgwt.stats.json" | ConvertFrom-Json
if ($stats.buffered_bytes -ne 0) { throw "Collector did not drain FIFO: $($stats.buffered_bytes)" }
if ((Get-FileHash "$root\export-trace.ps1" -Algorithm SHA256).Hash.ToLowerInvariant() -ne '280038c4025f57c90557035d9eac7a3cbf7e404f1b321e020e8945baac8ab4c3') { throw 'Exporter hash mismatch' }
& "$root\export-trace.ps1" -Path "$trace\gsp.kgwt" -OutputPath "$trace\gsp.jsonl" -MaxInputMiB 512
& "$root\export-trace.ps1" -Path "$trace\gsp.kgwt" -OutputPath "$trace\gfx-pool-query.jsonl" -MaxInputMiB 512 -GfxPoolOnly
Disable-ScheduledTask -TaskName 'KayfabeGspCapture4070B297' | Out-Null
Set-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\KayfabeGspTrace' -Name Start -Value 3
Stop-Service KayfabeGspTrace
[ordered]@{time=[DateTime]::UtcNow.ToString('o');collector_exit=[int]$exit.exit_code;stats=$stats;task_state=[string](Get-ScheduledTask -TaskName 'KayfabeGspCapture4070B297').State;observer_start=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\KayfabeGspTrace').Start;observer_status=[string](Get-Service KayfabeGspTrace).Status;native_deferred=[IO.File]::Exists('C:\ProgramData\VastWindows\defer-native-gpu.flag');reboots_held=[IO.File]::Exists('C:\ProgramData\VastWindows\hold-native-reboots.flag');files=@(Get-ChildItem $trace -File | ForEach-Object {[ordered]@{name=$_.Name;length=$_.Length}})} | ConvertTo-Json -Depth 8 | Set-Content "$trace\restored.json"
Get-Content -Raw "$trace\restored.json"
