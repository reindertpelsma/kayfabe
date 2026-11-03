$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$g=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object InstanceId -like 'PCI\VEN_10DE*')
if($g.Count -ne 1) { throw 'Exactly one NVIDIA function required' }
$id=$g[0].InstanceId
$v=(Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_DriverVersion).Data
$inf=(Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_DriverInfPath).Data
if($v -ne '32.0.15.8088' -or $inf -notmatch '^oem[0-9]+\.inf$') { throw 'Actual580.88 binding required; no install replay' }
$since=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
try { $tdr=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101;StartTime=$since} -ErrorAction Stop) } catch { if($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*'){throw}; $tdr=@() }
$w=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like 'PCI\VEN_10DE*')
if($w.Count -ne 1){throw 'Exactly one actual video controller required'}
$r=[ordered]@{utc=[DateTime]::UtcNow.ToString('o');driver=$v;inf=$inf;problem=[int]$g[0].Problem;code=[int]$w[0].ConfigManagerErrorCode;tdr4101=$tdr.Count;dwm=@(Get-Process dwm -ErrorAction SilentlyContinue).Count;explorer=@(Get-Process explorer -ErrorAction SilentlyContinue).Count;logonui=@(Get-Process LogonUI -ErrorAction SilentlyContinue).Count}
'BW580_RESULT '+($r | ConvertTo-Json -Compress)
