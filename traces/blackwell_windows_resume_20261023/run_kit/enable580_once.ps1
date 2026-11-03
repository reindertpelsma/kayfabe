$ErrorActionPreference='Stop'
$g=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object InstanceId -like 'PCI\VEN_10DE*')
if($g.Count -ne 1 -or [int]$g[0].Problem -ne 22) { throw 'Require one disabled device before single580 enable' }
$id=$g[0].InstanceId
$v=(Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_DriverVersion).Data
if($v -ne '32.0.15.8088') { throw 'Actual580.51 required' }
$marker='C:\kf\enable580_once.started'
if(Test-Path $marker) { throw 'An earlier580 enable exists; inspect outcome, never replay' }
[DateTime]::UtcNow.ToString('o') | Set-Content $marker
Write-Output "ENABLE580_START actual=$v UTC=$([DateTime]::UtcNow.ToString('o'))"
pnputil /enable-device "$id"
Write-Output "ENABLE580_PNPUTIL_RC=$LASTEXITCODE"
Start-Sleep -Seconds 20
Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,ConfigManagerErrorCode | ConvertTo-Json -Compress
Get-PnpDevice -InstanceId $id | Select-Object FriendlyName,Status,Problem | ConvertTo-Json -Compress
Write-Output "ENABLE580_OBSERVED UTC=$([DateTime]::UtcNow.ToString('o')); device status is verdict, not this command rc"
