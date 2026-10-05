$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$trace='C:\ProgramData\KayfabeGsp\captures\rtx4070-b297-restart-01'
[ordered]@{task=[string](Get-ScheduledTask KayfabeGspCapture4070B297).State;service=[string](Get-Service KayfabeGspTrace).Status;files=@(Get-ChildItem $trace -File | Select-Object Name,Length);initial=(Get-Content -Raw "$trace\manual-observer-initial.json" | ConvertFrom-Json)} | ConvertTo-Json -Depth 8
foreach($name in @('startup-error.json','collector-exit.json','collector.stderr')){if([IO.File]::Exists("$trace\$name")){Get-Content -Raw "$trace\$name"}}
if((Get-ScheduledTask KayfabeGspCapture4070B297).State -eq 'Running' -and [IO.File]::Exists("$trace\gsp.kgwt") -and (Get-Item "$trace\gsp.kgwt").Length -gt 64 -and -not [IO.File]::Exists("$trace\collector-exit.json")){
 [ordered]@{time=[DateTime]::UtcNow.ToString('o');readiness='Collector has written observed records beyond header; exclusive driver handle belongs to collector';capture_bytes=(Get-Item "$trace\gsp.kgwt").Length} | ConvertTo-Json | Set-Content "$trace\before-probe-stats.json"
 'CAPTURE_READY_FOR_RESTART'
}
