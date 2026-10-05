$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$p='C:\ProgramData\KayfabeGsp\captures\rtx4070-b297-restart-01\gsp.jsonl'
if((Get-Item $p).Length -gt 140MB){throw 'Text export exceeds bound'}
[Console]::Out.Write([IO.File]::ReadAllText($p))
