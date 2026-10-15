$ErrorActionPreference='Stop'
$gpu=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
if($gpu.Count -ne 1 -or $gpu[0].DriverVersion -ne '32.0.15.8088' -or $gpu[0].ConfigManagerErrorCode -ne 0) { throw 'Refuse removal unless target580.88 is healthy/current' }
$d=Get-WindowsDriver -Online -Driver oem13.inf
Write-Output "UNUSED_STORE_PACKAGE name=$($d.Driver) provider=$($d.ProviderName) version=$($d.Version) class=$($d.ClassName)"
if($d.ProviderName -ne 'NVIDIA' -or $d.Version -ne '32.0.15.9186' -or $d.ClassName -ne 'Display') { throw 'Unexpected package; no removal' }
& pnputil.exe /delete-driver oem13.inf
if($LASTEXITCODE) { throw 'Unused driver removal failed' }
Write-Output "UNUSED_WU_DRIVER_REMOVED=1 $([DateTime]::UtcNow.ToString('o'))"
