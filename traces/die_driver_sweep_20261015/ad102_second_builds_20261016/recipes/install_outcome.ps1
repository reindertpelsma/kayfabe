$ErrorActionPreference='Stop'
$path='C:\ProgramData\KayfabeSweep\driver58151.exit'
if(!(Test-Path $path)){throw 'Installer outcome remains unknown; do not replay or freeze parent'}
if((Get-Item $path).Length -gt 1024){throw 'Outcome marker length invalid'}
$text=Get-Content -Raw $path;Write-Output $text
if($text -notmatch '^INSTALL_EXIT rc=0 '){throw 'Installer did not complete successfully; do not replay'}
