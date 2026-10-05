$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$trace='C:\ProgramData\KayfabeGsp\captures\rtx4070-b297-restart-01'
[ordered]@{time=[DateTime]::UtcNow.ToString('o');display=@(Get-PnpDevice -PresentOnly -Class Display | Select-Object Status,Problem,InstanceId);observer=[string](Get-Service KayfabeGspTrace).Status;task=[string](Get-ScheduledTask KayfabeGspCapture4070B297).State;records=(Get-Content -Raw "$trace\gsp.kgwt.stats.json" | ConvertFrom-Json);collector_exit=(Get-Content -Raw "$trace\collector-exit.json" | ConvertFrom-Json);restart=(Get-Content -Raw "$trace\restart-result.json" | ConvertFrom-Json)} | ConvertTo-Json -Depth 8
& nvidia-smi -q | Select-String 'Driver Version|CUDA Version|Product Name|GSP Firmware Version'
