# BW7 native reference: driver 580.88, Code 0, uninterrupted 600-second hold.
$ErrorActionPreference='Stop'
$start=Get-Date
$boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
$rows=@()
function Sample($n){
 $v=@(Get-CimInstance Win32_VideoController |
  Where-Object {$_.PNPDeviceID -like 'PCI\VEN_10DE*'})
 if($v.Count -ne 1){ throw 'Expected exactly one NVIDIA controller' }
 $events=@(Get-WinEvent -FilterHashtable @{LogName='System';StartTime=$start} `
  -ErrorAction SilentlyContinue -ErrorVariable queryErrors)
 foreach($e in $queryErrors){
  if($e.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*'){ throw $e }
 }
 $fatal=@($events | Where-Object {$_.Id -in 1001,6008,4101})
 $display=@($events | Where-Object {
  $_.ProviderName -in 'nvlddmkm','Display' -and $_.Level -le 2 })
 $nowBoot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime
 $r=[ordered]@{n=$n;utc=[DateTime]::UtcNow.ToString('o');
  boot=$nowBoot.ToUniversalTime().ToString('o');code=[int]$v[0].ConfigManagerErrorCode;
  name=$v[0].Name;driver=$v[0].DriverVersion;event1001=@($fatal|Where-Object Id -eq 1001).Count;
  event6008=@($fatal|Where-Object Id -eq 6008).Count;
  event4101=@($fatal|Where-Object Id -eq 4101).Count;displayErrors=$display.Count;
  elapsedSeconds=((Get-Date)-$start).TotalSeconds}
 Write-Host ('REF_HOLD_SAMPLE '+($r|ConvertTo-Json -Compress))
 if($r.code -ne 0 -or $r.driver -ne '32.0.15.8088' -or
  $r.name -notlike '*RTX 5090*' -or $fatal.Count -gt 0 -or
  $display.Count -gt 0 -or $nowBoot -ne $boot){ throw 'H-REF falsifier or identity failure' }
 return $r
}
'REF_HOLD_START '+$start.ToUniversalTime().ToString('o')
$rows+=Sample 0
foreach($n in 1..10){
 Start-Sleep -Seconds 60
 $rows+=Sample $n
 $rows|ConvertTo-Json -Depth 4|Set-Content C:\kf\bw7_hold.json
}
'REF_HOLD_DONE'
