$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
New-Item -ItemType Directory -Force 'C:\ProgramData\KayfabeSweep\downloads' | Out-Null
Write-Output 'STAGE_DIRECTORY_READY=1'
