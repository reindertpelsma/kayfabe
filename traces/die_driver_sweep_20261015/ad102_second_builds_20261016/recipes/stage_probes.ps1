$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
$cd=@([IO.DriveInfo]::GetDrives() | Where-Object { $_.DriveType -eq [IO.DriveType]::CDRom -and [IO.File]::Exists((Join-Path $_.RootDirectory.FullName 'kf_dxprobe.exe')) })
if($cd.Count -ne 1) { throw 'One public probe payload CD required' }
$root='C:\ProgramData\KayfabeSweep';New-Item -ItemType Directory -Force $root | Out-Null
foreach($name in 'cup2.exe','cup3.exe','cup8.exe','kf_dxprobe.exe') { Copy-Item -LiteralPath (Join-Path $cd[0].RootDirectory.FullName $name) -Destination (Join-Path $root $name) -Force }
Write-Output 'PUBLIC_PROBES_STAGED=1'
