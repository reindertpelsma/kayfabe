$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
if((Get-Service KayfabeGspTrace).Status -ne 'Stopped'){throw 'Observer must be stopped first'}
$trace='C:\ProgramData\KayfabeGsp\captures\rtx4070-d3d11-resume-02'
$probe=Get-Content -Raw "$trace\probe-result.json" | ConvertFrom-Json
$restored=Get-Content -Raw "$trace\restored.json" | ConvertFrom-Json
if($probe.exit_code -ne 0 -or $restored.collector_exit -ne 0 -or $restored.stats.buffered_bytes -ne 0){throw 'Evidence not complete'}
[ordered]@{time=[DateTime]::UtcNow.ToString('o');action='shutdown Windows fixture; supervisor restores Linux PCI ownership';probe_exit=$probe.exit_code;observer=[string](Get-Service KayfabeGspTrace).Status} | ConvertTo-Json
& shutdown.exe /s /t 5 /c 'Kayfabe native D3D11 evidence saved; return GPU to Linux'
if($LASTEXITCODE -ne 0){throw 'Guest shutdown request failed'}
