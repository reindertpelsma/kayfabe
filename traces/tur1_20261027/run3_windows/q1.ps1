$d = Get-PnpDevice -PresentOnly | Where-Object InstanceId -like 'PCI\VEN_10DE*'
$d | Format-List FriendlyName,Status,Problem,ConfigManagerErrorCode,InstanceId
Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName DEVPKEY_Device_ProblemStatus,DEVPKEY_Device_ProblemCode | Format-List KeyName,Data
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=(Get-Date).AddHours(-2)} -ErrorAction SilentlyContinue |
  Where-Object { $_.ProviderName -match 'nvlddmkm|Display|Kernel-PnP|dxgkrnl|Microsoft-Windows-Kernel-PnP|UserPnp|DriverFrameworks' } |
  Select-Object -First 40 TimeCreated,Id,ProviderName,@{n='M';e={($_.Message -replace '\s+',' ').Substring(0,[Math]::Min(300,$_.Message.Length))}} | Format-List
Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,Status,ConfigManagerErrorCode | Format-List
