$ErrorActionPreference='Stop'
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
# PnPEntity includes disabled devices, so the initial Code22 handshake is observable.
$v=@(Get-CimInstance Win32_PnPEntity | Where-Object {$_.PNPDeviceID -like 'PCI\VEN_10DE*' -and $_.PNPClass -eq 'Display'})
if($v.Count -ne 1){throw "expected one NVIDIA display device, actual=$($v.Count)"}
if($null -eq $v[0].ConfigManagerErrorCode){throw 'CIM error code absent'}
$d=@(Get-CimInstance Win32_PnPSignedDriver | Where-Object {$_.DeviceID -eq $v[0].PNPDeviceID})
if($d.Count -ne 1){throw "expected one bound driver, actual=$($d.Count)"}
$ev=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101;StartTime=$boot} -ErrorAction SilentlyContinue)
$r=[ordered]@{utc=[DateTime]::UtcNow.ToString('o');boot=$boot.ToUniversalTime().ToString('o');driver=$d[0].DriverVersion;code=[int]$v[0].ConfigManagerErrorCode;tdr4101=$ev.Count}
'BW580_RESULT '+($r|ConvertTo-Json -Compress)
