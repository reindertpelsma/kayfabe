# BW4 REFERENCE: 10-minute hold, one sample per minute (device code, new 4101/nvlddmkm events).
$ErrorActionPreference='Continue'
$start=Get-Date
foreach($n in 1..10){
 Start-Sleep -Seconds 60
 $v=@(Get-CimInstance Win32_VideoController | Where-Object {$_.PNPDeviceID -like 'PCI\VEN_10DE*'})
 $ev=@(Get-WinEvent -FilterHashtable @{LogName='System';Id=4101;StartTime=$start} `
  -ErrorAction SilentlyContinue)
 $h=@{LogName='System';ProviderName='nvlddmkm';StartTime=$start}
 $nv=@(Get-WinEvent -FilterHashtable $h `
  -ErrorAction SilentlyContinue)
 $code=[int]$v[0].ConfigManagerErrorCode
 "REF_HOLD n=$n utc=$([DateTime]::UtcNow.ToString('o')) code=$code " +
  "tdr4101=$($ev.Count) nvlddmkm=$($nv.Count)"
}
'REF_HOLD_DONE'
