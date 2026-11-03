$ErrorActionPreference='Stop'
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
$v=@(Get-CimInstance Win32_VideoController | Where-Object {$_.PNPDeviceID -like 'PCI\VEN_10DE*'})
if($v.Count -ne 1){throw "expected one NVIDIA device, actual=$($v.Count)"}
$d=@(Get-CimInstance Win32_PnPSignedDriver | Where-Object {$_.DeviceID -eq $v[0].PNPDeviceID})
$ev=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101;StartTime=$boot} -ErrorAction SilentlyContinue)
$r=[ordered]@{utc=[DateTime]::UtcNow.ToString('o');boot=$boot.ToUniversalTime().ToString('o');driver=$v[0].DriverVersion;inf=$d[0].InfName;code=[int]$v[0].ConfigManagerErrorCode;status=$v[0].Status;tdr4101=$ev.Count;width=$v[0].CurrentHorizontalResolution;height=$v[0].CurrentVerticalResolution}
'BW580_RESULT '+($r|ConvertTo-Json -Compress)
